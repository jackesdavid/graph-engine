// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `cooldown` — let one through, then ignore the rest for a while.
//!
//! The node between "a thing happened" and "tell somebody". Without it, a sensor that reports
//! forty times a minute sends forty messages, and the person turns notifications off — which
//! costs more than the missed alert ever would.
//!
//! The last-fired stamp is **durable and instance-scoped**, and both halves matter. Durable,
//! because a cooldown that resets when a process restarts is not a cooldown — a crash loop
//! becomes a message storm. Instance-scoped, because two independent things running the same
//! graph must not silence each other.

use crate::host::{Host, Slot};
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeError, NodeSpec, NodeStep, Step, StepCx, Timeout};
use serde_json::json;

static ARMS: [Port; 2] = [
    Port::opt("passed", PortType::EXEC),
    Port::opt("blocked", PortType::EXEC),
];

struct Cooldown;

impl<H: Host> NodeStep<H> for Cooldown {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        let window: i64 = cx
            .cfg_str("window_secs")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(60);
        let now = cx.host.now_epoch_secs();
        let key = cx.state_key(Slot::State);

        let last = cx
            .host
            .state()
            .get(&key)
            .and_then(|v| v.get("last").and_then(serde_json::Value::as_i64));

        if let Some(last) = last {
            let elapsed = now - last;
            if elapsed < window {
                // `blocked` is an arm rather than nothing, so a graph can count what it dropped.
                // Silence that cannot be observed is indistinguishable from a broken node.
                return Ok(Step::default()
                    .arm("blocked")
                    .logged(format!("{}s left", window - elapsed)));
            }
        }

        cx.host.state().set(&key, &json!({ "last": now }));
        Ok(Step::default().arm("passed"))
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("cooldown", "Cooldown", "Control")
        .about(r#"Lets control through, then blocks it for a set time.

For anything that would otherwise fire repeatedly on a busy trigger — one alert an hour rather than
one a second. It is a gate on control, not on data.

```
Chunk Search --found--> Cooldown --> Send email
```"#)
        .with_exec_out(ExecOut::Static(&ARMS))
        .with_config(|| json!({ "window_secs": "60" }))
        .with_timeout(Timeout::Inline)
        .stepping(Cooldown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;
    use crate::value::PortValues;
    use uuid::Uuid;

    fn fire(host: &TestHost, instance: &str) -> String {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            unreachable!()
        };
        let cfg = json!({ "window_secs": "60" });
        let inputs = PortValues::new();
        let empty = PortValues::new();
        let mut scratch = json!({});
        let mut cx = StepCx {
            vars: Default::default(),
            config: &cfg,
            inputs: &inputs,
            node: 2,
            graph: Uuid::nil(),
            instance,
            forced: false,
            entry_payload: &empty,
            host,
            scratch: &mut scratch,
        };
        node.step(&mut cx).unwrap().arms[0].as_str().to_string()
    }

    #[test]
    fn the_first_one_passes_and_the_next_is_blocked() {
        let host = TestHost::new();
        assert_eq!(fire(&host, ""), "passed");
        assert_eq!(fire(&host, ""), "blocked");
        assert_eq!(fire(&host, ""), "blocked");
    }

    #[test]
    fn it_opens_again_once_the_window_has_passed() {
        let host = TestHost::new();
        assert_eq!(fire(&host, ""), "passed");
        host.advance(59);
        assert_eq!(fire(&host, ""), "blocked");
        host.advance(2);
        assert_eq!(fire(&host, ""), "passed");
    }

    #[test]
    fn two_instances_do_not_silence_each_other() {
        let host = TestHost::new();
        assert_eq!(fire(&host, "north"), "passed");
        assert_eq!(
            fire(&host, "south"),
            "passed",
            "one instance's cooldown must not mute another's — they are independent things that \
             happen to share a graph"
        );
        assert_eq!(fire(&host, "north"), "blocked");
    }
}
