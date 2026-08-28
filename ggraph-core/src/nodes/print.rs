// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `print` — put a line in the run log.
//!
//! The node people reach for first when a graph does not do what they expected, so it does the
//! obvious thing and nothing else: whatever is on `message`, rendered, handed to the observer.

use crate::host::Host;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

/// The one port in the standard set that still takes anything, and it is not a hole.
///
/// Every other `any` was a node making a promise it could not keep. This one promises nothing: it
/// looks at what is on a wire and writes down what it saw. Refusing a table here would mean the
/// node that exists to show you what went wrong is the one node you cannot point at the thing that
/// went wrong.
static IN: [Port; 1] = [Port::opt("message", PortType::ANY)];

struct Print;

impl<H: Host> NodeRun<H> for Print {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let text = cx
            .input_or_cfg("message")
            .as_ref()
            .map(Value::summary)
            .unwrap_or_default();
        cx.host.observer().ui(
            cx.node,
            crate::host::UiEvent::Value {
                label: cx.cfg_str("label").unwrap_or("").to_string(),
                value: Value::Text(text),
            },
        );
        Ok(PortValues::new())
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        cx.input_or_cfg("message")
            .as_ref()
            .map(Value::summary)
            .unwrap_or_default()
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("print", "Print", "Debug")
        .with_inputs(Ports::Static(&IN))
        .with_config(|| json!({ "message": "", "label": "" }))
        .with_timeout(Timeout::Inline)
        .running(Print)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn the_message_reaches_the_log() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let cfg = json!({ "message": "", "label": "" });
        let mut inputs = PortValues::new();
        inputs.insert(crate::id::PortName::new("message"), Value::int(42));
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 7,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        let out = r.run(&cx).unwrap();
        assert_eq!(r.summary(&cx, &out), "42");
    }

    #[test]
    fn an_unwired_message_falls_back_to_the_configured_one() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let cfg = json!({ "message": "from the inspector", "label": "" });
        let inputs = PortValues::new();
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 7,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        assert_eq!(
            r.summary(&cx, &r.run(&cx).unwrap()),
            "from the inspector",
            "a port that is not wired takes its value from the inspector — the rule every node \
             follows, tested here because this is where people notice when it stops holding"
        );
    }
}
