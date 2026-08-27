// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `pick_texts` — one field, down a column of records, as text.

use super::{at, column};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 1] = [Port::opt("values", PortType::TEXTS)];

struct PickTexts;

impl<H: Host> NodeRun<H> for PickTexts {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let name = column(cx.config)
            .ok_or_else(|| NodeError::new("no column — which one should be read?"))?;

        let values: Vec<Value> = match cx.input("table") {
            Some(Value::List(rows)) => rows
                .iter()
                // Absent reads as empty rather than being dropped: a label column has to stay in
                // step with the numbers it names, and a shorter one silently renames every bar
                // after the gap.
                .map(|r| Value::text(at(r, &name).map(|f| f.as_text()).unwrap_or_default()))
                .collect(),
            _ => Vec::new(),
        };

        let mut out = PortValues::new();
        out.insert(PortName::new("values"), Value::List(values));
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        let n = match out.get(&PortName::new("values")) {
            Some(Value::List(v)) => v.len(),
            _ => 0,
        };
        format!("{} × {n}", column(cx.config).unwrap_or_default())
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("pick_texts", "Pick texts", "Data")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "column": "" }))
        .with_timeout(Timeout::Inline)
        .running(PickTexts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label column must stay in step with the numbers it names. A shorter one silently renames
    /// every bar after the gap.
    #[test]
    fn an_absent_field_keeps_its_place() {
        let records = [
            Value::Json(json!({ "doc": "a.pdf" })),
            Value::Json(json!({})),
        ];
        let read: Vec<String> = records
            .iter()
            .map(|r| at(r, "doc").map(|f| f.as_text()).unwrap_or_default())
            .collect();
        assert_eq!(read, vec!["a.pdf".to_string(), String::new()]);
    }
}
