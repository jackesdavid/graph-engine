// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_bar_chart` — one bar per label.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

/// Two different types on purpose. Swapping a chart's values and names is the mistake that yields
/// a chart which is wrong and looks entirely fine — so the editor refuses the wire.
static IN: [Port; 2] = [
    Port::req("values", PortType::NUMBERS),
    Port::opt("labels", PortType::TEXTS),
];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

fn numbers(v: Option<&Value>) -> Vec<f64> {
    match v {
        Some(Value::List(items)) => items.iter().filter_map(|i| i.as_f64()).collect(),
        Some(one) => one.as_f64().into_iter().collect(),
        None => Vec::new(),
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::List(items)) => items
            .iter()
            .map(|i| i.as_text().unwrap_or_else(|| i.summary()))
            .collect(),
        Some(one) => vec![one.as_text().unwrap_or_else(|| one.summary())],
        None => Vec::new(),
    }
}

struct BarChart;

impl<H: Host> NodeRun<H> for BarChart {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let values = numbers(cx.input("values"));
        let mut labels = strings(cx.input("labels"));

        // Labels and values are read in step. Padding rather than failing, because a chart with a
        // missing name is still readable and a report that refuses to render over one is not.
        while labels.len() < values.len() {
            labels.push(format!("{}", labels.len() + 1));
        }
        labels.truncate(values.len());

        let title = cx.cfg_str("title").unwrap_or("").to_string();

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::BarChart {
                title,
                labels,
                values,
            }),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        format!("{} bar(s)", numbers(cx.input("values")).len())
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_bar_chart", "ReportBarChart", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "title": "" }))
        .with_timeout(Timeout::Inline)
        .running(BarChart)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fewer labels than values is a readable chart, not a failed report.
    #[test]
    fn missing_labels_are_filled_rather_than_refused() {
        let values = Value::List(vec![
            Value::float(1.0),
            Value::float(2.0),
            Value::float(3.0),
        ]);
        let labels = Value::List(vec![Value::text("a")]);
        let vs = numbers(Some(&values));
        let mut ls = strings(Some(&labels));
        while ls.len() < vs.len() {
            ls.push(format!("{}", ls.len() + 1));
        }
        assert_eq!(ls, vec!["a", "2", "3"]);
    }

    /// Scores from a search arrive as floats; counts arrive as ints. Both are bars.
    #[test]
    fn ints_and_floats_are_both_numbers() {
        let v = Value::List(vec![Value::int(2), Value::float(0.5)]);
        assert_eq!(numbers(Some(&v)), vec![2.0, 0.5]);
    }
}
