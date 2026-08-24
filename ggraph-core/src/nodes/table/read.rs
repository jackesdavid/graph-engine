//! `table_read` — the whole table, as a list of rows.
//!
//! Pairs with `for_each`: read a table, loop over it, do something per row. That is the shape
//! most workflows built on tables actually take.

use super::{row_value, table_name};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static OUT: [Port; 2] = [
    Port::opt("rows", PortType::LIST),
    Port::opt("count", PortType::NUM),
];

struct Read;

impl<H: Host> NodeRun<H> for Read {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError("no table name".into()))?;
        let rows = cx.host.tables().read(&table)?;
        let mut out = PortValues::new();
        out.insert(PortName::new("count"), Value::int(rows.len() as i64));
        out.insert(
            PortName::new("rows"),
            Value::List(rows.iter().map(|r| row_value(r)).collect()),
        );
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("count")).and_then(Value::as_i64) {
            Some(n) => format!("{n} row(s)"),
            None => String::new(),
        }
    }
}

pub fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("table_read", "Read a Table", "Tables")
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "table": "" }))
        .with_timeout(Timeout::Secs(60))
        .running(Read)
}
