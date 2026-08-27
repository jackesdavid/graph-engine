// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_series` — where results become a chart.
//!
//! Emits values and labels as two DIFFERENT types, so a chart cannot be wired with them swapped.
//! That swap is the mistake worth designing against: it produces a chart that is wrong and looks
//! entirely fine.

use super::{LABELS, SERIES};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("items", PortType::LIST)];
static OUT: [Port; 2] = [Port::opt("values", SERIES), Port::opt("labels", LABELS)];

fn field(cfg: &Json, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(Json::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn number_at(item: &Value, f: &str) -> Option<f64> {
    match item {
        Value::Json(j) => j.get(f).and_then(Json::as_f64),
        Value::Map(pairs) => pairs
            .iter()
            .find(|(k, _)| k == f)
            .and_then(|(_, v)| v.as_f64()),
        one => one.as_f64(),
    }
}

fn text_at(item: &Value, f: &str) -> Option<String> {
    match item {
        Value::Json(j) => j.get(f).map(|v| match v {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        }),
        Value::Map(pairs) => pairs
            .iter()
            .find(|(k, _)| k == f)
            .map(|(_, v)| v.as_text().unwrap_or_else(|| v.summary())),
        one => one.as_text(),
    }
}

struct Series;

impl<H: Host> NodeRun<H> for Series {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let value_field = field(cx.config, "value_field")
            .ok_or_else(|| NodeError::new("no value_field — a chart needs to know what to plot"))?;
        let label_field = field(cx.config, "label_field");

        let items: Vec<Value> = match cx.input("items") {
            Some(Value::List(i)) => i.clone(),
            Some(one) => vec![one.clone()],
            None => Vec::new(),
        };

        let mut values = Vec::new();
        let mut labels = Vec::new();
        for (i, item) in items.iter().enumerate() {
            // An item with no number is skipped rather than plotted as zero: a zero bar reads as a
            // measured zero, and "we could not read this" is a different statement.
            let Some(v) = number_at(item, &value_field) else {
                continue;
            };
            values.push(Value::float(v));
            labels.push(Value::text(
                label_field
                    .as_deref()
                    .and_then(|f| text_at(item, f))
                    .unwrap_or_else(|| (i + 1).to_string()),
            ));
        }

        let mut out = PortValues::new();
        out.insert(PortName::new("values"), Value::List(values));
        out.insert(PortName::new("labels"), Value::List(labels));
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("values")) {
            Some(Value::List(v)) => format!("{} point(s)", v.len()),
            _ => "0 points".into(),
        }
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_series", "Series from data", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "value_field": "", "label_field": "" }))
        .with_timeout(Timeout::Inline)
        .running(Series)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "We could not read this" is a different statement from "this measured zero".
    #[test]
    fn an_unreadable_item_is_skipped_not_plotted_as_zero() {
        let items = [
            Value::Json(json!({ "score": 0.8, "doc": "a" })),
            Value::Json(json!({ "doc": "b" })),
            Value::Json(json!({ "score": 0.4, "doc": "c" })),
        ];
        let read: Vec<f64> = items.iter().filter_map(|i| number_at(i, "score")).collect();
        assert_eq!(read, vec![0.8, 0.4]);
    }

    #[test]
    fn labels_come_from_the_named_field() {
        let item = Value::Json(json!({ "doc": "a.pdf" }));
        assert_eq!(text_at(&item, "doc"), Some("a.pdf".to_string()));
    }
}
