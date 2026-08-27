// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_rows` — where results become a table.
//!
//! The boundary between the world's data and report data, and it is a node on the canvas rather
//! than a conversion hidden inside the table. Someone reading the graph sees where a list of
//! whatever became rows of something, and can change which fields without touching the table.

use super::ROWS;
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("items", PortType::LIST)];
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
pub(super) fn read(item: &Value, fields: &[String]) -> Vec<String> {
    match item {
        Value::Json(j) if j.is_object() => fields
            .iter()
            .map(|f| match j.get(f) {
                Some(Json::String(s)) => s.clone(),
                // Absent is an empty cell, never a missing one: a ragged row misaligns every row
                // after it.
                Some(other) => other.to_string(),
                None => String::new(),
            })
            .collect(),
        Value::Map(pairs) => fields
            .iter()
            .map(|f| {
                pairs
                    .iter()
                    .find(|(k, _)| k == f)
                    .map(|(_, v)| v.as_text().unwrap_or_else(|| v.summary()))
                    .unwrap_or_default()
            })
            .collect(),
        // A list is already in order — nothing to look up.
        Value::List(cells) => cells
            .iter()
            .map(|c| c.as_text().unwrap_or_else(|| c.summary()))
            .collect(),
        one => vec![one.as_text().unwrap_or_else(|| one.summary())],
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

        let rows: Vec<Value> = match cx.input("items") {
            Some(Value::List(items)) => items
                .iter()
                .map(|i| Value::List(read(i, &f).into_iter().map(Value::text).collect()))
                .collect(),
            Some(one) => vec![Value::List(
                read(one, &f).into_iter().map(Value::text).collect(),
            )],
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
        assert_eq!(read(&item, &f), vec!["4", "a.pdf"]);
    }

    /// A ragged row would misalign every row after it.
    #[test]
    fn an_absent_field_is_an_empty_cell() {
        let f = vec!["a".to_string(), "b".to_string()];
        assert_eq!(read(&Value::Json(json!({ "a": 1 })), &f), vec!["1", ""]);
    }
}
