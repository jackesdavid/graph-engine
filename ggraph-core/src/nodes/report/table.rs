// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_table` — rows under named columns.
//!
//! Rows arrive as a list of lists or a list of maps, and both are accepted because both are what a
//! graph naturally produces: a `for_each` collecting values gives lists, a search giving structured
//! hits gives maps. Refusing one would push the caller into a reshaping node that exists only to
//! satisfy this one.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

/// A `table`, taken as it comes. The source declared its schema, so the columns are the author's
/// already — projecting them again here would be a second place to decide the same thing.
static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

/// The column names, read off the first row.
///
/// Not from config: the source already declared them, and a second declaration here is a second
/// thing to keep in step. A table with no rows has no columns to name, and renders as an empty one.


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
        let source = crate::table::rows(cx.input("table"));
        // The columns come from the table itself. Reading them back off the first row was an order
        // that happened to hold, and it had nothing to say about a table with no rows at all.
        let names: Vec<String> = crate::table::columns(cx.input("table"))
            .iter()
            .map(|c| c.name.as_str().to_string())
            .collect();

        // Read in the schema's order, not each row's. They agree when the source built them, and
        // when they do not it is the column order a person expects that should win.
        let rows: Vec<Vec<String>> = source
            .iter()
            .map(|r| match r {
                Value::Map(pairs) => names
                    .iter()
                    .map(|n| {
                        pairs
                            .iter()
                            .find(|(k, _)| k == n)
                            .map(|(_, v)| cell(v))
                            .unwrap_or_default()
                    })
                    .collect(),
                one => vec![cell(one)],
            })
            .collect();

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::Table {
                columns: names,
                rows,
            }),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        let n = match cx.input("table") {
            Some(Value::List(r)) => r.len(),
            Some(_) => 1,
            None => 0,
        };
        format!("{n} row(s)")
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_table", "ReportTable", "Report")
        .about(r#"Draws a table in a report.

Takes the table itself — the columns and their names come with it.

```
Ask --result--> ReportTable --block--> ReportRender
```"#)
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({}))
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

    /// The columns come from the table, because the source already declared them. Reading them
    /// back off the first row was an order that happened to hold.
    #[test]
    fn the_columns_come_from_the_table() {
        let cols = [
            crate::port::Column::new(crate::id::PortName::new("Documento"), PortType::TEXT),
            crate::port::Column::new(crate::id::PortName::new("Página"), PortType::NUM),
        ];
        let t = crate::table::make(&cols, Vec::new());
        let names: Vec<String> = crate::table::columns(Some(&t))
            .iter()
            .map(|c| c.name.as_str().to_string())
            .collect();
        assert_eq!(names, vec!["Documento", "Página"]);
    }

    /// And a table with no rows still names them, which is the case the old way could not answer:
    /// an empty report used to lose its headings as well as its data.
    #[test]
    fn an_empty_table_still_names_its_columns() {
        let cols = [crate::port::Column::new(
            crate::id::PortName::new("Documento"),
            PortType::TEXT,
        )];
        let t = crate::table::make(&cols, Vec::new());
        assert_eq!(crate::table::columns(Some(&t)).len(), 1);
    }
}
