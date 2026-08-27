// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `if` — send control down one arm or another.
//!
//! ## Three arms, not two
//!
//! A branch on a boolean has two outcomes. But the *condition* has three: true, false, and "I
//! could not tell". A field that was not in the document, an extraction the model was not
//! confident about, a query that returned no rows — these are not `false`. Folding them into
//! the false arm means a workflow treats "the two documents disagree" and "I could not read one
//! of them" identically, and those need different people to look at them.
//!
//! So the third arm exists, and it is called `unknown` rather than `error`, because not
//! knowing is an ordinary outcome here rather than a failure.
//!
//! It is **opt-in**, and that is not timidity. Exec arms are pins on a canvas: turning the
//! third one on unconditionally would grow a pin on every branch in every saved graph, in front
//! of everyone, for a case most of them do not have.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeCx, NodeError, NodeRoute, NodeRun, NodeSpec, Ports};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("condition", PortType::BOOL)];
static TWO_ARMS: [Port; 2] = [
    Port::opt("true", PortType::EXEC),
    Port::opt("false", PortType::EXEC),
];
static THREE_ARMS: [Port; 3] = [
    Port::opt("true", PortType::EXEC),
    Port::opt("false", PortType::EXEC),
    Port::opt("unknown", PortType::EXEC),
];

fn wants_unknown(cfg: &Json) -> bool {
    cfg.get("unknown_arm")
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

fn arms(cfg: &Json) -> Vec<Port> {
    if wants_unknown(cfg) {
        THREE_ARMS.to_vec()
    } else {
        TWO_ARMS.to_vec()
    }
}

struct Branch;

impl<H: Host> NodeRun<H> for Branch {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        // With the third arm on, an absent condition is a routing outcome, not an error. With it
        // off, it is an error — which is the behaviour a two-armed branch has to keep, because
        // the alternative is silently taking the false arm.
        if !wants_unknown(cx.config) {
            let v = cx.require("condition")?;
            if v.as_bool().is_none() {
                return Err(NodeError::new(format!(
                    "condition must be a boolean, got {}",
                    v.port_type()
                )));
            }
        }
        Ok(PortValues::new())
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        match cx.input("condition").and_then(Value::as_bool) {
            Some(true) => "true".into(),
            Some(false) => "false".into(),
            None => "unknown".into(),
        }
    }
}

impl<H: Host> NodeRoute<H> for Branch {
    fn arms(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> Vec<PortName> {
        let arm = match cx.input("condition").and_then(Value::as_bool) {
            Some(true) => "true",
            Some(false) => "false",
            None => "unknown",
        };
        vec![PortName::new(arm)]
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("if", "Branch", "Control")
        .with_inputs(Ports::Static(&IN))
        .with_exec_out(ExecOut::dynamic(arms))
        .with_config(|| json!({ "unknown_arm": false }))
        .routing(Branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    fn route(condition: Option<Value>, unknown_arm: bool) -> Vec<String> {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let cfg = json!({ "unknown_arm": unknown_arm });
        let mut inputs = PortValues::new();
        if let Some(v) = condition {
            inputs.insert(PortName::new("condition"), v);
        }
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let Behavior::Route(r) = &s.behavior else {
            panic!("a branch must route")
        };
        let out = r.run(&cx).unwrap_or_default();
        r.arms(&cx, &out)
            .iter()
            .map(|p| p.as_str().to_string())
            .collect()
    }

    #[test]
    fn true_and_false_take_their_arms() {
        assert_eq!(route(Some(Value::Bool(true)), false), vec!["true"]);
        assert_eq!(route(Some(Value::Bool(false)), false), vec!["false"]);
    }

    #[test]
    fn with_the_third_arm_on_an_absent_condition_is_not_a_no() {
        assert_eq!(
            route(None, true),
            vec!["unknown"],
            "'I could not tell' and 'no' send different people to look at it"
        );
    }

    #[test]
    fn with_the_third_arm_off_an_absent_condition_fails_rather_than_routing() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let cfg = json!({ "unknown_arm": false });
        let inputs = PortValues::new();
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let Behavior::Route(r) = &s.behavior else {
            unreachable!()
        };
        assert!(
            r.run(&cx).is_err(),
            "a two-armed branch must not quietly treat a missing condition as false"
        );
    }

    #[test]
    fn a_saved_branch_does_not_grow_a_pin() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        assert_eq!(
            s.exec_out.resolve(&json!({})).len(),
            2,
            "a graph saved before the third arm existed must look exactly as it did"
        );
        assert_eq!(s.exec_out.resolve(&json!({ "unknown_arm": true })).len(), 3);
    }
}
