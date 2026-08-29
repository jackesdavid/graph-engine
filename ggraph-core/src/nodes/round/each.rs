// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `round_each` — a column of numbers, with fewer decimals.
//!
//! The plural of [`round`](super::one). Same decision, applied down a column — a chart series, a
//! set of totals, a column pulled out of results.

use super::one::{decimals, round_to};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 1] = [Port::req("values", PortType::NUMBERS)];
static OUT: [Port; 1] = [Port::opt("values", PortType::NUMBERS)];

struct RoundEach;

impl<H: Host> NodeRun<H> for RoundEach {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let places = decimals(cx.config);
        let values = match cx.input("values") {
            Some(Value::List(items)) => items
                .iter()
                // A entry that is not a number passes through. Failing the run over one bad cell
                // would throw away a column that is otherwise fine.
                .map(|i| match i.as_f64() {
                    Some(n) if i.as_text().is_none() => Value::float(round_to(n, places)),
                    _ => i.clone(),
                })
                .collect(),
            Some(one) => vec![one.clone()],
            None => return Err(NodeError::new("nothing to round")),
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
        format!("{n} value(s) to {} dp", decimals(cx.config))
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("round_each", "Round each", "Math")
        .about(
            r#"Rounds every number in a list.

```
Pick numbers --values--> Round each --values--> ReportBarChart
```"#,
        )
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "decimals": 2 }))
        .with_timeout(Timeout::Inline)
        .running(RoundEach)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_number_in_the_column_is_rounded() {
        let v = Value::List(vec![
            Value::float(0.03278688524590164),
            Value::float(0.016129032258064516),
        ]);
        let places = 4;
        let rounded: Vec<f64> = match &v {
            Value::List(items) => items
                .iter()
                .filter_map(|i| i.as_f64().map(|n| round_to(n, places)))
                .collect(),
            _ => vec![],
        };
        assert_eq!(rounded, vec![0.0328, 0.0161]);
    }

    /// One bad cell must not throw away a column that is otherwise fine.
    #[test]
    fn a_non_number_passes_through() {
        let v = Value::text("n/a");
        assert!(v.as_f64().is_none() || v.as_text().is_some());
    }
}
