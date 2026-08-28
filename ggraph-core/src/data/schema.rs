// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The shape an author asks a source for.
//!
//! Every source has the same problem: it knows a set of fields, and the graph needs a stable table.
//! A corpus search, a SQL query, a parsed log — none of them should decide the shape of the graph
//! built on top of them, because then adding a field at the source shifts every stored graph that
//! read from it.
//!
//! So the author declares, and the source accommodates. **The schema is imperative**: a column
//! declared as text arrives as text, whatever the field happens to hold. A source that returned its
//! own preferred type would make the contract advisory, and nothing downstream could rely on it.
//!
//! # A column binds by name
//!
//! `{ "name": "score", "type": "num" }` says *give me a numeric column called score*, and every
//! source looks for the field that is called that. The alternative — carrying the source field in
//! the column — makes a schema mean something to one source and nothing to the next, which is the
//! opposite of why this lives in the engine.
//!
//! A source that has no such field leaves the column out, and a missing column is already reported
//! against the port's declaration. That is the guardian working, not a gap.
//!
//! Renaming is a separate job, for a node that renames columns of a table. A pretty heading is a
//! third thing again, and belongs to whatever draws the table.

use crate::id::PortName;
use crate::port::{Column, PortType};
use crate::value::Value;
use serde_json::Value as Json;

/// A field a source can offer, and what it naturally holds.
///
/// Published so a schema is chosen from a list rather than typed from memory, and so a model knows
/// what exists before it wires anything.
#[derive(Clone, Debug)]
pub struct Available {
    pub name: &'static str,
    pub ty: PortType,
}

/// One declared column: the field it names, and the type it must arrive as.
#[derive(Clone, Debug)]
pub struct Mapping {
    pub name: String,
    pub ty: PortType,
}

/// Reads a declared schema, keeping only the columns the source can offer.
///
/// A column naming a field that does not exist is dropped rather than fatal: the catalogue is what
/// stops that being written, and refusing a run over one typo throws away the other four columns.
pub fn parse(config: &Json, available: &[Available]) -> Vec<Mapping> {
    config
        .get("columns")
        .and_then(Json::as_array)
        .map(|cols| {
            cols.iter()
                .filter_map(|c| {
                    let name = c.get("name").and_then(Json::as_str)?.trim();
                    let natural = available.iter().find(|a| a.name == name)?.ty.clone();
                    Some(Mapping {
                        name: name.to_string(),
                        // The author's type wins; the field's is the default. A schema that says
                        // nothing still says something.
                        ty: c
                            .get("type")
                            .and_then(Json::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .map(PortType::new)
                            .unwrap_or(natural),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The columns a port should declare for this schema.
pub fn columns(mappings: &[Mapping]) -> Vec<Column> {
    mappings
        .iter()
        .map(|m| Column::new(PortName::new(m.name.clone()), m.ty.clone()))
        .collect()
}

/// A value as the declared type, or `None` when it cannot be one.
///
/// Anything becomes text — that direction never fails. The other way round can: a document name is
/// not a number, and returning a zero would be a lie that passes every check downstream.
///
/// A cell that cannot convert is left OUT of the row, and absence is already reported as a missing
/// column — which points at the real problem instead of inventing a value that hides it.
pub fn convert(v: Value, ty: &PortType) -> Option<Value> {
    match ty.as_str() {
        "text" => Some(Value::text(v.as_text().unwrap_or_else(|| v.summary()))),
        "num" => v.as_f64().map(Value::float).or_else(|| {
            v.as_text()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(Value::float)
        }),
        "bool" => v
            .as_bool()
            .or_else(|| v.as_text().and_then(|s| s.trim().parse::<bool>().ok()))
            .map(Value::Bool),
        // A type the engine does not define is a product's own, and the engine has no business
        // guessing at its shape.
        _ => Some(v),
    }
}

/// One row, built from a source's fields according to the schema.
pub fn row(mappings: &[Mapping], field: impl Fn(&str) -> Value) -> Value {
    Value::Map(
        mappings
            .iter()
            .filter_map(|m| convert(field(&m.name), &m.ty).map(|cell| (m.name.clone(), cell)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<Available> {
        vec![
            Available {
                name: "document",
                ty: PortType::TEXT,
            },
            Available {
                name: "page",
                ty: PortType::NUM,
            },
            Available {
                name: "score",
                ty: PortType::NUM,
            },
        ]
    }

    /// The whole point: what is not asked for is not there.
    #[test]
    fn the_schema_decides_the_columns() {
        let cfg = serde_json::json!({ "columns": [
            { "name": "document" },
            { "name": "score" }
        ]});
        let cols: Vec<String> = columns(&parse(&cfg, &available()))
            .iter()
            .map(|c| c.name.as_str().to_string())
            .collect();
        assert_eq!(cols, vec!["document", "score"]);
    }

    /// The author's type wins. Asking for a page as text gets text, because a source returning its
    /// own preferred type would make the contract advisory.
    #[test]
    fn a_declared_type_is_imperative() {
        let cfg = serde_json::json!({ "columns": [
            { "name": "page", "type": "text" }
        ]});
        let m = parse(&cfg, &available());
        assert_eq!(m[0].ty, PortType::TEXT);

        let r = row(&m, |_| Value::int(14));
        let Value::Map(cells) = r else { panic!() };
        assert_eq!(cells[0].1.as_text().as_deref(), Some("14"));
    }

    /// With no type declared, the field's own is used — so a schema built by clicking names still
    /// produces a numeric column where the source has a number.
    #[test]
    fn an_undeclared_type_falls_back_to_the_field() {
        let cfg = serde_json::json!({ "columns": [{ "name": "score" }] });
        assert_eq!(parse(&cfg, &available())[0].ty, PortType::NUM);
    }

    /// A zero would be a lie that passes every check. Absence is already reported as a missing
    /// column, which points at the real problem.
    #[test]
    fn a_cell_that_cannot_convert_is_left_out() {
        let cfg = serde_json::json!({ "columns": [{ "name": "score", "type": "num" }] });
        let m = parse(&cfg, &available());
        let Value::Map(cells) = row(&m, |_| Value::text("a.pdf")) else {
            panic!()
        };
        assert!(cells.is_empty(), "no cell rather than a fabricated one");
    }

    /// A schema tightening a loose field is the useful case: a log column that holds text but is
    /// really a number.
    #[test]
    fn text_holding_a_number_converts() {
        assert_eq!(
            convert(Value::text("2.5"), &PortType::NUM).and_then(|v| v.as_f64()),
            Some(2.5)
        );
    }

    /// A field the source does not offer is dropped, not fatal: one typo must not throw away the
    /// other columns.
    #[test]
    fn an_unknown_field_is_dropped() {
        let cfg = serde_json::json!({ "columns": [
            { "name": "document" },
            { "name": "invented" }
        ]});
        assert_eq!(parse(&cfg, &available()).len(), 1);
    }
}
