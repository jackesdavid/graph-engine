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
use crate::spec::{Behavior, NodeCx, NodeError, NodeRun, NodeSpec, Purity, Step, StepCx, Timeout};
use crate::topo::{back_edges, ordering_pairs};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Why a run stopped early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// A node named in the document is not registered. Named, because "unknown node kind" with
    /// no name sends people reading the whole graph.
    UnknownKind { node: u32, kind: String },
    /// A node refused. Carries the node's own judgement about whether trying again could help,
    /// so a durable host does not have to match on the message to decide.
    Node {
        node: u32,
        kind: String,
        message: String,
        retry: crate::host::Retry,
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
                ..
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

/// When a run's progress is written down.
///
/// The two schedulers the plan called for — one for continuous dataflow, one for durable task
/// runs — turned out to be one scheduler and this enum. What actually differs between them is
/// not how nodes are ordered but how often the run is committed, and that is a policy the host
/// chooses rather than a second implementation to keep in step with the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Checkpoint {
    /// Nothing is written while the run is in flight.
    ///
    /// Right for runs measured in milliseconds, where a lost run is simply the next frame's
    /// problem, and where a write per node would cost more than the work.
    #[default]
    None,

    /// Each node's outputs are written as it produces them, and a resumption reads them back.
    ///
    /// This is what makes a run survive the process it started in. A node that already ran is
    /// restored rather than re-executed, which is the difference between resuming a workflow
    /// and running it again — and for a workflow that sends mail, running it again is not a
    /// recoverable mistake.
    ///
    /// The checkpoints are cleared when the run finishes, so what is on disk is always either a
    /// run in flight or nothing.
    EveryNode,
}

/// How to run.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    pub budget: Budget,
    pub checkpoint: Checkpoint,
}

impl RunOptions {
    /// Every node committed as it completes — a run that survives a restart.
    pub fn durable() -> Self {
        RunOptions {
            checkpoint: Checkpoint::EveryNode,
            ..Self::default()
        }
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
    if !seed_entries && st.checkpoint == Checkpoint::EveryNode {
        restore(graph, reg, host, &instance, &forced, &mut st);
    }

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

fn values_key(graph: uuid::Uuid, node: u32, instance: &str) -> crate::host::StateKey {
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
fn restore<M: GraphMeta, H: Host<Meta = M>>(
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

struct State {
    checkpoint: Checkpoint,
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

    // A memoized node runs once per run and its result is reused, even when control reaches it
    // again through a loop. This is how a graph reads a table once and iterates it, rather than
    // re-reading it on every pass.
    if graph.node(nid).is_some_and(|n| n.memoize) && st.ran.contains(&nid) {
        return false;
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

    let fail = |e: crate::spec::NodeError| RunError::Node {
        node: nid,
        kind: node.kind.as_str().to_string(),
        message: e.message,
        retry: e.retry,
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
            let (out, summary, ms) =
                run_timed(runner, spec.timeout, &node.config, &inputs, nid, host).map_err(fail)?;
            host.observer().node_finished(nid, &summary, ms);
            record(host, graph.id, nid, instance, &out, st);
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
            let as_run: Arc<dyn NodeRun<H>> = router.clone();
            let (out, summary, ms) =
                run_timed(&as_run, spec.timeout, &node.config, &inputs, nid, host).map_err(fail)?;
            let cx = NodeCx {
                config: &node.config,
                inputs: &inputs,
                node: nid,
                host,
            };
            let arms = router.arms(&cx, &out);
            host.observer().node_finished(nid, &summary, ms);
            for a in arms {
                st.live_arms.insert((nid, a));
            }
            record(host, graph.id, nid, instance, &out, st);
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
            // Measured, but never abandoned. A cooperating node owns durable state and a
            // scratch the scheduler is holding a mutable borrow of; walking away from one
            // mid-flight leaves an armed window nobody will ever close. Timing out here would
            // trade a stuck run for corrupt state, which is the worse of the two.
            let started = Instant::now();
            let step: Step = stepper.step(&mut cx).map_err(fail)?;
            st.scratch.insert(nid, scratch);

            if let Some(msg) = &step.log {
                host.observer()
                    .node_finished(nid, msg, started.elapsed().as_millis());
            }
            for a in &step.arms {
                st.live_arms.insert((nid, a.clone()));
            }
            record(host, graph.id, nid, instance, &step.outputs, st);
            st.outputs.insert(nid, step.outputs);
            st.ran.insert(nid);
            if step.halt {
                st.halted = true;
            }
            Ok(step.reenter)
        }
    }
}

/// Run a node, giving up on it if it takes longer than it declared.
///
/// A node with [`Timeout::Inline`] runs in place: it declared that it cannot block, and paying
/// for a thread to supervise arithmetic is worse than the risk.
///
/// Anything else runs on its own thread and is **abandoned** if it overruns. Abandoned, not
/// killed — a thread cannot be killed, so the overrunning work carries on and its result is
/// dropped when it finally arrives. That is the honest trade: the alternative is one wedged
/// socket holding a run open until somebody restarts the process.
fn run_timed<H: Host>(
    runner: &Arc<dyn NodeRun<H>>,
    timeout: Timeout,
    config: &Json,
    inputs: &PortValues,
    nid: u32,
    host: &H,
) -> Result<(PortValues, String, u128), NodeError> {
    let started = Instant::now();

    if let Timeout::Secs(secs) = timeout {
        let (tx, rx) = std::sync::mpsc::channel();
        let runner = Arc::clone(runner);
        // Cloning the inputs is cheap where it matters: `Bytes` holds an `Arc<[u8]>`, so a frame
        // on a wire is a refcount bump rather than a copy.
        let (config, inputs, host) = (config.clone(), inputs.clone(), host.clone());
        std::thread::spawn(move || {
            let cx = NodeCx {
                config: &config,
                inputs: &inputs,
                node: nid,
                host: &host,
            };
            let out = runner.run(&cx);
            let summary = match &out {
                Ok(o) => runner.summary(&cx, o),
                Err(_) => String::new(),
            };
            // The receiver is gone if we already gave up. Nothing to report to.
            let _ = tx.send(out.map(|o| (o, summary)));
        });

        return match rx.recv_timeout(Duration::from_secs(secs)) {
            Ok(Ok((out, summary))) => Ok((out, summary, started.elapsed().as_millis())),
            Ok(Err(e)) => Err(e),
            // Transient by construction: a node that ran out of time is the definition of
            // something that might work on a less busy afternoon.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(NodeError::transient(format!("gave up after {secs}s")))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(NodeError::transient("the node stopped without answering"))
            }
        };
    }

    let cx = NodeCx {
        config,
        inputs,
        node: nid,
        host,
    };
    let out = runner.run(&cx)?;
    let summary = runner.summary(&cx, &out);
    Ok((out, summary, started.elapsed().as_millis()))
}

/// Commit a node's outputs, if this run is committing them.
fn record<H: Host>(
    host: &H,
    graph: uuid::Uuid,
    nid: u32,
    instance: &str,
    out: &PortValues,
    st: &State,
) {
    if st.checkpoint != Checkpoint::EveryNode {
        return;
    }
    let j = crate::codec::encode_ports(out, host.io());
    host.state().set(&values_key(graph, nid, instance), &j);
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
        message: e.message,
        retry: e.retry,
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
