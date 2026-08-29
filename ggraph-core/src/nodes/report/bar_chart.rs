// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_bar_chart` — one bar per row of a table.
//!
//! It takes the table whole and is told which column is which axis. Feeding it two loose lists —
//! the numbers from one wire and the names from another — meant two nodes in front of it and one
//! assumption nothing checked: that the two lists were still in the same order. They came from the
//! same rows, so they were, until somebody sorted one of them.
//!
//! # The columns are chosen by name, and by type
//!
//! The schema comes in beside the table, so the inspector offers the NUMERIC columns for the values
//! and the TEXT ones for the labels. Swapping a chart's values and names is the mistake that yields
//! a chart which is wrong and looks entirely fine; here it is not a wire to refuse, it is a choice
//! that was never offered.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};

static IN: [Port; 2] = [
    Port::req("table", PortType::TABLE),
    // A shape dependency: connecting a schema writes its columns here, and the choices are built
    // from them. `Ports::dynamic` never sees an edge.
    Port::opt("schema", PortType::SCHEMA),
];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

/// The columns of one type, for the inspector to offer.
fn of_type(cfg: &Json, ty: &PortType) -> Vec<String> {
    crate::nodes::schema::declared(cfg)
        .into_iter()
        .filter(|c| &c.ty == ty)
        .map(|c| c.name.as_str().to_string())
        .collect()
}

fn fields(cfg: &Json) -> Vec<Field> {
    vec![
        Field::text("title", "Title"),
        // Required: a chart with no column to measure has nothing to draw. The labels are not —
        // a bar chart of unnamed bars is still a chart.
        Field::choice("values", "Values", of_type(cfg, &PortType::NUM)).required(),
        Field::choice("labels", "Labels", of_type(cfg, &PortType::TEXT)),
        // How it is drawn, as distinct from what it shows.
        Field::choice("bars", "Bars", ["vertical", "horizontal"]),
        Field::num("gap", "Gap between bars (%)"),
        Field::bool("show_axes", "Show axes"),
        Field::bool("show_labels", "Show labels"),
    ]
}

fn column(cfg: &Json, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

struct BarChart;

impl<H: Host> NodeRun<H> for BarChart {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let value_col = column(cx.config, "values")
            .ok_or_else(|| NodeError::new("no column for the values — connect a schema and pick one"))?;

        let rows = crate::table::rows(cx.input("table"));
        let label_col = column(cx.config, "labels");

        // Read from the SAME row, in one pass. A bar and its name cannot fall out of step if they
        // never travelled separately.
        let mut values = Vec::new();
        let mut labels = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            // A row with no number there is SKIPPED, not read as zero: "we could not read this" and
            // "this measured zero" are different statements, and a zero bar makes the second one.
            let Some(v) = crate::table::cell(row, &value_col).and_then(|v| v.as_f64()) else {
                continue;
            };
            values.push(v);
            labels.push(
                label_col
                    .as_deref()
                    .and_then(|c| crate::table::cell(row, c))
                    .and_then(|v| v.as_text())
                    // Numbered rather than blank: a chart with an unnamed bar is still readable,
                    // and one with a gap where a name should be reads as a missing value.
                    .unwrap_or_else(|| format!("{}", i + 1)),
            );
        }

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::BarChart {
                title: cx.cfg_str("title").unwrap_or("").to_string(),
                labels,
                values,
                style: crate::report::ChartStyle::read(cx.config),
            }),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        match column(cx.config, "values") {
            Some(c) => format!("{c} × {}", crate::table::rows(cx.input("table")).len()),
            None => "no column".into(),
        }
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_bar_chart", "ReportBarChart", "Report")
        .about(r#"Draws a bar chart in a report.

Takes the table and its schema, then you choose which column is which axis BY NAME in the inspector.
How it is drawn — bar direction, gap, whether the axes show — is separate from what it shows.

```
Ask --result--> ReportBarChart --block--> ReportLayout
Schema ------schema-------------^
```"#)
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
        .with_outputs(Ports::Static(&OUT))
        .with_fields(Fields::dynamic(fields))
        .with_config(|| {
            json!({
                "title": "", "values": "", "labels": "", "columns": [],
                "bars": "vertical", "gap": 20, "show_axes": true, "show_labels": true
            })
        })
        .with_timeout(Timeout::Inline)
        .running(BarChart)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::FieldKind;

    fn cfg() -> Json {
        json!({ "columns": [
            { "name": "documento", "type": "text" },
            { "name": "pontuação", "type": "num" },
            { "name": "página", "type": "num" }
        ]})
    }

    fn options(f: &Field) -> Vec<String> {
        match &f.kind {
            FieldKind::Choice(o) => o.clone(),
            _ => panic!("a column is chosen from a list"),
        }
    }

    /// The mistake this prevents: values and names swapped yields a chart that is wrong and looks
    /// entirely fine. Here it is not a wire to refuse — it is a choice never offered.
    #[test]
    fn each_axis_is_offered_only_the_columns_it_can_take() {
        let f = fields(&cfg());
        assert_eq!(options(&f[1]), vec!["pontuação", "página"], "values: the numbers");
        assert_eq!(options(&f[2]), vec!["documento"], "labels: the text");
    }

    /// With no schema there is nothing to choose from, which is visible — rather than a free text
    /// field somebody types a column name into from memory.
    #[test]
    fn without_a_schema_there_is_nothing_to_pick() {
        let f = fields(&json!({}));
        assert!(options(&f[1]).is_empty());
    }
}
