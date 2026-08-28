// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

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

/// A `table`, not a list. A row read from a store is cells under named columns, and every node
/// downstream that reads one by name assumes exactly that — an assumption the port should state
/// rather than leave to hope.
static OUT: [Port; 2] = [
    Port::opt("rows", PortType::TABLE),
    Port::opt("count", PortType::NUM),
];

struct Read {
    tables: std::sync::Arc<dyn crate::nodes::services::TableStore>,
}

impl<H: Host> NodeRun<H> for Read {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        let rows = self.tables.read(&table)?;
        let mut out = PortValues::new();
        out.insert(PortName::new("count"), Value::int(rows.len() as i64));
        // The declared columns travel with the rows, so a table that came back empty still says
        // what shape it has.
        let cols = crate::nodes::table::columns(cx.config);
        let cols: Vec<crate::port::Column> = cols
            .iter()
            .map(|c| {
                crate::port::Column::new(crate::id::PortName::new(c.clone()), crate::port::PortType::TEXT)
            })
            .collect();
        let values: Vec<Value> = rows.iter().map(|r| row_value(r)).collect();
        out.insert(
            PortName::new("rows"),
            crate::table::make(&cols, values),
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

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("table_read", "Read a Table", "Tables")
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "table": "" }))
        .with_timeout(Timeout::Secs(60))
        .running(Read {
            tables: services.tables.clone(),
        })
}
