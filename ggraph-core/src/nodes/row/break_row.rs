// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `break_row` — one row, opened into a port per column.
//!
//! [`cell`](super::cell) takes one column at a time, which is right when a graph wants one value
//! and wrong when it wants the row: four fields meant four nodes and four wires that all say the
//! same thing about where they came from.
//!
//! Same shape as breaking a detection open in a product that has detections — one input, no
//! configuration to speak of. The difference is that this one has no fixed shape: the ports come
//! from the schema, so it opens a row of any table from any source, and each port carries the type
//! its column was declared as.
//!
//! Before a schema is connected there are no ports. A pin whose type is a guess is worse than a pin
//! that is not there yet — the guess is what the editor would let somebody wire.

use crate::host::Host;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 2] = [
    Port::req("row", PortType::TABLE_ROW),
    // A shape dependency: connecting a schema writes its columns onto this node, which is what the
    // ports are built from. `Ports::dynamic` never sees an edge.
    Port::opt("schema", PortType::SCHEMA),
];

fn ports(cfg: &Json) -> Vec<Port> {
    crate::nodes::schema::declared(cfg)
        .into_iter()
        .map(|c| Port::new(c.name, c.ty, false))
        .collect()
}

struct BreakRow;

impl<H: Host> NodeRun<H> for BreakRow {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let columns = crate::nodes::schema::declared(cx.config);
        if columns.is_empty() {
            return Err(NodeError::new(
                "no schema — connect one so this node knows what the row holds",
            ));
        }

        let Some(row @ Value::Map(_)) = cx.input("row") else {
            return Err(NodeError::new("no row to open"));
        };

        let mut out = PortValues::new();
        for c in &columns {
            // A column the row does not carry produces NOTHING, not a blank: "this row has no such
            // cell" and "this cell is empty" are different facts, and only one of them is a defect.
            let Some(v) = crate::table::cell(row, c.name.as_str()) else {
                continue;
            };
            // Converted to the declared type, because that is what the port promised.
            if let Some(v) = crate::schema::convert(v, &c.ty) {
                out.insert(c.name.clone(), v);
            }
        }
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        format!(
            "{}/{} field(s)",
            out.len(),
            crate::nodes::schema::declared(cx.config).len()
        )
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("break_row", "Break row", "Data")
        .about(r#"Opens a row into one port per column.

The same job as **Cell value**, done once instead of once per column. The ports come from the schema
you wire in, each with its column's type.

```
First row --row--> Break row --model--> Format
                              --price--> Round
```"#)
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::dynamic(ports))
        .with_config(|| json!({ "columns": [] }))
        .with_timeout(Timeout::Inline)
        .running(BreakRow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Json {
        json!({ "columns": [
            { "name": "document", "type": "text" },
            { "name": "score", "type": "num" }
        ]})
    }

    /// The ports come from the schema, and each carries its column's type.
    #[test]
    fn a_port_per_column_typed_as_the_column() {
        let p: Vec<(String, String)> = ports(&cfg())
            .iter()
            .map(|p| (p.name.as_str().to_string(), p.ty.as_str().to_string()))
            .collect();
        assert_eq!(
            p,
            vec![
                ("document".to_string(), "text".to_string()),
                ("score".to_string(), "num".to_string())
            ]
        );
    }

    /// A pin whose type is a guess is worse than one that is not there yet.
    #[test]
    fn no_schema_no_ports() {
        assert!(ports(&json!({})).is_empty());
    }
}
