//! One pass over the graph, and who is allowed to run in it.
//!
//! A run is a sequence of epochs rather than a single walk, because a loop's body has to run again
//! with new values and a node cannot be visited once if the answer changes. Each epoch settles the
//! graph, and a node that asks to be re-entered starts the next one.
//!
//! [`should_run`] is where the untaken branch dies. A node is reached only if at least one exec
//! edge into it comes from an arm that ACTUALLY FIRED — not merely from a node that ran. That
//! distinction is the whole of branching: an `if` runs, but only one of its arms fires, and
//! everything hanging off the other one must stay dead all the way down.
use super::*;
use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::registry::NodeRegistry;
use crate::spec::NodeSpec;
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};

pub(crate) struct State {
    pub(crate) checkpoint: Checkpoint,
    /// Named values shared across this run. Working state, gone when the run ends.
    pub(crate) vars: crate::spec::Vars,
    pub(crate) outputs: Outputs,
    pub(crate) ran: HashSet<u32>,
    /// `(node, arm)` pairs that fired **in the current epoch**, and only it.
    ///
    /// Cleared at the start of every epoch, and that is load-bearing. Let it accumulate and a
    /// loop's body stays reachable after the loop has finished: the arm that fired on pass one
    /// is still marked live on the pass that fires `completed`, so the body runs one extra time.
    /// Three items, four executions, no error — the kind of thing that is only ever noticed as
    /// a duplicate e-mail.
    pub(crate) live_arms: HashSet<(u32, PortName)>,
    pub(crate) scratch: HashMap<u32, Json>,
    pub(crate) steps: u32,
    pub(crate) halted: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_epoch<M: GraphMeta, H: Host<Meta = M>>(
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
pub(crate) fn should_run<M: GraphMeta, H: Host>(
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
