// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `table_schema` — the shape a source is asked to produce.
//!
//! Columns and their types, and no rows. It is the one place an author writes down what a graph
//! expects to receive, and it is a node rather than a field on each source for one reason: the same
//! shape can be demanded of two sources at once, and stay the same when it is edited. A shape
//! written twice by hand agrees until the day somebody changes one of them.
//!
//! The wire it produces is a **shape dependency**. A consumer resolves its ports from configuration
//! alone — [`Ports::dynamic`] never sees an edge — so connecting this node copies its columns into
//! the consumer, and the value on the wire is what a run log shows rather than what a consumer
//! reads. That is the same mechanic the editor already uses for a dictionary row.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Column, Port, PortType};
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

/// The types a column may be declared as: the ones a source is able to convert into.
pub const COLUMN_TYPES: [&str; 3] = ["text", "num", "bool"];

/// The columns declared in config, in the author's order.
///
/// Blank names are skipped — a column nobody can name is one nothing can read — and so are
/// duplicates, because a row is a map and the second would silently replace the first.
pub fn declared(cfg: &Json) -> Vec<Column> {
    let mut seen: Vec<String> = Vec::new();
    cfg.get("columns")
        .and_then(Json::as_array)
        .map(|cols| {
            cols.iter()
                .filter_map(|c| {
                    let name = c.get("name").and_then(Json::as_str)?.trim();
                    if name.is_empty() || seen.iter().any(|s| s == name) {
                        return None;
                    }
                    seen.push(name.to_string());
                    let ty = c
                        .get("type")
                        .and_then(Json::as_str)
                        .filter(|s| COLUMN_TYPES.contains(s))
                        .unwrap_or("text");
                    Some(Column::new(PortName::new(name), PortType::new(ty)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The value a schema travels as: the column list itself.
///
/// Rows of named pairs, like every other table in the engine, so a run log renders it without a
/// special case and a person reading one sees the shape rather than an opaque handle.
pub fn to_value(columns: &[Column]) -> Value {
    Value::List(
        columns
            .iter()
            .map(|c| {
                Value::Map(vec![
                    ("name".into(), Value::text(c.name.as_str())),
                    ("type".into(), Value::text(c.ty.as_str())),
                ])
            })
            .collect(),
    )
}

fn ports(cfg: &Json) -> Vec<Port> {
    vec![Port::opt("schema", PortType::SCHEMA).with_columns(declared(cfg))]
}

struct TableSchema;

impl<H: Host> NodeRun<H> for TableSchema {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        out.insert(
            PortName::new("schema"),
            to_value(&declared(cx.config)),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        let n = declared(cx.config).len();
        format!("{n} column(s)")
    }
}

pub fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("table_schema", "Schema", "Data")
        .with_inputs(Ports::NONE)
        .with_outputs(Ports::dynamic(ports))
        .with_config(|| json!({ "columns": [] }))
        // Required: a schema with no columns declares nothing, and every consumer of it — the
        // ports it bakes, the conversion it guards — then has nothing to work from.
        .with_fields(Fields::List(vec![Field::rows(
            "columns",
            "Columns",
            vec![
                Field::text("name", "Name"),
                Field::choice("type", "Type", COLUMN_TYPES),
            ],
        )
        .required()]))
        .with_timeout(Timeout::Inline)
        .running(TableSchema)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config drives the port's columns, which is what lets everything downstream see the
    /// shape while it is being drawn.
    #[test]
    fn the_port_carries_the_declared_columns() {
        let p = ports(&json!({ "columns": [
            { "name": "document", "type": "text" },
            { "name": "score", "type": "num" }
        ]}));
        let cols: Vec<(String, String)> = p[0]
            .columns
            .iter()
            .map(|c| (c.name.as_str().to_string(), c.ty.as_str().to_string()))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("document".to_string(), "text".to_string()),
                ("score".to_string(), "num".to_string())
            ]
        );
    }

    /// A row is a map: two columns of the same name means the second silently replaces the first.
    #[test]
    fn a_repeated_name_is_dropped() {
        assert_eq!(
            declared(&json!({ "columns": [{ "name": "a" }, { "name": "a" }] })).len(),
            1
        );
    }

    /// A column nobody can name is one nothing can read.
    #[test]
    fn a_blank_name_is_dropped() {
        assert!(declared(&json!({ "columns": [{ "name": "  " }] })).is_empty());
    }

    /// A type outside the set falls back to text rather than declaring something no source can
    /// produce.
    #[test]
    fn an_unknown_type_becomes_text() {
        let c = declared(&json!({ "columns": [{ "name": "a", "type": "colour" }] }));
        assert_eq!(c[0].ty, PortType::TEXT);
    }
}
