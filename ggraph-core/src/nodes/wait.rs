// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `wait` — carry on later.
//!
//! It does not sleep. Sleeping holds a thread, a connection and a lease across whatever happens
//! next, including a deploy, and a graph that waits an hour would hold them for an hour.
//!
//! Instead it asks the host to bring control back here at a given time and ends the run. How
//! that wake-up is delivered is the host's business: a durable queue, a timer wheel, or — for a
//! host that genuinely wants a blocking one-second pause — a thread that sleeps and re-enters.
//! The node does not care, and that is what lets the same node serve a pipeline where a wait is
//! milliseconds and a workflow where it is a week.

use crate::host::Host;
use crate::port::{Port, PortType};
use crate::spec::{NodeError, NodeSpec, NodeStep, Ports, Step, StepCx, Timeout};
use crate::value::Value;
use serde_json::json;

static IN: [Port; 1] = [Port::opt("seconds", PortType::NUM)];

struct Wait;

impl<H: Host> NodeStep<H> for Wait {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        // Woken up: control is back, carry on.
        if cx.forced {
            return Ok(Step::default().arm("exec_out"));
        }

        let secs = cx
            .input("seconds")
            .and_then(Value::as_f64)
            .or_else(|| cx.cfg_str("seconds").and_then(|s| s.trim().parse().ok()))
            .unwrap_or(1.0);
        if secs <= 0.0 {
            // Waiting for no time is not waiting. Scheduling a wake-up for the past would make
            // it a round trip through the host for nothing.
            return Ok(Step::default().arm("exec_out"));
        }

        let at = cx.host.now_epoch_secs() + secs.ceil() as i64;
        cx.host.schedule(at, cx.target())?;
        Ok(Step::default().halted().logged(format!("waiting {secs}s")))
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("wait", "Wait", "Control")
        .about(r#"Pauses the graph for a number of seconds, then carries on.

For pacing something that will not be hurried — an API that rate-limits, a device that needs a
moment. It holds the run, so a long wait is a long run.

```
HTTP Request --> Wait (30) --> HTTP Request
```"#)
        .with_inputs(Ports::Static(&IN))
        .with_config(|| json!({ "seconds": "1" }))
        .with_timeout(Timeout::Inline)
        .stepping(Wait)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;
    use crate::spec::Next;
    use crate::value::PortValues;
    use serde_json::Value as Json;
    use uuid::Uuid;

    fn step(cfg: Json, forced: bool, host: &TestHost) -> Step {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            unreachable!()
        };
        let inputs = PortValues::new();
        let empty = PortValues::new();
        let mut scratch = json!({});
        let mut cx = StepCx {
            vars: Default::default(),
            config: &cfg,
            inputs: &inputs,
            node: 5,
            graph: Uuid::nil(),
            instance: "",
            forced,
            entry_payload: &empty,
            host,
            scratch: &mut scratch,
        };
        node.step(&mut cx).unwrap()
    }

    #[test]
    fn it_schedules_a_wake_up_and_ends_the_run() {
        let host = TestHost::new();
        let now = host.now_epoch_secs();
        let s = step(json!({ "seconds": "30" }), false, &host);
        assert!(
            (s.next == Next::Halt),
            "a run that sleeps holds a thread across a deploy"
        );
        assert!(s.arms.is_empty());
        let scheduled = host.inner().scheduled.lock().unwrap().clone();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].0, now + 30);
        assert_eq!(scheduled[0].1.node, 5, "it must come back to this node");
    }

    #[test]
    fn waking_up_carries_on() {
        let host = TestHost::new();
        let s = step(json!({ "seconds": "30" }), true, &host);
        assert!(!(s.next == Next::Halt));
        assert_eq!(
            s.arms.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
            vec!["exec_out"]
        );
        assert!(
            host.inner().scheduled.lock().unwrap().is_empty(),
            "waking up must not schedule another wake-up, or the wait never ends"
        );
    }

    #[test]
    fn a_zero_wait_does_not_take_a_trip_through_the_host() {
        let host = TestHost::new();
        let s = step(json!({ "seconds": "0" }), false, &host);
        assert!(!(s.next == Next::Halt));
        assert!(host.inner().scheduled.lock().unwrap().is_empty());
    }
}
