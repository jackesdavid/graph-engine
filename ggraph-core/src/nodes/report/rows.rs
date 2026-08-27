// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_rows` — records become table rows.
//!
//! Takes `records`, not a bare list: reading a field by name is an assumption about the shape of
//! what arrived, and an assumption the port does not state is one the editor cannot check.
//!
//! The boundary between the world's data and report data, and it is a node on the canvas rather
//! than a conversion hidden inside the table. Someone reading the graph sees where results became
//! rows, and can change which fields without touching the table.

use super::ROWS;
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("records", PortType::RECORDS)];
static OUT: [Port; 1] = [Port::opt("rows", ROWS)];

pub(super) fn fields(cfg: &Json) -> Vec<String> {
    cfg.get("fields")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// One item reduced to the declared fields, in the order they were declared.
///
/// The author's order wins over whatever order the producer used: a table is read by a person, and
/// the person who chose the columns chose their order too.
/// A JSON value as one of the engine's own.
///
/// Not wrapped in `Value::Json`: a wrapped number cannot be read by `as_f64`, so a rounding node
/// downstream would see nothing to round and a table would print the word "json". Converting here
/// is what keeps the cells usable by every node after this one.
fn native(j: &Json) -> Value {
    match j {
        Json::String(s) => Value::text(s.clone()),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::int(i),
            None => n
                .as_f64()
                .map(Value::float)
                .unwrap_or_else(|| Value::text("")),
        },
        Json::Null => Value::text(""),
        // Nested shapes keep their JSON: a cell holding an object is unusual, and flattening it
        // would invent a rendering nobody asked for.
        other => Value::Json(other.clone()),
    }
}

/// One item reduced to the declared fields, **keeping their types**.
///
/// A number stays a number here. How it should read — two decimals, a currency prefix, a percent
/// sign — is a presentation decision, and presentation decisions belong to a node in the flow where
/// somebody can see and change them, not to a config field on the extractor.
pub(super) fn read(item: &Value, fields: &[String]) -> Vec<Value> {
    match item {
        Value::Json(j) if j.is_object() => fields
            .iter()
            .map(|f| match j.get(f) {
                Some(v) => native(v),
                // Absent is an empty cell, never a missing one: a ragged row misaligns every row
                // after it.
                None => Value::text(""),
            })
            .collect(),
        Value::Map(pairs) => fields
            .iter()
            .map(|f| {
                pairs
                    .iter()
                    .find(|(k, _)| k == f)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| Value::text(""))
            })
            .collect(),
        // A list is already in order — nothing to look up.
        Value::List(cells) => cells.clone(),
        one => vec![one.clone()],
    }
}

struct Rows;

impl<H: Host> NodeRun<H> for Rows {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let f = fields(cx.config);
        if f.is_empty() {
            return Err(NodeError::new(
                "no fields declared — a table needs to know which columns to read",
            ));
        }

        let rows: Vec<Value> = match cx.input("records") {
            Some(Value::List(items)) => items.iter().map(|i| Value::List(read(i, &f))).collect(),
            Some(one) => vec![Value::List(read(one, &f))],
            None => Vec::new(),
        };

        let mut out = PortValues::new();
        out.insert(PortName::new("rows"), Value::List(rows));
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("rows")) {
            Some(Value::List(r)) => format!("{} row(s)", r.len()),
            _ => "0 rows".into(),
        }
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_rows", "Rows from data", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "fields": [] }))
        .with_timeout(Timeout::Inline)
        .running(Rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared order wins over the producer's field order.
    #[test]
    fn fields_are_read_in_the_declared_order() {
        let f = vec!["page".to_string(), "document".to_string()];
        let item = Value::Json(json!({ "document": "a.pdf", "page": 4, "score": 0.9 }));
        let cells: Vec<String> = read(&item, &f)
            .iter()
            .map(|c| c.as_text().unwrap_or_else(|| c.summary()))
            .collect();
        assert_eq!(cells, vec!["4", "a.pdf"]);
    }

    /// A ragged row would misalign every row after it.
    #[test]
    fn an_absent_field_is_an_empty_cell() {
        let f = vec!["a".to_string(), "b".to_string()];
        let cells = read(&Value::Json(json!({ "a": 1 })), &f);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1].as_text().unwrap_or_default(), "");
    }

    /// A number stays a number. How it reads is decided later, by a node somebody can see.
    #[test]
    fn a_number_keeps_its_type() {
        let f = vec!["score".to_string()];
        let cells = read(&Value::Json(json!({ "score": 0.0327 })), &f);
        assert_eq!(cells[0].as_f64(), Some(0.0327));
    }
}
