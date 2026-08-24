//! `table_append` — add a row.

use super::{columns, table_name};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

/// One input per declared column, so the canvas shows what is being written.
fn ports(cfg: &Json) -> Vec<Port> {
    columns(cfg)
        .into_iter()
        .map(|c| Port::new(PortName::new(c), PortType::ANY, false))
        .collect()
}

struct Append;

impl<H: Host> NodeRun<H> for Append {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        let cols = columns(cx.config);
        if cols.is_empty() {
            return Err(NodeError::new("this table has no columns"));
        }
        // A column with nothing wired is written as an empty cell rather than skipped. A row
        // missing a column is a row that lines up with nothing when somebody opens the table.
        let row: Vec<(String, Value)> = cols
            .into_iter()
            .map(|c| {
                let v = cx.input(&c).cloned().unwrap_or(Value::Text(String::new()));
                (c, v)
            })
            .collect();
        cx.host.tables().append(&table, &row)?;
        Ok(PortValues::new())
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        table_name(cx.config).unwrap_or_default()
    }
}

pub fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("table_append", "Add a Row", "Tables")
        .with_inputs(Ports::Dynamic(ports))
        .with_config(|| json!({ "table": "", "columns": [] }))
        .with_timeout(Timeout::Secs(30))
        .running(Append)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn a_column_with_nothing_wired_is_written_empty_not_skipped() {
        let cols = columns(&json!({ "columns": ["a", "b"] }));
        assert_eq!(cols, vec!["a", "b"]);
        // The row built from an input map missing `b` still has two cells; without that, the
        // row lines up with nothing when somebody opens the table.
        let mut inputs = PortValues::new();
        inputs.insert(PortName::new("a"), Value::int(1));
        let built: Vec<String> = cols
            .iter()
            .map(|c| {
                inputs
                    .get(&PortName::new(c.clone()))
                    .map(Value::summary)
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(built, vec!["1", ""]);
    }

    #[test]
    fn a_table_with_no_columns_is_refused() {
        let s: NodeSpec<TestHost> = spec();
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "table": "findings", "columns": [] });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
        };
        assert!(r.run(&cx).unwrap_err().message.contains("columns"));
    }

    #[test]
    fn ports_come_from_the_columns() {
        let names: Vec<String> = ports(&json!({ "columns": ["cnpj", "valor"] }))
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(names, vec!["cnpj", "valor"]);
    }
}
