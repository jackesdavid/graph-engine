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
//! run, unless the node is a [`PureSource`](crate::Purity::PureSource), which re-reads the world
//! every time it is asked.
//!
//! ## The step budget
//!
//! A loop whose exit condition never becomes true is not a hang the engine can reason about, so
//! there is a ceiling on node executions per run. Reaching it is an error with the graph named,
//! not a silent stop: a run that quietly did half its work is worse than one that failed.

use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::registry::NodeRegistry;
use crate::spec::{Behavior, NodeCx, NodeSpec, Purity, Step, StepCx};
use crate::topo::{back_edges, ordering_pairs};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};

/// Why a run stopped early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// A node named in the document is not registered. Named, because "unknown node kind" with
    /// no name sends people reading the whole graph.
    UnknownKind { node: u32, kind: String },
    /// A node refused.
    Node {
        node: u32,
        kind: String,
        message: String,
    },
    /// The step ceiling was reached — almost always a loop with no exit.
    Budget { limit: u32 },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::UnknownKind { node, kind } => {
                write!(f, "node {node}: no registered kind {kind:?}")
            }
            RunError::Node {
                node,
                kind,
                message,
            } => {
                write!(f, "node {node} ({kind}): {message}")
            }
            RunError::Budget { limit } => {
                write!(
                    f,
                    "stopped after {limit} node executions — a loop with no exit?"
                )
            }
        }
    }
}

impl std::error::Error for RunError {}

/// How much a run may do before the engine assumes it will never finish.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub max_steps: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { max_steps: 10_000 }
    }
}

/// Where a run begins.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// Start at these nodes specifically. Empty means every node with no incoming exec edge.
    ///
    /// A non-empty set is a **resumption**: an answer arriving at the node that asked for it, a
    /// timer firing at the node that set it.
    pub at: Vec<u32>,
    /// What the entry carries — the answer, the event payload.
    pub payload: PortValues,
}

/// What a finished run produced, per node.
pub type Outputs = HashMap<u32, PortValues>;

/// Run a graph to completion.
pub fn run<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    entry: &Entry,
    budget: Budget,
) -> Result<Outputs, RunError> {
    let instance = host.instance_key(&graph.meta, &entry.payload);

    let back = back_edges(graph);
    let pairs = ordering_pairs(graph, &back);
    let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut indeg_template: HashMap<u32, usize> = graph.nodes.iter().map(|n| (n.id, 0)).collect();
    for (from, to) in &pairs {
        adj.entry(*from).or_default().push(*to);
        *indeg_template.entry(*to).or_insert(0) += 1;
    }

    let mut st = State {
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

    loop {
        let reentries = epoch(
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

    Ok(st.outputs)
}

struct State {
    outputs: Outputs,
    ran: HashSet<u32>,
    /// `(node, arm)` pairs that fired **in the current epoch**, and only it.
    ///
    /// Cleared at the start of every epoch, and that is load-bearing. Let it accumulate and a
    /// loop's body stays reachable after the loop has finished: the arm that fired on pass one
    /// is still marked live on the pass that fires `completed`, so the body runs one extra time.
    /// Three items, four executions, no error — the kind of thing that is only ever noticed as
    /// a duplicate e-mail.
    live_arms: HashSet<(u32, PortName)>,
    scratch: HashMap<u32, Json>,
    steps: u32,
    halted: bool,
}

#[allow(clippy::too_many_arguments)]
fn epoch<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    instance: &str,
    entry: &Entry,
    seed_entries: bool,
    forced: &HashSet<u32>,
    indeg_template: &HashMap<u32, usize>,
    adj: &HashMap<u32, Vec<u32>>,
    back: &HashSet<(u32, u32)>,
    budget: Budget,
    st: &mut State,
) -> Result<Vec<u32>, RunError> {
    st.live_arms.clear();

    let mut indeg = indeg_template.clone();
    let mut ready: Vec<u32> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    ready.sort_unstable();

    let mut reentries = Vec::new();

    while let Some(nid) = ready.first().copied() {
        ready.remove(0);
        let node = graph.node(nid).expect("ready node exists");
        let spec = reg.get(&node.kind).ok_or_else(|| RunError::UnknownKind {
            node: nid,
            kind: node.kind.as_str().to_string(),
        })?;

        if should_run(graph, spec, nid, forced, seed_entries, st) {
            st.steps += 1;
            if st.steps > budget.max_steps {
                return Err(RunError::Budget {
                    limit: budget.max_steps,
                });
            }
            let reenter = execute(graph, reg, host, instance, entry, spec, nid, forced, st)?;
            if reenter {
                reentries.push(nid);
            }
            if st.halted {
                return Ok(Vec::new());
            }
        }

        for m in adj.get(&nid).cloned().unwrap_or_default() {
            if back.contains(&(nid, m)) {
                continue;
            }
            if let Some(d) = indeg.get_mut(&m) {
                *d -= 1;
                if *d == 0 {
                    ready.push(m);
                    ready.sort_unstable();
                }
            }
        }
    }

    Ok(reentries)
}

/// Has control reached this node?
fn should_run<M: GraphMeta, H: Host>(
    graph: &Graph<M>,
    spec: &NodeSpec<H>,
    nid: u32,
    forced: &HashSet<u32>,
    seed_entries: bool,
    st: &State,
) -> bool {
    // Pure nodes are pulled, never pushed.
    if !spec.purity.has_exec() {
        return false;
    }
    if forced.contains(&nid) {
        return true;
    }
    // Note there is no "already ran" guard here. A node reached again through a different arm
    // may legitimately run again — that is what a join inside a loop is — and the thing that
    // stops runaway repetition is the step budget, which reports itself, rather than a silent
    // skip that makes half a loop look like a finished one.

    let incoming: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.to == nid && e.to_port == crate::port::EXEC_IN.name)
        .collect();

    if incoming.is_empty() {
        // An entry point — but only when the run is starting from entries at all. On a
        // resumption, entries elsewhere in the graph must stay asleep.
        return seed_entries;
    }

    // Reached only if at least one incoming arm actually fired. This is what makes the untaken
    // side of a branch, and everything under it, stay dead.
    incoming
        .iter()
        .any(|e| st.live_arms.contains(&(e.from, e.from_port.clone())))
}

/// Run one node and record what it produced.
#[allow(clippy::too_many_arguments)]
fn execute<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    instance: &str,
    entry: &Entry,
    spec: &NodeSpec<H>,
    nid: u32,
    forced: &HashSet<u32>,
    st: &mut State,
) -> Result<bool, RunError> {
    let node = graph.node(nid).expect("node exists");

    let fail = |m: String| RunError::Node {
        node: nid,
        kind: node.kind.as_str().to_string(),
        message: m,
    };

    match &spec.behavior {
        Behavior::Inert => {
            host.observer().node_started(nid);
            st.ran.insert(nid);
            fire_default(spec, nid, &node.config, st);
            Ok(false)
        }

        Behavior::Run(runner) => {
            // Inputs first: pulling a pure source announces that source, and announcing this
            // node before its inputs would report the consumer as having started before the
            // thing it consumes.
            let inputs = gather(graph, reg, host, nid, st)?;
            host.observer().node_started(nid);
            let cx = NodeCx {
                config: &node.config,
                inputs: &inputs,
                node: nid,
                host,
            };
            let out = runner.run(&cx).map_err(|e| fail(e.0))?;
            let summary = runner.summary(&cx, &out);
            host.observer().node_finished(nid, &summary, 0);
            st.outputs.insert(nid, out);
            st.ran.insert(nid);
            fire_default(spec, nid, &node.config, st);
            Ok(false)
        }

        Behavior::Route(router) => {
            // Inputs first: pulling a pure source announces that source, and announcing this
            // node before its inputs would report the consumer as having started before the
            // thing it consumes.
            let inputs = gather(graph, reg, host, nid, st)?;
            host.observer().node_started(nid);
            let cx = NodeCx {
                config: &node.config,
                inputs: &inputs,
                node: nid,
                host,
            };
            let out = router.run(&cx).map_err(|e| fail(e.0))?;
            let arms = router.arms(&cx, &out);
            let summary = router.summary(&cx, &out);
            host.observer().node_finished(nid, &summary, 0);
            for a in arms {
                st.live_arms.insert((nid, a));
            }
            st.outputs.insert(nid, out);
            st.ran.insert(nid);
            Ok(false)
        }

        Behavior::Step(stepper) => {
            // Inputs first: pulling a pure source announces that source, and announcing this
            // node before its inputs would report the consumer as having started before the
            // thing it consumes.
            let inputs = gather(graph, reg, host, nid, st)?;
            host.observer().node_started(nid);
            let mut scratch = st.scratch.remove(&nid).unwrap_or_else(|| json!({}));
            let mut cx = StepCx {
                config: &node.config,
                inputs: &inputs,
                node: nid,
                graph: graph.id,
                instance,
                forced: forced.contains(&nid),
                entry_payload: &entry.payload,
                host,
                scratch: &mut scratch,
            };
            let step: Step = stepper.step(&mut cx).map_err(|e| fail(e.0))?;
            st.scratch.insert(nid, scratch);

            if let Some(msg) = &step.log {
                host.observer().node_finished(nid, msg, 0);
            }
            for a in &step.arms {
                st.live_arms.insert((nid, a.clone()));
            }
            st.outputs.insert(nid, step.outputs);
            st.ran.insert(nid);
            if step.halt {
                st.halted = true;
            }
            Ok(step.reenter)
        }
    }
}

/// Fire the node's exec arms for a node that does not choose them itself.
fn fire_default<H: Host>(spec: &NodeSpec<H>, nid: u32, config: &Json, st: &mut State) {
    for p in spec.exec_out.resolve(config) {
        st.live_arms.insert((nid, p.name));
    }
}

/// Collect a node's inputs, evaluating pure sources behind them on demand.
fn gather<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    nid: u32,
    st: &mut State,
) -> Result<PortValues, RunError> {
    let mut inputs = PortValues::new();
    let wires: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.to == nid && e.to_port != crate::port::EXEC_IN.name)
        .cloned()
        .collect();

    for w in wires {
        // A wire from a node that never ran carries nothing. That is not an error: it is how a
        // dead branch's absence reaches the far side of a join.
        if !st.ran.contains(&w.from) {
            pull(graph, reg, host, w.from, st)?;
        }
        if let Some(v) = st
            .outputs
            .get(&w.from)
            .and_then(|o| o.get(&w.from_port))
            .cloned()
        {
            inputs.insert(w.to_port.clone(), v);
        }
    }
    Ok(inputs)
}

/// Evaluate a pure node because something is reading it.
fn pull<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    nid: u32,
    st: &mut State,
) -> Result<(), RunError> {
    let node = graph.node(nid).expect("wire source exists");
    let spec = reg.get(&node.kind).ok_or_else(|| RunError::UnknownKind {
        node: nid,
        kind: node.kind.as_str().to_string(),
    })?;

    // An effectful node that has not been reached stays unreached. Pulling it would run the
    // untaken side of a branch through the back door.
    if spec.purity.has_exec() {
        return Ok(());
    }
    if st.ran.contains(&nid) && spec.purity != Purity::PureSource {
        return Ok(());
    }

    let inputs = gather(graph, reg, host, nid, st)?;
    let Behavior::Run(runner) = &spec.behavior else {
        st.ran.insert(nid);
        return Ok(());
    };
    let cx = NodeCx {
        config: &node.config,
        inputs: &inputs,
        node: nid,
        host,
    };
    st.steps += 1;
    host.observer().node_started(nid);
    let out = runner.run(&cx).map_err(|e| RunError::Node {
        node: nid,
        kind: node.kind.as_str().to_string(),
        message: e.0,
    })?;
    let summary = runner.summary(&cx, &out);
    host.observer().node_finished(nid, &summary, 0);
    st.outputs.insert(nid, out);
    st.ran.insert(nid);
    Ok(())
}

/// Read a node's output port after a run.
pub fn output<'a>(outputs: &'a Outputs, node: u32, port: &str) -> Option<&'a Value> {
    outputs.get(&node)?.get(&PortName::new(port))
}
