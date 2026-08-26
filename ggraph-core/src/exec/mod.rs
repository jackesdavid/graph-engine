//! The scheduler: deciding what runs, in what order, and what does not run at all.
//!
//! ## Why epochs rather than one pass
//!
//! A topological sweep runs every node once, which is wrong in two directions at once. A branch
//! means some nodes must **not** run — the arm that was not taken, and everything downstream of
//! it. A loop means one node must run **many** times. Neither fits a single ordered pass.
//!
//! So a run is a sequence of *epochs*. Each epoch is a settling pass in dependency order over
//! the nodes that control flow has reached; a loop's back edge marks its target for the next
//! epoch. The run ends when an epoch reaches nobody new.
//!
//! ## Dead branches
//!
//! A node is reached when control arrives on `exec_in` from an arm that actually fired. If every
//! incoming arm belongs to a branch that went the other way, the node is *dead*: it does not run,
//! it produces nothing, and its own downstream is dead in turn. This is the difference between a
//! branch and an if-statement that runs both sides and throws one away.
//!
//! ## Pull, not push
//!
//! Pure nodes have no exec pins and are never reached. They are **pulled** when something reads
//! them: a required input with a wire behind it evaluates that wire's source first. Once per
//! run, unless the node is a [`PureSource`](crate::Purity::PURE_SOURCE), which re-reads the world
//! every time it is asked.
//!
//! ## The step budget
//!
//! A loop whose exit condition never becomes true is not a hang the engine can reason about, so
//! there is a ceiling on node executions per run. Reaching it is an error with the graph named,
//!
//! Split by the question each part answers:
//!
//! - [`options`] — what a run is asked for, and how it can fail;
//! - [`epoch`] — one pass over the graph, and who is allowed to run in it;
//! - [`node`] — running a single node;
//! - [`inputs`] — where a node's inputs come from.
//!
//! What stays here is the loop itself: start, run epochs until nothing asks to go again, restore
//! on a resumption, and tell the observer it is over.

use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::registry::NodeRegistry;
use crate::topo::{back_edges, ordering_pairs};
use std::collections::{HashMap, HashSet};

pub mod epoch;
pub mod inputs;
pub mod node;
pub mod options;

pub(crate) use epoch::*;
pub use inputs::*;
pub(crate) use node::*;
pub use options::*;

/// Run a graph to completion.
///
/// The observer is told the run finished on **either** path. An observer that buffers — one
/// deciding whether to report a node based on what happened later — would otherwise lose
/// everything it was holding whenever a run failed, which is the run you most want the report
/// from.
pub fn run<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    entry: &Entry,
    opts: &RunOptions,
) -> Result<Outputs, RunError> {
    let out = run_inner(graph, reg, host, entry, opts);
    host.observer().run_finished();
    out
}

fn run_inner<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    entry: &Entry,
    opts: &RunOptions,
) -> Result<Outputs, RunError> {
    let instance = host.instance_key(&graph.meta, &entry.payload);
    let budget = opts.budget;

    let back = back_edges(graph);
    let pairs = ordering_pairs(graph, &back);
    let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut indeg_template: HashMap<u32, usize> = graph.nodes.iter().map(|n| (n.id, 0)).collect();
    for (from, to) in &pairs {
        adj.entry(*from).or_default().push(*to);
        *indeg_template.entry(*to).or_insert(0) += 1;
    }

    let mut st = State {
        checkpoint: opts.checkpoint,
        vars: Default::default(),
        outputs: Outputs::new(),
        ran: HashSet::new(),
        live_arms: HashSet::new(),
        scratch: HashMap::new(),
        steps: 0,
        halted: false,
    };

    // The first epoch's entry set: an explicit resumption, or everything control can start at.
    let mut forced: HashSet<u32> = entry.at.iter().copied().collect();
    let seed_entries = forced.is_empty();

    // A resumption reads back what the interrupted run had already produced. Only on a
    // resumption: seeding a fresh run from a previous one's leftovers would make stale values
    // look live and revive branches the engine deliberately left dead.
    if !seed_entries && (entry.restore || st.checkpoint == Checkpoint::EveryNode) {
        restore(graph, reg, host, &instance, &forced, &mut st);
    }

    loop {
        let reentries = run_epoch(
            graph,
            reg,
            host,
            &instance,
            entry,
            seed_entries,
            &forced,
            &indeg_template,
            &adj,
            &back,
            budget,
            &mut st,
        )?;
        if st.halted || reentries.is_empty() {
            break;
        }
        forced = reentries.into_iter().collect();
    }

    // A run that ended has nothing left to resume. A halted one does — it is waiting for a
    // person or a timer — so its checkpoints stay exactly where they are.
    if !st.halted && st.checkpoint == Checkpoint::EveryNode {
        for node in &graph.nodes {
            host.state()
                .clear(&values_key(graph.id, node.id, &instance));
        }
    }

    Ok(st.outputs)
}

/// Where one node's outputs are remembered, for one instance of one graph.
///
/// Public because a host that wants to seed, inspect or migrate what a run will restore has to
/// name the same key the engine does. Guessing it is how two halves of a product end up writing
/// to two different places and neither noticing.
pub fn values_key(graph: uuid::Uuid, node: u32, instance: &str) -> crate::host::StateKey {
    crate::host::StateKey {
        target: crate::host::NodeTarget {
            graph,
            node,
            instance: smol_str::SmolStr::new(instance),
        },
        slot: crate::host::Slot::Values,
    }
}

/// Seed the run with what the interrupted one already did.
pub(crate) fn restore<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    instance: &str,
    forced: &HashSet<u32>,
    st: &mut State,
) {
    for node in &graph.nodes {
        // The node being re-entered is the one that was waiting. It runs again, with the answer
        // — restoring its old outputs would hand it the state it had before it asked.
        if forced.contains(&node.id) {
            continue;
        }
        let Some(j) = host.state().get(&values_key(graph.id, node.id, instance)) else {
            continue;
        };
        st.outputs.insert(
            node.id,
            crate::codec::decode_ports(&j, host.io(), reg.decoders()),
        );
        // Marked as run, so it is not executed a second time. That is the whole point: a
        // resumption must not repeat what already happened.
        st.ran.insert(node.id);
    }
}
