// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `round` — one number, with fewer decimals.
//!
//! One in, one out, and nothing else. Paired with [`each`](super::each), which does the same to a
//! column: two narrow nodes rather than one that inspects what it was given, because a node whose
//! behaviour depends on the shape of its input is a node whose wire cannot be checked.
//!
//! It changes a VALUE. How a number is written down — two decimals in a table, a currency prefix, a
//! percent sign — is a different question, answered by whatever renders it.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("value", PortType::NUM)];
static OUT: [Port; 1] = [Port::opt("value", PortType::NUM)];

pub(super) fn decimals(cfg: &Json) -> i32 {
    cfg.get("decimals")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(2)
        .clamp(0, 10) as i32
}

pub(super) fn round_to(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

struct Round;

impl<H: Host> NodeRun<H> for Round {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let n = cx
            .input("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| NodeError::new("nothing to round"))?;

        let mut out = PortValues::new();
        out.insert(
            PortName::new("value"),
            Value::float(round_to(n, decimals(cx.config))),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        format!("{} dp", decimals(cx.config))
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("round", "Round", "Math")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "decimals": 2 }))
        .with_timeout(Timeout::Inline)
        .running(Round)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_loses_its_extra_decimals() {
        assert_eq!(round_to(0.03278688524590164, 4), 0.0328);
    }

    /// Zero decimals is a whole number, not an error.
    #[test]
    fn zero_decimals_is_allowed() {
        assert_eq!(round_to(1.6, 0), 2.0);
        assert_eq!(decimals(&json!({ "decimals": 0 })), 0);
    }

    /// The inspector writes strings; a seeded document writes numbers.
    #[test]
    fn a_count_written_as_a_string_still_counts() {
        assert_eq!(decimals(&json!({ "decimals": "3" })), 3);
    }
}
