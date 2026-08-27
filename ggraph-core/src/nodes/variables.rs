// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `set_variable` and `get_variable` — a named value shared across a run.
//!
//! Two nodes in one file, against the one-node-per-file rule, because they are one feature and
//! splitting them would put the naming rules in one file and the reading of them in another.
//!
//! Variables are **run-scoped**, not durable. A graph that wants a value to outlive its run
//! wants a table: the difference is visible in the editor, and making a variable quietly
//! persistent is how two runs start overwriting each other's working state.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Purity, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};

fn name_of(cfg: &Json) -> String {
    cfg.get("variable")
        .and_then(Json::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// The port is named after the variable, so a canvas reads `count` rather than `value`.
fn named_port(cfg: &Json) -> Vec<Port> {
    let n = name_of(cfg);
    if n.is_empty() {
        return Vec::new();
    }
    vec![Port::new(PortName::new(n), PortType::ANY, false)]
}

struct SetVariable;

impl<H: Host> NodeRun<H> for SetVariable {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let name = name_of(cx.config);
        if name.is_empty() {
            return Err(NodeError::new("this node has no variable name"));
        }
        let Some(v) = cx.input(&name).cloned() else {
            // Storing nothing would leave the previous value in place, which reads downstream as
            // a fresh one. Refusing says which node and which name.
            return Err(NodeError::new(format!("nothing wired into {name:?}")));
        };
        cx.vars.lock().unwrap().insert(PortName::new(name), v);
        Ok(PortValues::new())
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        let name = name_of(cx.config);
        match cx.input(&name) {
            Some(v) => format!("{name} = {}", v.summary()),
            None => name,
        }
    }
}

struct GetVariable;

impl<H: Host> NodeRun<H> for GetVariable {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let name = name_of(cx.config);
        if name.is_empty() {
            return Err(NodeError::new("this node has no variable name"));
        }
        let mut out = PortValues::new();
        // An unset variable produces nothing rather than a default. A zero here is
        // indistinguishable from a real zero, and the branch's third arm exists for exactly
        // this shape of absence.
        if let Some(v) = cx.vars.lock().unwrap().get(name.as_str()).cloned() {
            out.insert(PortName::new(name), v);
        }
        Ok(out)
    }
}

pub fn set_spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("set_variable", "Set Variable", "Variables")
        .with_inputs(Ports::dynamic(named_port))
        .with_config(|| json!({ "variable": "" }))
        .with_timeout(Timeout::Inline)
        .running(SetVariable)
}

pub fn get_spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::pure("get_variable", "Get Variable", "Variables")
        .with_outputs(Ports::dynamic(named_port))
        .with_config(|| json!({ "variable": "" }))
        // Re-read every time it is asked: a variable set inside a loop must be visible to a
        // reader later in the same loop, not frozen at the value it had on the first pass.
        .with_purity(Purity::PURE_SOURCE)
        .running(GetVariable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;
    use crate::value::Value;

    fn run(
        spec: &NodeSpec<TestHost>,
        cfg: Json,
        inputs: PortValues,
        host: &TestHost,
        vars: &crate::spec::Vars,
    ) -> Result<PortValues, NodeError> {
        let Behavior::Run(r) = &spec.behavior else {
            unreachable!()
        };
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host,
            vars: vars.clone(),
            declared_inputs: None,
        };
        r.run(&cx)
    }

    #[test]
    fn what_is_set_can_be_read() {
        let host = TestHost::new();
        let vars = crate::spec::Vars::default();
        let mut inputs = PortValues::new();
        inputs.insert(PortName::new("count"), Value::int(3));
        run(
            &set_spec(&crate::nodes::services::Services::none()),
            json!({ "variable": "count" }),
            inputs,
            &host,
            &vars,
        )
        .unwrap();

        let got = run(
            &get_spec(&crate::nodes::services::Services::none()),
            json!({ "variable": "count" }),
            PortValues::new(),
            &host,
            &vars,
        )
        .unwrap();
        assert_eq!(
            got.get(&PortName::new("count")).and_then(Value::as_i64),
            Some(3)
        );
    }

    #[test]
    fn an_unset_variable_reads_as_absent_not_as_zero() {
        let host = TestHost::new();
        let vars = crate::spec::Vars::default();
        let got = run(
            &get_spec(&crate::nodes::services::Services::none()),
            json!({ "variable": "count" }),
            PortValues::new(),
            &host,
            &vars,
        )
        .unwrap();
        assert!(
            got.is_empty(),
            "a default here is indistinguishable from a real value, and a graph acting on it \
             cannot tell that it never ran"
        );
    }

    #[test]
    fn setting_nothing_is_refused_rather_than_leaving_the_old_value() {
        let host = TestHost::new();
        let vars = crate::spec::Vars::default();
        let err = run(
            &set_spec(&crate::nodes::services::Services::none()),
            json!({ "variable": "count" }),
            PortValues::new(),
            &host,
            &vars,
        )
        .unwrap_err();
        assert!(
            err.message.contains("count"),
            "the message must name the variable: {}",
            err.message
        );
    }

    #[test]
    fn the_port_is_named_after_the_variable() {
        let ports = named_port(&json!({ "variable": "threshold" }));
        assert_eq!(ports.len(), 1);
        assert_eq!(
            ports[0].name.as_str(),
            "threshold",
            "a canvas that says `threshold` reads; one that says `value` has to be opened"
        );
    }

    #[test]
    fn a_reader_re_reads_rather_than_freezing_on_the_first_pass() {
        let s: NodeSpec<TestHost> = get_spec(&crate::nodes::services::Services::none());
        assert_eq!(
            s.purity,
            Purity::PURE_SOURCE,
            "a variable set inside a loop must be visible to a reader later in that loop"
        );
    }
}
