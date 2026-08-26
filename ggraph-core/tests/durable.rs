// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Runs that survive the process they started in.
//!
//! A graph that waits — for a timer, for a person — cannot hold a thread open while it does, so
//! it ends the run and is re-entered later. That works with no persistence at all as long as
//! nothing downstream needs what the earlier nodes produced. The moment something does, the
//! resumption has to read it back, and *not* re-execute the nodes that produced it: for a
//! workflow that has already sent mail, running it again is not a recoverable mistake.
//!
//! That is what `Checkpoint::EveryNode` buys, and these tests are the difference it makes.

use ggraph_core::exec::{Entry, RunOptions};
use ggraph_core::host::testkit::TestHost;
use ggraph_core::{Graph, Host, NodeId, NodeRegistry, PortName, Value};
use serde_json::json;

struct Built {
    graph: Graph,
    reg: NodeRegistry<TestHost>,
    host: TestHost,
}

/// A loop that produces a value, a wait that suspends the run, and a node downstream that reads
/// the value back. The shape of every workflow that pauses in the middle.
fn suspending_graph() -> (Built, u32, u32, u32) {
    let mut reg = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut reg, &ggraph_core::Services::none());
    let mut graph = Graph::new("suspends");

    let source = graph.add_node(NodeId::new("for_each"), 0, 0);
    graph.node_mut(source).unwrap().config = json!({ "items": "alpha" });
    let pause = graph.add_node(NodeId::new("wait"), 200, 0);
    graph.node_mut(pause).unwrap().config = json!({ "seconds": "300" });
    let after = graph.add_node(NodeId::new("print"), 400, 0);
    graph.node_mut(after).unwrap().config = json!({ "message": "" });

    graph
        .add_edge(&reg, source, "loop_body", pause, "exec_in")
        .unwrap();
    graph
        .add_edge(&reg, pause, "exec_out", after, "exec_in")
        .unwrap();
    graph
        .add_edge(&reg, source, "item", after, "message")
        .unwrap();

    let host = TestHost::new();
    (Built { graph, reg, host }, source, pause, after)
}

impl Built {
    fn run(&self, entry: Entry, opts: &RunOptions) {
        ggraph_core::run(&self.graph, &self.reg, &self.host, &entry, opts).expect("runs");
    }
    fn started(&self) -> Vec<u32> {
        self.host.inner().observer.started.lock().unwrap().clone()
    }
    fn said_by(&self, node: u32) -> Vec<String> {
        self.host
            .inner()
            .observer
            .finished
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| *n == node)
            .map(|(_, s)| s.clone())
            .collect()
    }
    fn clear_log(&self) {
        self.host.inner().observer.started.lock().unwrap().clear();
        self.host.inner().observer.finished.lock().unwrap().clear();
    }
}

#[test]
fn a_resumption_reads_back_what_the_first_run_produced() {
    let (b, source, pause, after) = suspending_graph();

    b.run(Entry::default(), &RunOptions::durable());
    assert!(
        b.started().contains(&source),
        "the first run gets as far as the wait"
    );
    assert!(!b.started().contains(&after), "and no further");

    // Later, elsewhere, possibly in another process: control comes back to the wait.
    b.clear_log();
    b.run(
        Entry {
            at: vec![pause],
            ..Entry::default()
        },
        &RunOptions::durable(),
    );

    assert!(
        b.started().contains(&after),
        "the run continues past the wait"
    );
    assert_eq!(
        b.said_by(after),
        vec!["alpha"],
        "and the node downstream sees what the first run produced, not a blank"
    );
    assert!(
        !b.started().contains(&source),
        "the node that already ran must NOT run again — for a workflow that has already sent \
         mail, running it again is not a recoverable mistake"
    );
}

#[test]
fn without_checkpointing_the_resumption_has_nothing_to_read() {
    let (b, _source, pause, after) = suspending_graph();

    b.run(Entry::default(), &RunOptions::default());
    b.clear_log();
    b.run(
        Entry {
            at: vec![pause],
            ..Entry::default()
        },
        &RunOptions::default(),
    );

    assert!(b.started().contains(&after));
    assert_eq!(
        b.said_by(after),
        vec![""],
        "this is the honest cost of Checkpoint::None, and the reason a workflow host wants \
         EveryNode: the value is simply gone"
    );
}

#[test]
fn a_run_that_finishes_leaves_nothing_behind() {
    let mut reg = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut reg, &ggraph_core::Services::none());
    let mut graph = Graph::new("completes");
    let a = graph.add_node(NodeId::new("for_each"), 0, 0);
    graph.node_mut(a).unwrap().config = json!({ "items": "x" });
    let b = graph.add_node(NodeId::new("print"), 200, 0);
    graph.node_mut(b).unwrap().config = json!({ "message": "done" });
    graph.add_edge(&reg, a, "completed", b, "exec_in").unwrap();

    let host = TestHost::new();
    ggraph_core::run(
        &graph,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions::durable(),
    )
    .expect("runs");

    let key = ggraph_core::StateKey {
        target: ggraph_core::NodeTarget {
            graph: graph.id,
            node: a,
            instance: Default::default(),
        },
        slot: ggraph_core::Slot::Values,
    };
    assert!(
        host.state().get(&key).is_none(),
        "what is on disk should be either a run in flight or nothing — leftovers from finished \
         runs are what a later resumption would restore and act on"
    );
}

#[test]
fn a_fresh_run_is_not_seeded_from_a_previous_one() {
    let (b, source, pause, after) = suspending_graph();

    // Suspend once, leaving checkpoints behind.
    b.run(Entry::default(), &RunOptions::durable());

    // A new run, from the top rather than from the wait.
    b.clear_log();
    b.run(Entry::default(), &RunOptions::durable());

    assert!(
        b.started().contains(&source),
        "a fresh run must execute the source again — seeding it from the suspended run's \
         leftovers would make stale values look live and revive branches the engine left dead"
    );
    assert!(!b.started().contains(&after));
    let _ = pause;
}

#[test]
fn the_node_being_re_entered_runs_again_rather_than_being_restored() {
    // A resumption delivers something *to* the waiting node — an answer, a timer. Restoring its
    // own previous outputs would hand it the state it had before it asked.
    let (b, _source, pause, _after) = suspending_graph();
    b.run(Entry::default(), &RunOptions::durable());
    b.clear_log();
    b.run(
        Entry {
            at: vec![pause],
            ..Entry::default()
        },
        &RunOptions::durable(),
    );
    assert!(b.started().contains(&pause));
}

#[test]
fn a_value_that_cannot_be_persisted_does_not_take_the_rest_of_the_row_with_it() {
    // A node's other outputs are worth keeping even when one of them is a decoded image that
    // has no business in a state table. Refusing the whole row would mean a graph with an image
    // anywhere in it could not be checkpointed at all.
    let io = ggraph_core::host::Disabled;
    let mut vals = ggraph_core::PortValues::new();
    vals.insert(PortName::new("width"), Value::int(1920));
    vals.insert(
        PortName::new("frame"),
        Value::Bytes(ggraph_core::Bytes::new("image/jpeg", vec![0u8; 64])),
    );
    let j = ggraph_core::encode_ports(&vals, &io);
    let back = ggraph_core::decode_ports(&j, &io, &Default::default());
    assert_eq!(back.len(), 1);
    assert_eq!(
        back.get(&PortName::new("width")).and_then(Value::as_i64),
        Some(1920)
    );
}
