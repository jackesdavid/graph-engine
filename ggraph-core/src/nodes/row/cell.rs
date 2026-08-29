// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `cell` — one named cell of one row.
//!
//! The node where a table stops being a table and becomes a value a graph can act on.
//!
//! # The output port is typed by the column
//!
//! Reading a cell by name could return anything, and an `any` output is one the editor cannot
//! check — every wire from it would be drawable and half of them wrong. So the schema comes in as
//! well, its columns are written onto this node, and the output port carries the type THAT column
//! was declared as: pick `score` and the output is a number, pick `document` and it is text.
//!
//! Before a column is chosen there is no output pin at all. A pin whose type is a guess is worse
//! than a pin that is not there yet: the guess is what the editor would let somebody wire.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 2] = [
    // Wired, not typed: a row is a value another node produces, and no box holds one. Without
    // saying so, this reads as a kind a chain could BEGIN at.
    Port::req("row", PortType::TABLE_ROW).wired(),
    // A shape dependency: connecting a schema writes its columns onto this node, which is what
    // lets the column be picked from a list and the output be typed. `Ports::dynamic` never sees
    // an edge, so the columns have to be here rather than followed back up the wire.
    Port::opt("schema", PortType::SCHEMA),
];

/// The chosen column's name, and the type it was declared as.
fn chosen(cfg: &Json) -> Option<(String, PortType)> {
    let name = cfg
        .get("column")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let ty = crate::nodes::schema::declared(cfg)
        .into_iter()
        .find(|c| c.name.as_str() == name)?
        .ty;
    Some((name.to_string(), ty))
}

fn ports(cfg: &Json) -> Vec<Port> {
    match chosen(cfg) {
        Some((_, ty)) => vec![Port::opt("value", ty)],
        None => Vec::new(),
    }
}

fn fields(cfg: &Json) -> Vec<Field> {
    let names: Vec<String> = crate::nodes::schema::declared(cfg)
        .iter()
        .map(|c| c.name.as_str().to_string())
        .collect();
    // Required: the column IS this node's input. With none chosen there is nothing to take.
    vec![Field::choice("column", "Column", names).required()]
}

struct Cell;

impl<H: Host> NodeRun<H> for Cell {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let (name, ty) = chosen(cx.config).ok_or_else(|| {
            NodeError::new("no column — connect a schema and pick which cell to read")
        })?;

        let Some(Value::Map(cells)) = cx.input("row") else {
            return Err(NodeError::new("no row to read"));
        };

        let mut out = PortValues::new();
        // A cell the row does not carry produces NOTHING, not a zero or a blank: "this row has no
        // such cell" and "this cell is empty" are different facts, and only one of them is a bug.
        if let Some((_, v)) = cells.iter().find(|(k, _)| *k == name) {
            // Converted to the declared type, because that is what the port promised. The schema
            // that named the column is the same one that typed it.
            if let Some(v) = crate::schema::convert(v.clone(), &ty) {
                out.insert(PortName::new("value"), v);
            }
        }
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("value")) {
            Some(v) => v.summary(),
            None => "no cell".into(),
        }
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("cell", "Cell value", "Data")
        .about(
            r#"Takes ONE named column out of a row.

The column is chosen in the inspector, from the schema you wire in — so the port comes out with
that column's type: a `num` column gives a number, not text somebody has to convert.

```
Read a Table --table--> First row --row--> Cell value (column: price) --> Round
```"#,
        )
        // The columns the wire brought. `Ports::dynamic` cannot see edges, so the schema is
        // copied in and the ports resolve from config as they always did — the same copy the
        // editor made on the drop of a wire, made here so anything assembling a graph gets it.
        .baking(|cfg, wired| {
            let cols = &wired.on("schema")?.columns;
            let mut next = cfg.clone();
            next.as_object_mut()?.insert(
                "columns".into(),
                json!(cols
                    .iter()
                    .map(|c| json!({ "name": c.name.as_str(), "type": c.ty.as_str() }))
                    .collect::<Vec<_>>()),
            );
            Some(next)
        })
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::dynamic(ports))
        .with_fields(Fields::dynamic(fields))
        .with_config(|| json!({ "column": "", "columns": [] }))
        .with_timeout(Timeout::Inline)
        .running(Cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(column: &str) -> Json {
        json!({
            "column": column,
            "columns": [
                { "name": "document", "type": "text" },
                { "name": "score", "type": "num" }
            ]
        })
    }

    /// The whole point: the pin's type follows the column, so a wrong wire cannot be drawn.
    #[test]
    fn the_output_takes_the_columns_type() {
        assert_eq!(ports(&cfg("score"))[0].ty, PortType::NUM);
        assert_eq!(ports(&cfg("document"))[0].ty, PortType::TEXT);
    }

    /// A pin whose type is a guess is worse than one that is not there yet.
    #[test]
    fn there_is_no_pin_until_a_column_is_chosen() {
        assert!(ports(&cfg("")).is_empty());
        assert!(
            ports(&json!({ "column": "score" })).is_empty(),
            "no schema, no type"
        );
    }

    /// The names come from the schema, so a column is picked rather than remembered.
    #[test]
    fn the_panel_offers_the_schemas_columns() {
        let f = fields(&cfg(""));
        let crate::spec::FieldKind::Choice(opts) = &f[0].kind else {
            panic!("a column is chosen from a list")
        };
        assert_eq!(opts, &vec!["document".to_string(), "score".to_string()]);
    }
}
