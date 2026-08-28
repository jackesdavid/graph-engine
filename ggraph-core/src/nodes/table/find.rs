// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `table_find` — the first row where a column equals a value.
//!
//! Three arms, for the reason that recurs across this node set: `found`, `missing`, and the
//! distinction between them is the point. A graph that treats "no such row" as an error stops;
//! a graph that treats it as an empty row carries on with blanks. Neither is what the author
//! meant, and both are what happens when the node has one output and no way to say which.

use super::table_name;
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeCx, NodeError, NodeRoute, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 1] = [Port::opt("value", PortType::SCALAR)];
static OUT: [Port; 2] = [
    Port::opt("row", PortType::TABLE_ROW),
    Port::opt("index", PortType::NUM),
];
static ARMS: [Port; 2] = [
    Port::opt("found", PortType::EXEC),
    Port::opt("missing", PortType::EXEC),
];

struct Find {
    tables: std::sync::Arc<dyn crate::nodes::services::TableStore>,
}

impl<H: Host> NodeRun<H> for Find {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        let column = cx
            .cfg_str("column")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(NodeError::new("no column to match on"))?;
        let Some(wanted) = cx.input_or_cfg("value") else {
            // Matching against nothing would return the first row with an empty cell, which is
            // a plausible-looking wrong answer.
            return Err(NodeError::new("nothing to match against"));
        };

        let rows = self.tables.read(&table)?;
        let hit = rows.iter().enumerate().find(|(_, row)| {
            row.iter()
                .any(|(c, v)| c == column && v.as_text() == wanted.as_text())
        });

        let mut out = PortValues::new();
        if let Some((i, row)) = hit {
            out.insert(PortName::new("index"), Value::int(i as i64));
            out.insert(PortName::new("row"), Value::Map(row.clone()));
        }
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("index")).and_then(Value::as_i64) {
            Some(i) => format!("row {i}"),
            None => "no match".into(),
        }
    }
}

impl<H: Host> NodeRoute<H> for Find {
    fn arms(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> Vec<PortName> {
        let arm = if out.contains_key(&PortName::new("row")) {
            "found"
        } else {
            "missing"
        };
        vec![PortName::new(arm)]
    }
}

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("table_find", "Find a Row", "Tables")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_exec_out(ExecOut::Static(&ARMS))
        .with_config(|| json!({ "table": "", "column": "", "value": "" }))
        .with_timeout(Timeout::Secs(60))
        .routing(Find {
            tables: services.tables.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn a_miss_takes_its_own_arm_rather_than_failing_or_returning_blanks() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Route(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "table": "t", "column": "id", "value": "7" });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let arms: Vec<&str> = NodeRoute::arms(r.as_ref(), &cx, &PortValues::new())
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .iter()
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str)
            .collect();
        assert_eq!(arms, vec!["missing"]);
    }

    #[test]
    fn matching_against_nothing_is_refused() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Route(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "table": "t", "column": "id", "value": "" });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        assert!(
            r.run(&cx).is_err(),
            "it would otherwise return the first row with an empty cell, which looks like a hit"
        );
    }
}
