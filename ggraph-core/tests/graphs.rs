// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Whole graphs, built from the standard nodes and run end to end.
//!
//! The unit tests in each node file prove a node computes. These prove the *engine*: that a
//! branch leaves the other arm unexecuted, that a loop re-enters, that a pure node is pulled
//! rather than pushed, and that a loop with no exit fails loudly instead of hanging.
//!
//! They are also the documentation that cannot go stale — every one of them is a graph somebody
//! could build in the editor.

use ggraph_core::exec::{output, Budget, Entry, RunOptions};
use ggraph_core::host::testkit::TestHost;
use ggraph_core::{Graph, NodeId, NodeRegistry, RunError};
use serde_json::json;

fn registry() -> NodeRegistry<TestHost> {
    let mut r = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut r, &ggraph_core::Services::none());
    r
}

struct Built {
    graph: Graph,
    reg: NodeRegistry<TestHost>,
    host: TestHost,
}

impl Built {
    fn new(name: &str) -> Self {
        Built {
            graph: Graph::new(name),
            reg: registry(),
            host: TestHost::new(),
        }
    }

    fn node(&mut self, kind: &str, config: serde_json::Value) -> u32 {
        let id = self.graph.add_node(NodeId::new(kind), 0, 0);
        self.graph.node_mut(id).expect("just added").config = config;
        id
    }

    fn wire(&mut self, from: u32, fp: &str, to: u32, tp: &str) {
        self.graph
            .add_edge(&self.reg, from, fp, to, tp)
            .unwrap_or_else(|e| panic!("wire {from}.{fp} -> {to}.{tp}: {e}"));
    }

    fn run(&self) -> Result<ggraph_core::Outputs, RunError> {
        ggraph_core::run(
            &self.graph,
            &self.reg,
            &self.host,
            &Entry::default(),
            &RunOptions::default(),
        )
    }

    /// The nodes that executed, in order.
    fn order(&self) -> Vec<u32> {
        self.run().expect("graph runs");
        self.host.inner().observer.started.lock().unwrap().clone()
    }
}

#[test]
fn a_branch_runs_one_arm_and_leaves_the_other_untouched() {
    let mut b = Built::new("branch");
    let cmp = b.node("compare", json!({ "operator": ">", "a": "5", "b": "3" }));
    let br = b.node("if", json!({ "unknown_arm": false }));
    let hot = b.node("print", json!({ "message": "hot" }));
    let cold = b.node("print", json!({ "message": "cold" }));
    b.wire(cmp, "result", br, "condition");
    b.wire(br, "true", hot, "exec_in");
    b.wire(br, "false", cold, "exec_in");

    let order = b.order();
    assert!(
        order.contains(&hot),
        "5 > 3 must take the true arm: {order:?}"
    );
    assert!(
        !order.contains(&cold),
        "the false arm ran — a branch that executes both sides and discards one is not a branch"
    );
}

#[test]
fn everything_downstream_of_a_dead_arm_stays_dead() {
    let mut b = Built::new("dead subtree");
    let cmp = b.node("compare", json!({ "operator": ">", "a": "1", "b": "9" }));
    let br = b.node("if", json!({}));
    let first = b.node("print", json!({ "message": "first" }));
    let second = b.node("print", json!({ "message": "second" }));
    b.wire(cmp, "result", br, "condition");
    b.wire(br, "true", first, "exec_in");
    b.wire(first, "exec_out", second, "exec_in");

    let order = b.order();
    assert!(
        !order.contains(&first) && !order.contains(&second),
        "deadness has to propagate, or the second node runs on a branch nobody took: {order:?}"
    );
}

#[test]
fn a_pure_node_is_pulled_by_whoever_reads_it_not_pushed() {
    let mut b = Built::new("pull");
    // `compare` and `format` are pure: no exec pins, nothing hands them control. They run only
    // because the branch needs a condition.
    let fmt = b.node("format", json!({ "template": "7" }));
    let cmp = b.node("compare", json!({ "operator": "==", "b": "7" }));
    let br = b.node("if", json!({}));
    let seen = b.node("print", json!({ "message": "equal" }));
    b.wire(fmt, "text", cmp, "a");
    b.wire(cmp, "result", br, "condition");
    b.wire(br, "true", seen, "exec_in");

    let order = b.order();
    assert!(
        order.contains(&fmt) && order.contains(&cmp),
        "a pure node with no exec wiring must still be evaluated when read: {order:?}"
    );
    assert!(
        order.iter().position(|n| *n == fmt) < order.iter().position(|n| *n == br),
        "and it must be evaluated before the node that reads it"
    );
    assert!(
        order.contains(&seen),
        "\"7\" == 7 — the text parses as a number"
    );
}

#[test]
fn a_loop_runs_its_body_once_per_item_and_completes_after() {
    let mut b = Built::new("loop");
    let each = b.node("for_each", json!({ "items": "a,b,c" }));
    let body = b.node("print", json!({ "message": "item" }));
    let done = b.node("print", json!({ "message": "done" }));
    b.wire(each, "loop_body", body, "exec_in");
    b.wire(each, "completed", done, "exec_in");

    let order = b.order();
    assert_eq!(
        order.iter().filter(|n| **n == body).count(),
        3,
        "three items, three passes: {order:?}"
    );
    assert_eq!(
        order.iter().filter(|n| **n == done).count(),
        1,
        "completed fires once: {order:?}"
    );
    assert_eq!(
        order.last(),
        Some(&done),
        "and it fires last — an off-by-one here is invisible to every other test: {order:?}"
    );
}

#[test]
fn the_step_ceiling_stops_a_run_and_says_so() {
    let mut b = Built::new("budget");
    let each = b.node("for_each", json!({ "items": "a,b,c,d,e,f,g,h" }));
    let body = b.node("print", json!({ "message": "x" }));
    b.wire(each, "loop_body", body, "exec_in");

    // A ceiling below what this loop legitimately needs. The point is not that the graph is
    // wrong — it is that when a run hits the ceiling it FAILS, with the number, rather than
    // stopping quietly with half its work done and reporting success.
    let err = ggraph_core::run(
        &b.graph,
        &b.reg,
        &b.host,
        &Entry::default(),
        &RunOptions {
            budget: Budget { max_steps: 5 },
            ..RunOptions::default()
        },
    )
    .expect_err("a run past its ceiling must fail");
    assert!(matches!(err, RunError::Budget { limit: 5 }), "got {err:?}");
}

#[test]
fn an_unregistered_kind_names_the_node_and_the_kind() {
    let mut b = Built::new("typo");
    let id = b.graph.add_node(NodeId::new("prnit"), 0, 0);
    let err = b.run().expect_err("an unknown kind cannot run");
    assert_eq!(
        err,
        RunError::UnknownKind {
            node: id,
            kind: "prnit".into()
        },
        "'unknown node kind' with no name sends people reading the whole graph"
    );
}

#[test]
fn a_failing_node_names_itself() {
    let mut b = Built::new("bad compare");
    // Ordering text is refused: there is no locale-free answer.
    let cmp = b.node(
        "compare",
        json!({ "operator": "<", "a": "apple", "b": "banana" }),
    );
    let br = b.node("if", json!({}));
    b.wire(cmp, "result", br, "condition");

    match b.run() {
        Err(RunError::Node { node, kind, .. }) => {
            assert_eq!((node, kind.as_str()), (cmp, "compare"));
        }
        other => panic!("expected a named node failure, got {other:?}"),
    }
}

#[test]
fn the_third_arm_catches_what_the_false_arm_would_have_swallowed() {
    let mut b = Built::new("unknown");
    // Nothing is wired to `a`, so the comparison has no answer — which is not `false`.
    let cmp = b.node("compare", json!({ "operator": ">", "b": "3" }));
    let br = b.node("if", json!({ "unknown_arm": true }));
    let yes = b.node("print", json!({ "message": "greater" }));
    let no = b.node("print", json!({ "message": "not greater" }));
    let dunno = b.node("print", json!({ "message": "could not tell" }));
    b.wire(cmp, "result", br, "condition");
    b.wire(br, "true", yes, "exec_in");
    b.wire(br, "false", no, "exec_in");
    b.wire(br, "unknown", dunno, "exec_in");

    let order = b.order();
    assert!(
        order.contains(&dunno),
        "an unreadable value must reach the third arm: {order:?}"
    );
    assert!(
        !order.contains(&no),
        "and must NOT reach the false arm — that is the whole point of having three: {order:?}"
    );
}

#[test]
fn each_pass_carries_its_own_value_along_the_wire() {
    let mut b = Built::new("values");
    let each = b.node("for_each", json!({ "items": "alpha,beta,gamma" }));
    let say = b.node("print", json!({ "message": "" }));
    b.wire(each, "loop_body", say, "exec_in");
    b.wire(each, "item", say, "message");

    b.run().expect("runs");
    let said: Vec<String> = b
        .host
        .inner()
        .observer
        .finished
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| *n == say)
        .map(|(_, s)| s.clone())
        .collect();
    assert_eq!(
        said,
        vec!["alpha", "beta", "gamma"],
        "the body must see this pass's item, not the first one and not the last"
    );
}

#[test]
fn a_finished_loop_holds_nothing_on_its_item_port() {
    let mut b = Built::new("after");
    let each = b.node("for_each", json!({ "items": "a,b" }));
    let done = b.node("print", json!({ "message": "done" }));
    b.wire(each, "completed", done, "exec_in");

    let out = b.run().expect("runs");
    assert_eq!(
        output(&out, each, "item"),
        None,
        "a node reading `item` after the loop finished should get nothing, not the last item — \
         a stale value there reads as a real one"
    );
}
