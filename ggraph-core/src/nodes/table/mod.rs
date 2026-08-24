//! Named tables — the only thing in the standard set that outlives the run that wrote it.
//!
//! A variable is working state and dies with the run. A table is a *result*: the four hundred
//! rows a workflow produced, which somebody will open tomorrow, and which the next workflow
//! reads as its input. Making that distinction visible in the palette is worth more than the
//! convenience of one node that does both.
//!
//! ## Rows are ordered pairs, not maps
//!
//! A row is `Vec<(String, Value)>` because column order is meaningful. A table built by a graph
//! is read by a person, and a person reading `valor, cnpj, data` in a different order every time
//! stops trusting it.
//!
//! ## What is deliberately missing
//!
//! No query language, no joins, no schema migration. A graph that needs those wants a database,
//! and the product it is embedded in has one. These nodes are the small set that makes a table
//! usable *from a canvas*: put a row in, read it back, find one, change a cell, empty it.

use crate::value::Value;

pub mod append;
pub mod clear;
pub mod count;
pub mod find;
pub mod read;
pub mod set_cell;

/// The column names a node declares, in order.
pub(crate) fn columns(cfg: &serde_json::Value) -> Vec<String> {
    cfg.get("columns")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The table this node works on.
pub(crate) fn table_name(cfg: &serde_json::Value) -> Option<String> {
    cfg.get("table")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A row, as it travels on a wire.
pub(crate) fn row_value(row: &[(String, Value)]) -> Value {
    Value::Map(row.to_vec())
}
