//! Things the engine declares, verified to actually happen.
//!
//! A `NodeSpec` says a node has a timeout and an `Observer` is handed an elapsed time. Both
//! were, for a while, declarations the scheduler ignored — which is worse than not offering
//! them, because a consumer sets `Timeout::Secs(30)` and believes it.
//!
//! A third one, `memoize`, was deleted instead of implemented. See the loop test below.
//!
//! These tests exist so that stays fixed.

use ggraph_core::exec::{Entry, RunOptions};
use ggraph_core::host::testkit::TestHost;
use ggraph_core::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use ggraph_core::{Graph, NodeId, NodeRegistry, PortValues, Retry, RunError, Value};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A node that takes as long as its config says. What a slow HTTP call looks like to the
/// scheduler, without needing a slow HTTP call.
struct Slow;

impl NodeRun<TestHost> for Slow {
    fn run(&self, cx: &NodeCx<'_, TestHost>) -> Result<PortValues, NodeError> {
        let ms = cx
            .cfg_str("sleep_ms")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(PortValues::new())
    }
}

/// Counts how many times it ran, so memoization is observable.
#[derive(Clone, Default)]
struct Counter(Arc<AtomicUsize>);

impl NodeRun<TestHost> for Counter {
    fn run(&self, _cx: &NodeCx<'_, TestHost>) -> Result<PortValues, NodeError> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        let mut out = PortValues::new();
        out.insert(ggraph_core::PortName::new("n"), Value::int(n as i64));
        Ok(out)
    }
}

fn registry(counter: Counter) -> NodeRegistry<TestHost> {
    let mut r = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut r);
    r.register(
        NodeSpec::effectful("slow", "Slow", "Test")
            .with_config(|| json!({ "sleep_ms": "0" }))
            .with_timeout(Timeout::Secs(1))
            .running(Slow),
    );
    r.register(
        NodeSpec::effectful("counter", "Counter", "Test")
            .with_outputs(Ports::Static(&[]))
            .running(counter),
    );
    r
}

#[test]
fn a_node_that_overruns_its_timeout_is_abandoned_and_says_so() {
    let counter = Counter::default();
    let reg = registry(counter);
    let mut g: Graph = Graph::new("slow");
    let n = g.add_node(NodeId::new("slow"), 0, 0);
    g.node_mut(n).unwrap().config = json!({ "sleep_ms": "3000" });

    let host = TestHost::new();
    let err = ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default())
        .expect_err("a node past its deadline must not simply hang the run");

    match err {
        RunError::Node {
            node,
            retry,
            message,
            ..
        } => {
            assert_eq!(node, n);
            assert!(message.contains("gave up"), "got {message:?}");
            assert_eq!(
                retry,
                Retry::Maybe,
                "running out of time is the definition of something that might work later"
            );
        }
        other => panic!("expected a node failure, got {other:?}"),
    }
}

#[test]
fn an_inline_node_is_not_supervised_by_a_thread() {
    // `Timeout::Inline` is a node saying it cannot block. Paying for a thread to supervise
    // arithmetic costs more than the risk it removes, so the scheduler takes it at its word.
    let reg = registry(Counter::default());
    let spec = reg.resolve("compare").expect("registered");
    assert_eq!(spec.timeout, Timeout::Inline);
    let spec = reg.resolve("http_request").expect("registered");
    assert!(
        matches!(spec.timeout, Timeout::Secs(_)),
        "anything that touches the world declares a deadline"
    );
}

#[test]
fn the_observer_is_handed_a_real_elapsed_time() {
    let reg = registry(Counter::default());
    let mut g: Graph = Graph::new("timed");
    let n = g.add_node(NodeId::new("slow"), 0, 0);
    g.node_mut(n).unwrap().config = json!({ "sleep_ms": "40" });

    let host = TestHost::new();
    ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()).expect("runs");

    let finished = host.inner().observer.finished.lock().unwrap().clone();
    assert!(!finished.is_empty(), "the node reported nothing at all");
    // The recorder keeps (node, summary); the elapsed value reaching it is what this asserts,
    // and a zero would mean the parameter is decoration.
    let ms = host.inner().observer.last_elapsed_ms();
    assert!(
        ms >= 30,
        "a node that slept 40ms reported {ms}ms — the elapsed parameter is not being measured"
    );
}

/// A node reached twice runs twice. There is no per-node opt-out, and there should not be.
///
/// There was one — `memoize`, "run once per run and reuse the result". It was deleted rather
/// than kept, because the distinction it tried to express is already made correctly and
/// automatically by the two ways a node can be reached: **control** flow reaches an effectful
/// node, and reaching it again means run it again (that is what "inside a loop" means), while a
/// **pure** node is pulled by whoever reads it and pulled once (that is what fan-out means).
///
/// A flag on top of that could only disagree with it. In the codebase this engine came from it
/// mostly did: set on a looping node it silently stopped the loop after one pass, and the engine
/// logged a warning telling the operator to turn it off again.
#[test]
fn a_node_in_a_loop_runs_on_every_pass() {
    let counter = Counter::default();
    let reg = registry(counter.clone());
    let mut g: Graph = Graph::new("loop body");

    let each = g.add_node(NodeId::new("for_each"), 0, 0);
    g.node_mut(each).unwrap().config = json!({ "items": "a,b,c" });
    let every = g.add_node(NodeId::new("counter"), 200, 0);
    g.add_edge(&reg, each, "loop_body", every, "exec_in")
        .unwrap();

    let host = TestHost::new();
    ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()).expect("runs");

    assert_eq!(
        counter.0.load(Ordering::SeqCst),
        3,
        "three items, three executions — an opt-out here is how a loop body becomes a no-op"
    );
}

#[test]
fn a_node_refusing_its_inputs_is_not_worth_retrying() {
    let reg = registry(Counter::default());
    let mut g: Graph = Graph::new("bad");
    // Ordering text has no locale-free answer, so `compare` refuses. That refusal will be
    // identical every time.
    let cmp = g.add_node(NodeId::new("compare"), 0, 0);
    g.node_mut(cmp).unwrap().config = json!({ "operator": "<", "a": "apple", "b": "banana" });
    let br = g.add_node(NodeId::new("if"), 200, 0);
    g.add_edge(&reg, cmp, "result", br, "condition").unwrap();

    let host = TestHost::new();
    match ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()) {
        Err(RunError::Node { retry, .. }) => assert_eq!(
            retry,
            Retry::Never,
            "backing off and trying the same bad inputs again is a loop nobody benefits from"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
}
