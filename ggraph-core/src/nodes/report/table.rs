// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_table` — rows under named columns.
//!
//! Rows arrive as a list of lists or a list of maps, and both are accepted because both are what a
//! graph naturally produces: a `for_each` collecting values gives lists, a search giving structured
//! hits gives maps. Refusing one would push the caller into a reshaping node that exists only to
//! satisfy this one.

use super::{to_value, BLOCK, ROWS};
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

/// `rows`, not a list. The editor refuses anything else while the wire is being drawn, which is
/// the whole reason the report set has its own types.
static IN: [Port; 1] = [Port::req("rows", ROWS)];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

fn columns(cfg: &Json) -> Vec<String> {
    cfg.get("columns")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A value as it will be printed.
///
/// Choosing a representation IS the renderer's job — a number has to become text somewhere, and
/// full float precision is what a float prints, not what a person reads: a relevance score arrived
/// here as `0.03278688524590164`.
///
/// This is not the same decision as `round`, which changes a value. A flow that needs the value
/// itself rounded — because it will be compared, summed or exported — rounds it upstream.
fn cell(v: &Value) -> String {
    match v {
        // Six significant decimals, trailing zeros dropped. Enough for anything a report shows,
        // and short enough to read in a column.
        Value::Num(_) => match v.as_f64() {
            Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.0}"),
            Some(n) => {
                let s = format!("{n:.6}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            None => v.summary(),
        },
        other => other.as_text().unwrap_or_else(|| other.summary()),
    }
}

struct Table;

impl<H: Host> NodeRun<H> for Table {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        // Already reduced to cells by `report_rows`, which kept their types. This is the last
        // moment a value becomes text, and it does so plainly: a number that should read as
        // currency or to two decimals was rounded by a node upstream, where somebody could see it.
        let rows: Vec<Vec<String>> = match cx.input("rows") {
            Some(Value::List(rows)) => rows
                .iter()
                .map(|r| match r {
                    Value::List(cells) => cells.iter().map(cell).collect(),
                    one => vec![cell(one)],
                })
                .collect(),
            _ => Vec::new(),
        };

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::Table {
                columns: columns(cx.config),
                rows,
            }),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        let n = match cx.input("rows") {
            Some(Value::List(r)) => r.len(),
            Some(_) => 1,
            None => 0,
        };
        format!("{n} row(s)")
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_table", "Table", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "columns": [] }))
        .with_timeout(Timeout::Inline)
        .running(Table)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full precision is what a float prints and nobody reads.
    #[test]
    fn a_number_is_written_for_a_reader() {
        assert_eq!(cell(&Value::float(0.03278688524590164)), "0.032787");
        assert_eq!(cell(&Value::float(2.50)), "2.5", "trailing zeros dropped");
    }

    /// A page number is not a measurement.
    #[test]
    fn a_whole_number_has_no_decimal_point() {
        assert_eq!(cell(&Value::int(14)), "14");
        assert_eq!(cell(&Value::float(3.0)), "3");
    }

    /// Columns are the author's, and they name what `report_rows` already extracted. Two lists that
    /// must agree — and the node that produces the rows is the one that decided their order.
    #[test]
    fn the_columns_are_the_authors() {
        assert_eq!(
            columns(&json!({ "columns": ["Documento", "Página"] })),
            vec!["Documento", "Página"]
        );
    }
}
