// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What a table is, on a wire.
//!
//! ```text
//! table        { columns: [TableColumn], rows: [TableRow] }
//! table_rows   [TableRow]
//! table_row    { "document": "a.pdf", "score": 0.03 }
//! table_column { name: "score", type: "num" }
//! table_cell   one cell's value
//! ```
//!
//! # The columns travel with the rows
//!
//! They are also on the port declaration — that is what an editor reads while somebody is drawing,
//! before anything has run. But a value that carried only its rows made every consumer rediscover
//! the columns from the first row, which is an order that happens to hold rather than one anybody
//! promised, and which says nothing at all about an empty table.
//!
//! So the value carries both. The port says what WILL arrive; the value says what DID.
//!
//! # One place
//!
//! Every producer builds a table through [`make`] and every consumer opens one through [`rows`] and
//! [`columns`]. A shape assembled by hand in seven files is a shape that disagrees with itself in
//! one of them, and the disagreement shows up as an empty report rather than as an error.

use crate::id::PortName;
use crate::port::{Column, PortType};
use crate::value::Value;

/// A table value from its columns and its rows.
pub fn make(columns: &[Column], rows: Vec<Value>) -> Value {
    Value::Map(vec![
        (
            "columns".to_string(),
            Value::List(columns.iter().map(column_value).collect()),
        ),
        ("rows".to_string(), Value::List(rows)),
    ])
}

/// One column, as a value.
pub fn column_value(c: &Column) -> Value {
    Value::Map(vec![
        ("name".to_string(), Value::text(c.name.as_str())),
        ("type".to_string(), Value::text(c.ty.as_str())),
    ])
}

/// The rows of a table value.
///
/// A bare list is read as the rows it holds. Stored graphs and hand-written test fixtures predate
/// the columns travelling alongside, and refusing them would turn a shape change into a data loss.
pub fn rows(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Map(pairs)) => match pairs.iter().find(|(k, _)| k == "rows") {
            Some((_, Value::List(rows))) => rows.clone(),
            _ => Vec::new(),
        },
        Some(Value::List(rows)) => rows.clone(),
        _ => Vec::new(),
    }
}

/// The columns of a table value, in the author's order.
///
/// Falls back to the keys of the first row for a value that carries no columns — the same reason
/// [`rows`] accepts a bare list.
pub fn columns(v: Option<&Value>) -> Vec<Column> {
    if let Some(Value::Map(pairs)) = v {
        if let Some((_, Value::List(cols))) = pairs.iter().find(|(k, _)| k == "columns") {
            return cols.iter().filter_map(column_of).collect();
        }
    }
    match rows(v).first() {
        Some(Value::Map(cells)) => cells
            .iter()
            .map(|(k, val)| Column::new(PortName::new(k.clone()), PortType::describe_type(val)))
            .collect(),
        _ => Vec::new(),
    }
}

fn column_of(v: &Value) -> Option<Column> {
    let Value::Map(pairs) = v else { return None };
    let at = |k: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == k)
            .and_then(|(_, v)| v.as_text())
    };
    let name = at("name")?;
    let ty = at("type").unwrap_or_else(|| "text".to_string());
    Some(Column::new(PortName::new(name), PortType::new(ty)))
}

/// One named cell of a row.
pub fn cell(row: &Value, name: &str) -> Option<Value> {
    match row {
        Value::Map(cells) => cells
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<Column> {
        vec![
            Column::new(PortName::new("document"), PortType::TEXT),
            Column::new(PortName::new("score"), PortType::NUM),
        ]
    }

    fn row() -> Value {
        Value::Map(vec![
            ("document".into(), Value::text("a.pdf")),
            ("score".into(), Value::float(0.03)),
        ])
    }

    #[test]
    fn a_table_carries_its_columns_and_its_rows() {
        let t = make(&cols(), vec![row()]);
        assert_eq!(rows(Some(&t)).len(), 1);
        let c = columns(Some(&t));
        assert_eq!(c[0].name.as_str(), "document");
        assert_eq!(c[1].ty, PortType::NUM);
    }

    /// The case a value with only rows could never answer: what an EMPTY table's columns are.
    #[test]
    fn an_empty_table_still_knows_its_columns() {
        let t = make(&cols(), Vec::new());
        assert!(rows(Some(&t)).is_empty());
        assert_eq!(columns(Some(&t)).len(), 2, "the shape survives having no data");
    }

    /// A bare list is still read as rows: stored graphs predate the columns travelling alongside.
    #[test]
    fn a_bare_list_of_rows_is_read_as_a_table() {
        let t = Value::List(vec![row()]);
        assert_eq!(rows(Some(&t)).len(), 1);
        assert_eq!(columns(Some(&t)).len(), 2, "read back off the first row");
    }

    #[test]
    fn a_cell_is_read_by_name() {
        assert_eq!(cell(&row(), "score").and_then(|v| v.as_f64()), Some(0.03));
        assert!(cell(&row(), "missing").is_none());
    }
}
