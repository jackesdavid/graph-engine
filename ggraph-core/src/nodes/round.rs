// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `round` — fewer decimals.
//!
//! A data node, in the flow, where somebody can see it. Rounding that happens inside whatever
//! displays a number is rounding nobody can find when the figure looks wrong.
//!
//! Works on what a graph actually holds: a number, a list of numbers, or a list of records with a
//! named field. The last is the common one — a search returns records and one of their fields is a
//! score — and doing it there fixes every later reader of that field at once.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("value", PortType::ANY)];
static OUT: [Port; 1] = [Port::opt("value", PortType::ANY)];

fn decimals(cfg: &Json) -> i32 {
    cfg.get("decimals")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(2)
        .clamp(0, 10) as i32
}

fn field(cfg: &Json) -> Option<String> {
    cfg.get("field")
        .and_then(Json::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn round_to(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// One value, rounded if it is a number and left alone if it is not.
///
/// Text that happens to look numeric is left as text: a document named `2024` is a name, and
/// turning it into a number would change what the report says it is.
fn round_value(v: &Value, places: i32, field: Option<&str>) -> Value {
    match (v, field) {
        (Value::Json(Json::Object(map)), Some(f)) => {
            let mut m = map.clone();
            if let Some(n) = m.get(f).and_then(Json::as_f64) {
                if let Some(rounded) = serde_json::Number::from_f64(round_to(n, places)) {
                    m.insert(f.to_string(), Json::Number(rounded));
                }
            }
            Value::Json(Json::Object(m))
        }
        (Value::Json(Json::Number(n)), None) => n
            .as_f64()
            .and_then(|v| serde_json::Number::from_f64(round_to(v, places)))
            .map(|n| Value::Json(Json::Number(n)))
            .unwrap_or_else(|| v.clone()),
        (Value::Num(_), None) => v
            .as_f64()
            .map(|n| Value::float(round_to(n, places)))
            .unwrap_or_else(|| v.clone()),
        (Value::List(items), _) => Value::List(
            items
                .iter()
                .map(|i| round_value(i, places, field))
                .collect(),
        ),
        _ => v.clone(),
    }
}

struct Round;

impl<H: Host> NodeRun<H> for Round {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let input = cx
            .input("value")
            .ok_or_else(|| NodeError::new("nothing to round"))?;

        let mut out = PortValues::new();
        out.insert(
            PortName::new("value"),
            round_value(input, decimals(cx.config), field(cx.config).as_deref()),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        match field(cx.config) {
            Some(f) => format!("{f} to {} dp", decimals(cx.config)),
            None => format!("{} dp", decimals(cx.config)),
        }
    }
}

pub fn spec<H: Host>(_: &crate::Services) -> NodeSpec<H> {
    NodeSpec::pure("round", "Round", "Math")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "decimals": 2, "field": "" }))
        .with_timeout(Timeout::Inline)
        .running(Round)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The common case: a list of records with one numeric field. Rounding it here fixes every
    /// later reader of that field at once — the table and the chart both.
    #[test]
    fn a_field_is_rounded_across_a_list_of_records() {
        let items = Value::List(vec![
            Value::Json(json!({ "doc": "a.pdf", "score": 0.03278688524590164 })),
            Value::Json(json!({ "doc": "b.pdf", "score": 0.016129032258064516 })),
        ]);
        let Value::List(out) = round_value(&items, 4, Some("score")) else {
            panic!()
        };
        let Value::Json(first) = &out[0] else {
            panic!()
        };
        assert_eq!(first["score"].as_f64(), Some(0.0328));
        assert_eq!(first["doc"], "a.pdf", "the other fields are untouched");
    }

    /// A document named `2024` is a name. Turning it into a number would change what the report
    /// says it is.
    #[test]
    fn text_that_looks_numeric_is_left_alone() {
        let v = Value::text("2024");
        assert_eq!(round_value(&v, 2, None).as_text().as_deref(), Some("2024"));
    }

    /// A bare list of numbers is the other shape a graph holds.
    #[test]
    fn a_list_of_numbers_rounds_without_a_field() {
        let v = Value::List(vec![Value::float(1.239), Value::float(2.0)]);
        let Value::List(out) = round_value(&v, 2, None) else {
            panic!()
        };
        assert_eq!(out[0].as_f64(), Some(1.24));
        assert_eq!(out[1].as_f64(), Some(2.0));
    }

    /// Zero decimals is a whole number, not an error.
    #[test]
    fn zero_decimals_is_allowed() {
        assert_eq!(round_to(1.6, 0), 2.0);
        assert_eq!(decimals(&json!({ "decimals": 0 })), 0);
    }
}
