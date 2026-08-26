// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `debounce` — one signal at the start of a burst, one at the end of it.
//!
//! Something starts happening and keeps happening: a door held open, a person in frame, a
//! folder being written to. What a graph usually wants is not one message per signal, but *"it
//! started"* and, once things go quiet, *"it stopped, and it lasted this long"*.
//!
//! The trailing edge is the hard half, because nothing happens when a burst ends — the absence
//! of a signal is not a signal. So the node arms a window, has the host wake it when the window
//! expires, and every further signal pushes the deadline out. The wake-up that finds the
//! deadline already passed is the one that fires `stop`.
//!
//! ## Why the three state operations are conditional
//!
//! Two pods run the same graph. Both see the same signal. If arming were read-then-write, both
//! would read "idle", both would write "armed", and both would fire `start` — one door open,
//! two messages. Every operation here is a compare-and-set for that reason, and `try_arm`
//! returning `false` is not an error but the normal answer for whoever lost.
//!
//! The same applies at the other end: a wake-up whose deadline has since been extended must do
//! nothing, and let the later one close the window.

use crate::host::{Host, Slot};
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeError, NodeSpec, NodeStep, Ports, Step, StepCx, Timeout};
use serde_json::json;

static OUT: [Port; 1] = [Port::opt("held_secs", PortType::NUM)];
static ARMS: [Port; 2] = [
    Port::opt("start", PortType::EXEC),
    Port::opt("stop", PortType::EXEC),
];

struct Debounce;

impl<H: Host> NodeStep<H> for Debounce {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        let window: i64 = cx
            .cfg_str("window_secs")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(30);
        let now = cx.host.now_epoch_secs();
        let key = cx.state_key(Slot::State);
        let deadline = now + window;

        // The wake-up. Whoever wins the disarm owns the trailing edge.
        if cx.forced {
            if !cx.host.state().try_disarm_expired(&key, now) {
                return Ok(Step::default().logged("still active"));
            }
            let opened = cx
                .host
                .state()
                .get(&cx.state_key(Slot::Values))
                .and_then(|v| v.get("opened").and_then(serde_json::Value::as_i64));
            cx.host.state().clear(&cx.state_key(Slot::Values));

            let mut out = crate::value::PortValues::new();
            if let Some(opened) = opened {
                out.insert(
                    crate::id::PortName::new("held_secs"),
                    crate::value::Value::int((now - window - opened).max(0)),
                );
            }
            return Ok(Step::outputs(out).arm("stop").logged("quiet"));
        }

        if cx.host.state().try_arm(&key, deadline) {
            cx.host
                .state()
                .set(&cx.state_key(Slot::Values), &json!({ "opened": now }));
            cx.host.schedule(deadline, cx.target())?;
            return Ok(Step::default().arm("start"));
        }

        // Already open. Push the deadline out and book a later wake-up; the earlier one will
        // find the window extended and stand down.
        cx.host.state().extend(&key, deadline);
        cx.host.schedule(deadline, cx.target())?;
        Ok(Step::default().logged("extended"))
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("debounce", "Start and Stop", "Control")
        .with_outputs(Ports::Static(&OUT))
        .with_exec_out(ExecOut::Static(&ARMS))
        .with_config(|| json!({ "window_secs": "30" }))
        .with_timeout(Timeout::Inline)
        .stepping(Debounce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;
    use crate::value::{PortValues, Value};
    use uuid::Uuid;

    fn signal(host: &TestHost, forced: bool) -> Step {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            unreachable!()
        };
        let cfg = json!({ "window_secs": "30" });
        let inputs = PortValues::new();
        let empty = PortValues::new();
        let mut scratch = json!({});
        let mut cx = StepCx {
            vars: Default::default(),
            config: &cfg,
            inputs: &inputs,
            node: 4,
            graph: Uuid::nil(),
            instance: "",
            forced,
            entry_payload: &empty,
            host,
            scratch: &mut scratch,
        };
        node.step(&mut cx).unwrap()
    }

    fn arms(s: &Step) -> Vec<&str> {
        s.arms.iter().map(|a| a.as_str()).collect()
    }

    #[test]
    fn the_burst_starts_once_no_matter_how_many_signals() {
        let host = TestHost::new();
        assert_eq!(arms(&signal(&host, false)), vec!["start"]);
        assert!(arms(&signal(&host, false)).is_empty());
        assert!(arms(&signal(&host, false)).is_empty());
    }

    #[test]
    fn the_end_of_the_burst_fires_stop_with_how_long_it_lasted() {
        let host = TestHost::new();
        signal(&host, false);
        host.advance(10);
        signal(&host, false); // still going — pushes the deadline to now+30
        host.advance(30); // quiet since
        let stop = signal(&host, true);
        assert_eq!(arms(&stop), vec!["stop"]);
        assert_eq!(
            stop.outputs
                .get(&crate::id::PortName::new("held_secs"))
                .and_then(Value::as_i64),
            Some(10),
            "the burst lasted from the first signal to the last one, not to the wake-up"
        );
    }

    #[test]
    fn a_wake_up_whose_window_was_extended_stands_down() {
        let host = TestHost::new();
        signal(&host, false);
        host.advance(20);
        signal(&host, false); // deadline is now +30 again
                              // The FIRST wake-up arrives, on the original deadline.
        let early = signal(&host, true);
        assert!(
            arms(&early).is_empty(),
            "closing the window here would end a burst that is still going — the later wake-up \
             owns it"
        );
        host.advance(31);
        assert_eq!(arms(&signal(&host, true)), vec!["stop"]);
    }

    #[test]
    fn a_second_pod_seeing_the_same_signal_does_not_fire_start_twice() {
        // Both share one state store, which is the whole point of `try_arm` being a
        // compare-and-set: one door open must not become two messages.
        let host = TestHost::new();
        let other = host.clone();
        assert_eq!(arms(&signal(&host, false)), vec!["start"]);
        assert!(
            arms(&signal(&other, false)).is_empty(),
            "read-then-write here is how one event becomes two notifications"
        );
    }

    #[test]
    fn every_signal_books_a_wake_up_so_the_trailing_edge_cannot_be_lost() {
        let host = TestHost::new();
        signal(&host, false);
        host.advance(5);
        signal(&host, false);
        let booked = host.inner().scheduled.lock().unwrap().len();
        assert_eq!(
            booked, 2,
            "the absence of a signal is not a signal — if nothing is booked, nothing ever closes \
             the window"
        );
    }
}
