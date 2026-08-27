// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Things the engine declares, verified to actually happen.
//!
//! A `NodeSpec` says a node has a timeout and an `Observer` is handed an elapsed time. Both
//! were, for a while, declarations the scheduler ignored — which is worse than not offering
//! them, because a consumer sets `Timeout::Secs(30)` and believes it.
//!
//! A third one, `memoize`, was deleted instead of implemented. See the loop test below.
//!
//! These tests exist so that stays fixed.

use ggraph_core::exec::{Entry, Isolated, RunOptions};
use ggraph_core::host::testkit::TestHost;
use ggraph_core::host::Host;
use ggraph_core::port::{Column, Port, PortType};
use ggraph_core::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use ggraph_core::{Graph, NodeId, NodeRegistry, PortName, PortValues, Retry, RunError, Value};
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

/// What `counter` returns. Declared because the engine now checks — and the check caught this
/// very node returning `n` without declaring it.
static COUNTER_OUT: [Port; 1] = [Port::opt("n", PortType::NUM)];

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
            .with_outputs(Ports::Static(&COUNTER_OUT))
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

/// O registry enumera-se, e em ordem estável.
///
/// Sem isto não há catálogo, e o catálogo é como um editor descobre o que pode desenhar. Um produto
/// com um enum de tipos itera o enum; um que use identificadores abertos — que é o que esta engine
/// oferece — só tem o registry. Sem enumeração, cada consumidor mantinha uma segunda lista à parte,
/// e duas listas da mesma coisa divergem.
#[test]
fn o_catalogo_sai_do_registry_e_sai_igual_duas_vezes() {
    let reg = registry(Counter::default());

    let uma: Vec<&str> = reg.iter().map(|s| s.id.as_str()).collect();
    let outra: Vec<&str> = reg.iter().map(|s| s.id.as_str()).collect();

    assert_eq!(uma, outra, "duas leituras têm de dar a mesma ordem");
    assert_eq!(uma.len(), reg.len(), "enumera tudo o que está registado");
    assert!(uma.windows(2).all(|w| w[0] <= w[1]), "ordenado: {uma:?}");
    assert!(uma.contains(&"counter"), "inclui o que foi registado à mão");
}

/// A node that returns a port it never declared is caught.
///
/// The value goes nowhere — nothing can wire to a port the catalog does not list — so the graph
/// looks correct on the canvas and the flow silently loses what the node produced.
#[test]
fn returning_an_undeclared_port_is_reported() {
    struct Leaky;
    impl<H: Host> NodeRun<H> for Leaky {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            out.insert(PortName::new("declared"), Value::int(1));
            out.insert(PortName::new("surprise"), Value::int(2));
            Ok(out)
        }
    }

    static OUT: [Port; 1] = [Port::opt("declared", PortType::NUM)];

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("leaky", "Leaky", "Test")
            .with_outputs(Ports::Static(&OUT))
            .running(Leaky),
    );

    let mut g: Graph = Graph::new("leaky");
    g.add_node(NodeId::new("leaky"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    let defects = host.inner().observer.defects.lock().unwrap().clone();
    assert_eq!(defects.len(), 1, "one undeclared port: {defects:?}");
    assert!(defects[0].contains("surprise"), "{}", defects[0]);
}

/// And a node that declares what it returns reports nothing — the check must not cry wolf, or it
/// gets muted and stops catching the real thing.
#[test]
fn a_node_that_declares_what_it_returns_is_silent() {
    struct Honest;
    impl<H: Host> NodeRun<H> for Honest {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            out.insert(PortName::new("value"), Value::int(1));
            Ok(out)
        }
    }

    static OUT: [Port; 1] = [Port::opt("value", PortType::NUM)];

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("honest", "Honest", "Test")
            .with_outputs(Ports::Static(&OUT))
            .running(Honest),
    );

    let mut g: Graph = Graph::new("honest");
    g.add_node(NodeId::new("honest"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    assert!(host.inner().observer.defects.lock().unwrap().is_empty());
}

/// Reading an input port the node never declared is caught.
///
/// It returns `None` forever, indistinguishable from a declared port that is simply unwired —
/// which is how a renamed port becomes a node that quietly does nothing.
#[test]
fn reading_an_undeclared_input_is_reported() {
    struct Confused;
    impl<H: Host> NodeRun<H> for Confused {
        fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let _ = cx.input("document"); // declared
            let _ = cx.input("fille"); // typo
            Ok(PortValues::new())
        }
    }

    static IN: [Port; 1] = [Port::opt("document", PortType::TEXT)];

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("confused", "Confused", "Test")
            .with_inputs(Ports::Static(&IN))
            .running(Confused),
    );

    let mut g: Graph = Graph::new("confused");
    g.add_node(NodeId::new("confused"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    let defects = host.inner().observer.defects.lock().unwrap().clone();
    assert_eq!(defects.len(), 1, "only the typo: {defects:?}");
    assert!(defects[0].contains("fille"), "{}", defects[0]);
}

/// A hand-built context declares nothing, and reports nothing.
///
/// "This node declares no inputs" and "nobody told me what it declares" want opposite answers.
/// Conflating them would make every fabricated test context report a defect, and a check that
/// cries wolf gets muted.
#[test]
fn a_fabricated_context_reports_nothing() {
    let host = TestHost::new();
    let cfg = json!({});
    let inputs = PortValues::new();
    let cx = NodeCx {
        config: &cfg,
        inputs: &inputs,
        node: 1,
        host: &host,
        vars: Default::default(),
        declared_inputs: None,
    };

    assert!(cx.input("anything").is_none());
    assert!(host.inner().observer.defects.lock().unwrap().is_empty());
}

/// A report assembled by a graph, with a row nested inside a column.
///
/// The property the whole design rests on: a layout takes blocks and IS a block, so it takes
/// layouts. If this stops working, every arrangement more complex than a list stops with it — and a
/// column of stacked things is what a linear chain already produced.
///
/// Note what the graph does NOT have: exec wires between components. They are pure, so the render
/// pulls the whole tree backwards through the data wires on its own.
#[test]
fn a_graph_assembles_a_nested_report() {
    use ggraph_core::report::{Block, Direction};

    let reg: NodeRegistry<TestHost> = {
        let mut r = NodeRegistry::new();
        ggraph_core::nodes::register_all(&mut r, &ggraph_core::Services::none());
        r
    };

    let mut g: Graph = Graph::new("report");
    let heading = g.add_node(NodeId::new("report_heading"), 0, 0);
    let table = g.add_node(NodeId::new("report_table"), 200, 100);
    let chart = g.add_node(NodeId::new("report_bar_chart"), 200, 200);
    let row = g.add_node(NodeId::new("report_layout"), 400, 150);
    let col = g.add_node(NodeId::new("report_layout"), 600, 75);
    // A pure node is PULLED, never entered. Something effectful has to want its value, and `print`
    // is the smallest thing that does — the report's own render needs a blob store the test host
    // does not have.
    let sink = g.add_node(NodeId::new("print"), 800, 75);

    g.node_mut(heading).unwrap().config["text"] = json!("Findings");
    g.node_mut(table).unwrap().config["columns"] = json!(["Document"]);
    g.node_mut(chart).unwrap().config["title"] = json!("Relevance");
    // The row: table beside chart. What a linear chain could never produce.
    g.node_mut(row).unwrap().config["direction"] = json!("row");
    g.node_mut(row).unwrap().config["slots"] = json!(2);
    g.node_mut(col).unwrap().config["slots"] = json!(2);

    g.add_edge(&reg, table, "block", row, "slot_1").unwrap();
    g.add_edge(&reg, chart, "block", row, "slot_2").unwrap();
    g.add_edge(&reg, heading, "block", col, "slot_1").unwrap();
    g.add_edge(&reg, row, "block", col, "slot_2").unwrap();
    g.add_edge(&reg, col, "block", sink, "message").unwrap();

    let host = TestHost::new();
    let outs = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry {
            at: vec![sink],
            ..Default::default()
        },
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    )
    .expect("the graph runs");

    let Some(Value::Json(j)) = outs.get(&col).and_then(|v| v.get(&PortName::new("block"))) else {
        panic!("the column produced a block")
    };
    let tree: Block = serde_json::from_value(j.clone()).expect("it is a Block");

    let Block::Layout { children, .. } = &tree else {
        panic!("root is a layout")
    };
    assert_eq!(children.len(), 2, "heading and the row");
    let Block::Layout {
        layout,
        children: pair,
    } = &children[1]
    else {
        panic!("the second child is the nested layout")
    };
    assert_eq!(layout.direction, Direction::Row, "a row inside a column");
    assert_eq!(pair.len(), 2, "table beside chart");

    // And it renders as nested flex, which is what a reader sees.
    let html = ggraph_core::report::render_html(&tree, "Report", None);
    assert!(html.contains("flex-direction:column"));
    assert!(html.contains("flex-direction:row"));
    assert!(html.contains("<table"));
    assert!(html.contains("<svg"));
}

/// A node that declares a type and returns something else is caught.
///
/// A declared type is a promise to everything downstream. Without this check the wire is drawn, the
/// editor allows it, and the far end receives something it cannot read — so the failure surfaces in
/// a node that did nothing wrong, which is the expensive kind of failure to chase.
#[test]
fn returning_the_wrong_type_is_reported() {
    struct Liar;
    impl<H: Host> NodeRun<H> for Liar {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            // Declared `numbers`; these are text.
            out.insert(
                PortName::new("values"),
                Value::List(vec![Value::text("a"), Value::text("b")]),
            );
            Ok(out)
        }
    }

    static OUT: [Port; 1] = [Port::opt("values", PortType::NUMBERS)];

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("liar", "Liar", "Test")
            .with_outputs(Ports::Static(&OUT))
            .running(Liar),
    );

    let mut g: Graph = Graph::new("liar");
    g.add_node(NodeId::new("liar"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    let defects = host.inner().observer.defects.lock().unwrap().clone();
    assert_eq!(defects.len(), 1, "{defects:?}");
    assert!(
        defects[0].contains("declared port `values` as `numbers`"),
        "{}",
        defects[0]
    );
    assert!(
        defects[0].contains("a list of text"),
        "it says what arrived: {}",
        defects[0]
    );
}

/// A product's own type is not the engine's to judge.
///
/// `block`, `document`, `rows` — an open identifier is a product's vocabulary, and the engine
/// guessing at its runtime shape would be the engine acquiring knowledge it has no business having.
/// Reporting a defect on one would make the check cry wolf on every product type, and a check that
/// cries wolf gets muted.
#[test]
fn a_product_type_is_left_alone() {
    struct Custom;
    impl<H: Host> NodeRun<H> for Custom {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            out.insert(
                PortName::new("thing"),
                Value::text("whatever this product means"),
            );
            Ok(out)
        }
    }

    static OUT: [Port; 1] = [Port::opt("thing", PortType::new_static("invoice"))];

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("custom", "Custom", "Test")
            .with_outputs(Ports::Static(&OUT))
            .running(Custom),
    );

    let mut g: Graph = Graph::new("custom");
    g.add_node(NodeId::new("custom"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    assert!(host.inner().observer.defects.lock().unwrap().is_empty());
}

/// An empty list satisfies every element type. There is nothing in it to be wrong.
#[test]
fn an_empty_list_is_not_a_defect() {
    assert!(PortType::NUMBERS.accepts(&Value::List(vec![])));
    assert!(PortType::TABLE.accepts(&Value::List(vec![])));
}

/// One bad element is enough. A `numbers` holding one string is the wire that feeds a chart a bar
/// it cannot draw, and finding out on the tenth element is finding out.
#[test]
fn one_wrong_element_fails_the_list() {
    let mostly = Value::List(vec![
        Value::float(1.0),
        Value::text("oops"),
        Value::float(3.0),
    ]);
    assert!(!PortType::NUMBERS.accepts(&mostly));
}

/// A declared column that the rows do not carry is caught.
///
/// The schema is the contract everything downstream is built against. Without this check a source
/// can promise a column, hand back rows without it, and the failure surfaces in a chart that finds
/// nothing to plot — a node that did nothing wrong.
#[test]
fn a_missing_column_is_reported() {
    struct ShortRows;
    impl<H: Host> NodeRun<H> for ShortRows {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            // Declared `document` and `score`; only one of them is here.
            out.insert(
                PortName::new("results"),
                Value::List(vec![Value::Map(vec![(
                    "document".into(),
                    Value::text("a.pdf"),
                )])]),
            );
            Ok(out)
        }
    }

    fn ports(_: &serde_json::Value) -> Vec<Port> {
        vec![Port::opt("results", PortType::TABLE).with_columns(vec![
            Column::new(PortName::new("document"), PortType::TEXT),
            Column::new(PortName::new("score"), PortType::NUM),
        ])]
    }

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("short", "Short", "Test")
            .with_outputs(Ports::dynamic(ports))
            .running(ShortRows),
    );

    let mut g: Graph = Graph::new("short");
    g.add_node(NodeId::new("short"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    let defects = host.inner().observer.defects.lock().unwrap().clone();
    assert_eq!(defects.len(), 1, "{defects:?}");
    assert!(defects[0].contains("column `score`"), "{}", defects[0]);
}

/// An optional column may be absent. That is what optional means, and a check that ignored it
/// would make every schema with a sometimes-empty column noisy enough to be muted.
#[test]
fn an_optional_column_may_be_missing() {
    struct Rows;
    impl<H: Host> NodeRun<H> for Rows {
        fn run(&self, _cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
            let mut out = PortValues::new();
            out.insert(
                PortName::new("results"),
                Value::List(vec![Value::Map(vec![("a".into(), Value::text("x"))])]),
            );
            Ok(out)
        }
    }

    fn ports(_: &serde_json::Value) -> Vec<Port> {
        vec![Port::opt("results", PortType::TABLE).with_columns(vec![
            Column::new(PortName::new("a"), PortType::TEXT),
            Column::optional(PortName::new("b"), PortType::TEXT),
        ])]
    }

    let mut reg: NodeRegistry<TestHost> = NodeRegistry::new();
    reg.register(
        NodeSpec::effectful("rows", "Rows", "Test")
            .with_outputs(Ports::dynamic(ports))
            .running(Rows),
    );

    let mut g: Graph = Graph::new("rows");
    g.add_node(NodeId::new("rows"), 0, 0);

    let host = TestHost::new();
    let _ = ggraph_core::run(
        &g,
        &reg,
        &host,
        &Entry::default(),
        &RunOptions {
            isolated: Isolated::Run,
            ..Default::default()
        },
    );

    assert!(host.inner().observer.defects.lock().unwrap().is_empty());
}
