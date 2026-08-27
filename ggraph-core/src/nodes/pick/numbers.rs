// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `pick_numbers` — one numeric field, down a column of records.

use super::{at, column};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 1] = [Port::opt("values", PortType::NUMBERS)];

struct PickNumbers;

impl<H: Host> NodeRun<H> for PickNumbers {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let name = column(cx.config)
            .ok_or_else(|| NodeError::new("no column — which one should be read?"))?;

        let values: Vec<Value> = match cx.input("table") {
            Some(Value::List(rows)) => rows
                .iter()
                // A record with no number there is SKIPPED, not read as zero: "we could not read
                // this" and "this measured zero" are different statements, and a zero bar makes the
                // second one.
                .filter_map(|r| at(r, &name).and_then(|f| f.as_f64()))
                .map(Value::float)
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
    NodeSpec::pure("pick_numbers", "Pick numbers", "Data")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "column": "" }))
        .with_timeout(Timeout::Inline)
        .running(PickNumbers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "We could not read this" is a different statement from "this measured zero".
    #[test]
    fn a_record_without_the_field_is_skipped() {
        let records = [
            Value::Json(json!({ "score": 0.8 })),
            Value::Json(json!({ "other": 1 })),
            Value::Json(json!({ "score": 0.4 })),
        ];
        let read: Vec<f64> = records
            .iter()
            .filter_map(|r| at(r, "score").and_then(|f| f.as_f64()))
            .collect();
        assert_eq!(read, vec![0.8, 0.4]);
    }
}
