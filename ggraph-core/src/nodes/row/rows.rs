// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `get_table_rows` — a table's rows, on their own.
//!
//! A table is its columns AND its rows. This drops the columns and hands over the rows, which is
//! what a loop walks and what a count counts. Its own type rather than the same one: a list of rows
//! cannot answer what the shape is, and a port that claimed to be a table while unable to say its
//! columns is a port that lies to whatever reads it next.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 1] = [Port::opt("rows", PortType::TABLE_ROWS)];

struct GetTableRows;

impl<H: Host> NodeRun<H> for GetTableRows {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        out.insert(
            PortName::new("rows"),
            Value::List(crate::table::rows(cx.input("table"))),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        format!("{} row(s)", crate::table::rows(cx.input("table")).len())
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("get_table_rows", "Table rows", "Data")
        .about(r#"Turns a table into a **list** of rows, which is what a loop can take.

**For Each** never takes a table. This is the node between them.

```
Read a Table --table--> Table rows --rows--> For Each --item--> Break row
```"#)
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({}))
        .with_timeout(Timeout::Inline)
        .running(GetTableRows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Column;

    #[test]
    fn the_rows_come_out_without_the_columns() {
        let row = Value::Map(vec![("document".into(), Value::text("a.pdf"))]);
        let cols = [Column::new(PortName::new("document"), PortType::TEXT)];
        let t = crate::table::make(&cols, vec![row]);
        assert_eq!(crate::table::rows(Some(&t)).len(), 1);
    }
}
