// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Running one node.
//!
//! Three behaviours, and which one a node has is declared in its spec rather than decided here:
//! it runs, it routes (choosing which arms fire), or it steps (it may ask to be re-entered, or to
//! halt the run and wait for the world).
//!
//! Timeouts are supervised on a thread only when a node asks to be. Most nodes are arithmetic and
//! putting each one on its own thread would cost more than the work — but a node that talks to a
//! network needs a way back, and a run that hangs forever is indistinguishable from one that is
//! merely slow.
use super::*;
use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::registry::NodeRegistry;
use crate::spec::{Behavior, Next, NodeCx, NodeError, NodeRun, NodeSpec, Step, StepCx, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A port a node returns but never declared.
///
/// Nothing downstream can wire to it, so the value silently goes nowhere and the graph looks
/// correct on the canvas. Resolved against THIS node's config, because `Ports::Dynamic` derives
/// the list from it — checking against an empty config would fail every configurable node.
///
/// Reported, never fatal. The engine does not decide the severity: a product whose tests should
/// fail on this makes its observer panic, and a running installation logs it and carries on.
/// Deciding here would force one answer on every consumer — and asserting in the engine also makes
/// a deliberate negative test impossible to write.
fn check_declared<H: Host>(
    spec: &NodeSpec<H>,
    config: &Json,
    out: &PortValues,
    nid: u32,
    host: &H,
) {
    let ports = spec.outputs.resolve(config);
    let declared: HashSet<&str> = ports.iter().map(|p| p.name.as_str()).collect();

    for name in out.keys() {
        if !declared.contains(name.as_str()) {
            let msg = format!(
                "node `{}` returned undeclared output port `{}` — nothing can wire to it \
                 (declared: {:?})",
                spec.id.as_str(),
                name.as_str(),
                declared
            );
            host.observer().defect(nid, &msg);
        }
    }
}

/// Run one node and record what it produced.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute<M: GraphMeta, H: Host<Meta = M>>(
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
    let vars = st.vars.clone();

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
            fire_default(spec, host, nid, &node.config, st);
            Ok(false)
        }

        Behavior::Run(runner) => {
            // Inputs first: pulling a pure source announces that source, and announcing this
            // node before its inputs would report the consumer as having started before the
            // thing it consumes.
            let inputs = gather(graph, reg, host, nid, st)?;
            host.observer().node_started(nid);
            let declared = spec.inputs.resolve(&node.config);
            let (out, summary, ms) = run_timed(
                runner,
                &spec.timeout,
                &node.config,
                &inputs,
                nid,
                host,
                &vars,
                &declared,
            )
            .map_err(fail)?;
            host.observer().node_finished(nid, &summary, ms);
            check_declared(spec, &node.config, &out, nid, host);
            record(host, graph.id, nid, instance, &out, st);
            st.outputs.insert(nid, out);
            st.ran.insert(nid);
            fire_default(spec, host, nid, &node.config, st);
            Ok(false)
        }

        Behavior::Route(router) => {
            // Inputs first: pulling a pure source announces that source, and announcing this
            // node before its inputs would report the consumer as having started before the
            // thing it consumes.
            let inputs = gather(graph, reg, host, nid, st)?;
            host.observer().node_started(nid);
            let as_run: Arc<dyn NodeRun<H>> = router.clone();
            let declared = spec.inputs.resolve(&node.config);
            let (out, summary, ms) = run_timed(
                &as_run,
                &spec.timeout,
                &node.config,
                &inputs,
                nid,
                host,
                &vars,
                &declared,
            )
            .map_err(fail)?;
            let cx = NodeCx {
                config: &node.config,
                inputs: &inputs,
                node: nid,
                host,
                vars: vars.clone(),
                declared_inputs: Some(&declared),
            };
            let arms = router.arms(&cx, &out);
            host.observer().node_finished(nid, &summary, ms);
            for a in arms {
                fire_arm(host, st, nid, a);
            }
            check_declared(spec, &node.config, &out, nid, host);
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
                vars: vars.clone(),
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
                fire_arm(host, st, nid, a.clone());
            }
            check_declared(spec, &node.config, &step.outputs, nid, host);
            record(host, graph.id, nid, instance, &step.outputs, st);
            st.outputs.insert(nid, step.outputs);
            st.ran.insert(nid);
            if step.next == Next::Halt {
                st.halted = true;
            }
            Ok(step.next == Next::Reenter)
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
// Eight, because a node needs its config, its inputs, its declared ports and the run's variables,
// and bundling them into a struct here would only move the same eight across the boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_timed<H: Host>(
    runner: &Arc<dyn NodeRun<H>>,
    timeout: &Timeout,
    config: &Json,
    inputs: &PortValues,
    nid: u32,
    host: &H,
    vars: &crate::spec::Vars,
    declared: &[Port],
) -> Result<(PortValues, String, u128), NodeError> {
    let started = Instant::now();

    // Settled against this node's own configuration, because "too long" is a property a person
    // can set per node and a spec that could only state the default silently overrode them.
    if let Some(secs) = timeout.resolve(config) {
        let (tx, rx) = std::sync::mpsc::channel();
        let runner = Arc::clone(runner);
        // Cloning the inputs is cheap where it matters: `Bytes` holds an `Arc<[u8]>`, so a frame
        // on a wire is a refcount bump rather than a copy.
        let (config, inputs, host) = (config.clone(), inputs.clone(), host.clone());
        // An Arc: the thread shares the run's variables rather than a copy of them.
        let vars = vars.clone();
        let declared = declared.to_vec();
        std::thread::spawn(move || {
            let cx = NodeCx {
                config: &config,
                inputs: &inputs,
                node: nid,
                host: &host,
                vars,
                declared_inputs: Some(&declared),
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
        vars: vars.clone(),
        declared_inputs: Some(declared),
    };
    let out = runner.run(&cx)?;
    let summary = runner.summary(&cx, &out);
    Ok((out, summary, started.elapsed().as_millis()))
}

/// Commit a node's outputs, if this run is committing them.
pub(crate) fn record<H: Host>(
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

/// Fire one exec arm: mark it live, and say so.
///
/// Every arm goes through here. Marking an arm live and reporting it are the same event seen from
/// two sides — a scheduler where they are two statements is one that eventually fires an arm it
/// never reports, and an editor downstream of that lights the wrong edge.
pub(crate) fn fire_arm<H: Host>(host: &H, st: &mut State, nid: u32, port: PortName) {
    host.observer().arm(nid, port.as_str());
    st.live_arms.insert((nid, port));
}

/// Fire the node's exec arms for a node that does not choose them itself.
pub(crate) fn fire_default<H: Host>(
    spec: &NodeSpec<H>,
    host: &H,
    nid: u32,
    config: &Json,
    st: &mut State,
) {
    for p in spec.exec_out.resolve(config) {
        fire_arm(host, st, nid, p.name);
    }
}
