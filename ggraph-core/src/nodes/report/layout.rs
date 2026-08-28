// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_layout` — blocks beside or below each other.
//!
//! The node the whole design rests on. It takes blocks and returns a block, so **it takes layouts**,
//! and from that one fact any arrangement is reachable: a row of columns, a chart beside a table,
//! a header above both.
//!
//! Slots are dynamic ports driven by config, the mechanic the editor already has: set `slots` in the
//! inspector and the wires appear. Never two edges into one port — `gather` keeps the last and drops
//! the rest without a word, so a fan-in port would lose components silently.

use super::{from_value, to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};

static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

/// How many slots this layout was configured for.
///
/// Clamped: zero slots is a layout that can hold nothing, and a hundred is a mis-typed config that
/// would draw a node taller than the canvas.
fn slot_count(cfg: &Json) -> usize {
    cfg.get("slots")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(2)
        .clamp(1, 24) as usize
}

fn ports(cfg: &Json) -> Vec<Port> {
    (1..=slot_count(cfg))
        .map(|i| Port::new(PortName::new(format!("slot_{i}")), BLOCK, false))
        .collect()
}

fn layout_of(cfg: &Json) -> crate::report::Layout {
    crate::report::Layout::read(cfg)
}

struct LayoutNode;

impl<H: Host> NodeRun<H> for LayoutNode {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        // In slot order, and skipping the empty ones. An unwired slot is a gap the author left,
        // not an empty box to draw — and a dead branch upstream leaves exactly that.
        let children: Vec<_> = (1..=slot_count(cx.config))
            .filter_map(|i| cx.input(&format!("slot_{i}")))
            .filter_map(from_value)
            .collect();

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::stack(layout_of(cx.config), children)),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        let filled = (1..=slot_count(cx.config))
            .filter(|i| cx.input(&format!("slot_{i}")).is_some())
            .count();
        format!("{filled}/{} filled", slot_count(cx.config))
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_layout", "ReportLayout", "Report")
        .with_inputs(Ports::dynamic(ports))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| {
            json!({
                "slots": 2,
                "direction": "column",
                "gap": 16,
                "align": "stretch",
                "justify": "start"
            })
        })
        .with_timeout(Timeout::Inline)
        .running(LayoutNode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Direction;

    /// The config drives the ports, which is what makes the inspector able to add them.
    #[test]
    fn slots_come_from_the_config() {
        let names: Vec<String> = ports(&json!({ "slots": 3 }))
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(names, vec!["slot_1", "slot_2", "slot_3"]);
    }

    /// A mis-typed config must not draw a node taller than the canvas, and zero slots is a layout
    /// that can hold nothing.
    #[test]
    fn the_slot_count_is_clamped() {
        assert_eq!(slot_count(&json!({ "slots": 0 })), 1);
        assert_eq!(slot_count(&json!({ "slots": 999 })), 24);
        assert_eq!(slot_count(&json!({})), 2, "a default that is useful");
    }

    /// The inspector writes strings; the seeded document writes numbers. Both are the same layout.
    #[test]
    fn a_slot_count_written_as_a_string_still_counts() {
        assert_eq!(slot_count(&json!({ "slots": "4" })), 4);
    }

    /// The four decisions are read off the same config the ports came from.
    #[test]
    fn the_layout_reads_its_own_config() {
        let l = layout_of(&json!({ "direction": "row", "gap": 24 }));
        assert_eq!(l.direction, Direction::Row);
        assert_eq!(l.gap, 24);
    }
}
