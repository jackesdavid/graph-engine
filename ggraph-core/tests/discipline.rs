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

/// Produces `false`. The plain test host reads no literals — that is a capability a product
/// supplies — so a branch in a test is fed by a wire, like a branch in a real graph usually is.
struct Falsehood;

static FALSEHOOD_OUT: [ggraph_core::Port; 1] =
    [ggraph_core::Port::opt("out", ggraph_core::PortType::BOOL)];

impl NodeRun<TestHost> for Falsehood {
    fn run(&self, _cx: &NodeCx<'_, TestHost>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        out.insert(ggraph_core::PortName::new("out"), Value::Bool(false));
        Ok(out)
    }
}

/// Options for a graph that is a single node with no edges.
///
/// A bag of one. `Isolated::Skip` is the default and the right one for a canvas full of leftovers,
/// so a test whose whole graph is one unwired node has to say it means it — otherwise the run
/// succeeds without running anything and the test proves nothing at all.
fn lone_node() -> RunOptions {
    RunOptions {
        isolated: ggraph_core::exec::Isolated::Run,
        ..Default::default()
    }
}

fn registry(counter: Counter) -> NodeRegistry<TestHost> {
    let mut r = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut r, &ggraph_core::Services::none());
    r.register(
        NodeSpec::effectful("slow", "Slow", "Test")
            .with_config(|| json!({ "sleep_ms": "0" }))
            .with_timeout(Timeout::Secs(1))
            .running(Slow),
    );
    r.register(
        NodeSpec::effectful("falsehood", "Falsehood", "Test")
            .with_outputs(Ports::Static(&FALSEHOOD_OUT))
            .running(Falsehood),
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
    let err = ggraph_core::run(&g, &reg, &host, &Entry::default(), &lone_node())
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
    assert!(
        matches!(spec.timeout, Timeout::Inline),
        "a comparison declares it cannot block"
    );
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
    ggraph_core::run(&g, &reg, &host, &Entry::default(), &lone_node()).expect("runs");

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
    match ggraph_core::run(&g, &reg, &host, &Entry::default(), &lone_node()) {
        Err(RunError::Node { retry, .. }) => assert_eq!(
            retry,
            Retry::Never,
            "backing off and trying the same bad inputs again is a loop nobody benefits from"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// An unwired input takes its value from the node's configuration, and a **required** one
/// counts as satisfied when it does.
///
/// Found by a consumer's differential test, not by review: its scheduler injected inspector
/// literals before validating required inputs, this one only read wires, and so a branch whose
/// condition was typed into the inspector — the ordinary way to fill a port — failed with
/// "missing required input" instead of running.
#[test]
fn an_unwired_input_falls_back_to_configuration() {
    use ggraph_core::host::{Host, HostError, Literals, NodeTarget, Observer, StateStore};
    use ggraph_core::{Port, PortValues, Value};
    use std::sync::Arc;

    /// Reads a literal straight out of config, by port name.
    struct InspectorFields;

    impl Literals for InspectorFields {
        fn read(
            &self,
            _kind: &ggraph_core::NodeId,
            port: &Port,
            config: &serde_json::Value,
        ) -> Option<Value> {
            match config.get(port.name.as_str())?.as_str()? {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                other if !other.is_empty() => Some(Value::text(other)),
                _ => None,
            }
        }
    }

    /// The test host, plus a reader of inspector fields.
    #[derive(Clone, Default)]
    struct WithLiterals(Arc<TestHost>);

    impl Host for WithLiterals {
        type Meta = ();
        fn state(&self) -> &dyn StateStore {
            self.0.state()
        }
        fn io(&self) -> &dyn ggraph_core::host::ValueIo {
            self.0.io()
        }
        fn observer(&self) -> &dyn Observer {
            self.0.observer()
        }
        fn run_id(&self) -> uuid::Uuid {
            self.0.run_id()
        }
        fn now_epoch_secs(&self) -> i64 {
            self.0.now_epoch_secs()
        }
        fn schedule(&self, at: i64, t: NodeTarget) -> Result<(), HostError> {
            self.0.schedule(at, t)
        }
        fn literals(&self) -> &dyn Literals {
            &InspectorFields
        }
    }

    let mut reg: NodeRegistry<WithLiterals> = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut reg, &ggraph_core::Services::none());

    let mut g: Graph = Graph::new("literal condition");
    let br = g.add_node(NodeId::new("if"), 0, 0);
    // The condition is typed into the inspector rather than wired. This is the common case.
    g.node_mut(br).unwrap().config = json!({ "condition": "true", "unknown_arm": false });
    let taken = g.add_node(NodeId::new("print"), 200, 0);
    g.node_mut(taken).unwrap().config = json!({ "message": "yes" });
    g.add_edge(&reg, br, "true", taken, "exec_in").unwrap();

    let host = WithLiterals::default();
    ggraph_core::run(&g, &reg, &host, &Entry::default(), &lone_node())
        .expect("a condition typed into the inspector must satisfy the required port");

    let ran = host.0.inner().observer.started.lock().unwrap().clone();
    assert!(
        ran.contains(&taken),
        "the arm must fire from a configured condition, not only from a wired one: {ran:?}"
    );
    let _ = PortValues::new();
}

/// Every way a run can fail answers the retry question, not one of the three.
///
/// A durable host decides backoff from `err.retry()`. Making it ask the variant instead means
/// knowing which ones are permanent — which is knowledge about the engine's internals, and
/// exactly what the judgement was added to stop.
#[test]
fn every_run_failure_says_whether_retrying_could_help() {
    use ggraph_core::Retry;

    let reg = registry(Counter::default());
    let host = TestHost::new();

    let mut unknown: Graph = Graph::new("stale");
    unknown.add_node(NodeId::new("a_kind_from_another_deploy"), 0, 0);
    let e = ggraph_core::run(
        &unknown,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions::default(),
    )
    .expect_err("an unregistered kind cannot run");
    assert_eq!(
        e.retry(),
        Retry::Never,
        "the build will not learn the kind by being asked again"
    );

    let mut runaway: Graph = Graph::new("budget");
    let each = runaway.add_node(NodeId::new("for_each"), 0, 0);
    runaway.node_mut(each).unwrap().config = json!({ "items": "a,b,c,d,e,f" });
    let e = ggraph_core::run(
        &runaway,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            budget: ggraph_core::Budget { max_steps: 2 },
            ..lone_node()
        },
    )
    .expect_err("a run past its ceiling fails");
    assert_eq!(
        e.retry(),
        Retry::Never,
        "the same graph reaches the same ceiling"
    );
}

/// The run's end is reported on both paths, including the one that failed.
///
/// An observer that buffers — one deciding whether to report a node based on what happened
/// later in the run — has no other moment to make that call. Firing this only on success loses
/// everything it was holding exactly when a run went wrong, which is the run somebody most
/// wants the report from.
#[test]
fn the_observer_learns_the_run_ended_even_when_it_failed() {
    let reg = registry(Counter::default());
    let host = TestHost::new();

    let mut good: Graph = Graph::new("fine");
    good.add_node(NodeId::new("counter"), 0, 0);
    assert!(ggraph_core::run(
        &good,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions::default()
    )
    .is_ok());
    assert_eq!(*host.inner().observer.ends.lock().unwrap(), 1);

    // A graph that cannot run: the kind was never registered.
    let mut bad: Graph = Graph::new("doomed");
    bad.add_node(NodeId::new("no_such_kind"), 0, 0);
    assert!(
        ggraph_core::run(&bad, &reg, &host, &Entry::default(), &RunOptions::default()).is_err()
    );
    assert_eq!(
        *host.inner().observer.ends.lock().unwrap(),
        2,
        "a failed run must still tell the observer it is over"
    );
}

/// The scheduler says which arm control left by, not merely which node it reached.
///
/// Downstream of this an editor draws the live edge. Told only the destination it has to guess
/// the edge, and the guess is right until two branches converge on one node — then BOTH edges
/// light, including the one from the branch that never ran.
#[test]
fn an_arm_is_reported_with_the_port_it_left_by() {
    let reg = registry(Counter::default());
    let host = TestHost::new();

    let mut g: Graph = Graph::new("converging");
    let cond = g.add_node(NodeId::new("falsehood"), 0, 0);
    let gate = g.add_node(NodeId::new("if"), 0, 0);
    let target = g.add_node(NodeId::new("counter"), 0, 0);
    g.add_edge(&reg, cond, "out", gate, "condition").unwrap();
    g.add_edge(&reg, gate, "true", target, "exec_in").unwrap();
    g.add_edge(&reg, gate, "false", target, "exec_in").unwrap();

    ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()).expect("runs");

    let arms = host.inner().observer.arms.lock().unwrap().clone();
    assert!(
        arms.contains(&(gate, "false".to_string())),
        "the arm that fired must be named: {arms:?}"
    );
    assert!(
        !arms.contains(&(gate, "true".to_string())),
        "the arm that did not fire must not be: {arms:?}"
    );
}

/// A resumption can read back what earlier runs left without the engine writing checkpoints.
///
/// These were one flag, and conflating them made a whole product's policy inexpressible: Sentinel
/// remembers each node's last value on purpose — its editor shows them, and a window closing hours
/// later on a timer has no other way to see what the arming run saw — while wanting nothing
/// written or cleared mid-run. Writing checkpoints is about surviving a crash; restoring is about
/// resuming something left open. Different questions.
#[test]
fn a_resumption_restores_without_the_engine_checkpointing() {
    use ggraph_core::host::Host;

    let reg = registry(Counter::default());
    let host = TestHost::new();

    let mut g: Graph = Graph::new("resumed");
    let earlier = g.add_node(NodeId::new("counter"), 0, 0);
    let later = g.add_node(NodeId::new("counter"), 0, 0);
    g.add_edge(&reg, earlier, "exec_out", later, "exec_in")
        .unwrap();

    // What an earlier run of this instance left behind, put there by hand: with Checkpoint::None
    // the engine writes nothing itself, which is the point.
    let mut out = PortValues::new();
    out.insert(ggraph_core::PortName::new("n"), Value::int(41));
    host.state().set(
        &ggraph_core::exec::values_key(g.id, earlier, ""),
        &ggraph_core::codec::encode_ports(&out, host.io()),
    );

    let entry = Entry {
        at: vec![later],
        restore: true,
        ..Default::default()
    };
    ggraph_core::run(&g, &reg, &host, &entry, &RunOptions::default()).expect("runs");

    let started = host.inner().observer.started.lock().unwrap().clone();
    assert_eq!(
        started,
        vec![later],
        "only the node it entered at runs; the earlier one is restored, not re-run"
    );
}

/// A node's timeout can come from its own configuration, not only from its kind.
///
/// The same reason `Ports::Dynamic` exists. A person who sets a slow endpoint to sixty seconds
/// means it, and a spec that can only state the kind's default silently gives them the default —
/// which surfaces as a node that "randomly" fails on exactly the requests it was configured to
/// wait for.
#[test]
fn a_node_may_take_as_long_as_its_configuration_says() {
    let mut reg = registry(Counter::default());
    reg.register(
        NodeSpec::effectful("patient", "Patient", "Test")
            .with_config(|| json!({ "sleep_ms": "0", "timeout_secs": "" }))
            .with_timeout(Timeout::from_config(|cfg| {
                cfg.get("timeout_secs")?.as_str()?.parse::<u64>().ok()
            }))
            .running(Slow),
    );

    let host = TestHost::new();
    let mut g: Graph = Graph::new("patient");
    let n = g.add_node(NodeId::new("patient"), 0, 0);
    // Sleeps past the one second the kind's siblings get, and says so in its own configuration.
    g.node_mut(n).unwrap().config["sleep_ms"] = json!("1500");
    g.node_mut(n).unwrap().config["timeout_secs"] = json!("30");

    ggraph_core::run(&g, &reg, &host, &Entry::default(), &lone_node())
        .expect("a node configured to take longer is allowed to take longer");
}

/// A node wired to nothing does not run, unless the run says otherwise.
///
/// It has no incoming exec edge, so by the plain reading it is somewhere control can start. But a
/// canvas collects leftovers — a node dropped while trying something out and never wired — and
/// running those is how a graph sends a notification nobody asked for.
#[test]
fn a_node_wired_to_nothing_is_left_alone() {
    let counter = Counter::default();
    let reg = registry(counter.clone());
    let host = TestHost::new();

    let mut g: Graph = Graph::new("with a leftover");
    let wired_a = g.add_node(NodeId::new("counter"), 0, 0);
    let wired_b = g.add_node(NodeId::new("counter"), 0, 0);
    g.add_edge(&reg, wired_a, "exec_out", wired_b, "exec_in")
        .unwrap();
    let leftover = g.add_node(NodeId::new("counter"), 0, 0);

    ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()).expect("runs");
    let ran = host.inner().observer.started.lock().unwrap().clone();
    assert_eq!(
        ran,
        vec![wired_a, wired_b],
        "the leftover must not run just because nothing points at it"
    );

    // And a graph that IS a bag of independent nodes can say so.
    let opts = RunOptions {
        isolated: ggraph_core::exec::Isolated::Run,
        ..Default::default()
    };
    let host = TestHost::new();
    ggraph_core::run(&g, &reg, &host, &Entry::default(), &opts).expect("runs");
    let ran = host.inner().observer.started.lock().unwrap().clone();
    assert!(
        ran.contains(&leftover),
        "asked for explicitly, it runs: {ran:?}"
    );
}
