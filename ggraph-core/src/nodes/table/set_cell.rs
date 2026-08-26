// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `table_set_cell` — change one cell of one row.

use super::table_name;
use crate::host::Host;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 2] = [
    Port::req("row", PortType::NUM),
    Port::opt("value", PortType::ANY),
];

struct SetCell {
    tables: std::sync::Arc<dyn crate::nodes::services::TableStore>,
}

impl<H: Host> NodeRun<H> for SetCell {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        let column = cx
            .cfg_str("column")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(NodeError::new("no column"))?;
        // Not defaulted to row zero. A row index that failed to arrive would otherwise
        // overwrite the first row of the table, which is the worst possible guess.
        let row = cx
            .input_or_cfg("row")
            .as_ref()
            .and_then(Value::as_i64)
            .ok_or(NodeError::new("no row index"))?;
        if row < 0 {
            return Err(NodeError::new(format!("row {row} is not a row")));
        }
        let value = cx
            .input_or_cfg("value")
            .unwrap_or(Value::Text(String::new()));
        self.tables.set_cell(&table, row as u64, column, &value)?;
        Ok(PortValues::new())
    }
}

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("table_set_cell", "Change a Cell", "Tables")
        .with_inputs(Ports::Static(&IN))
        .with_config(|| json!({ "table": "", "column": "", "row": "", "value": "" }))
        .with_timeout(Timeout::Secs(30))
        .running(SetCell {
            tables: services.tables.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn a_missing_row_index_refuses_rather_than_writing_to_row_zero() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "table": "t", "column": "status", "row": "", "value": "done" });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
        };
        let err = r.run(&cx).unwrap_err();
        assert!(
            err.message.contains("row index"),
            "defaulting to zero silently overwrites the first row: {}",
            err.message
        );
    }
}
