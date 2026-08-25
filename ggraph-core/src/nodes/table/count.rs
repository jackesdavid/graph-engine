//! `table_count` — how many rows.
//!
//! Separate from `table_read` because a graph that only wants the number should not pull four
//! hundred rows across a wire to count them.

use super::table_name;
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static OUT: [Port; 1] = [Port::opt("count", PortType::NUM)];

struct Count {
    tables: std::sync::Arc<dyn crate::nodes::services::TableStore>,
}

impl<H: Host> NodeRun<H> for Count {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        let n = self.tables.row_count(&table)?;
        let mut out = PortValues::new();
        out.insert(PortName::new("count"), Value::int(n as i64));
        Ok(out)
    }
}

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::pure("table_count", "Count Rows", "Tables")
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "table": "" }))
        .with_timeout(Timeout::Secs(30))
        .running(Count {
            tables: services.tables.clone(),
        })
}
