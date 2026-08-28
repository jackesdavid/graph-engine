// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_layout` — blocks beside or below each other.
//!
//! The node the whole design rests on. It takes blocks and returns a block, so **it takes layouts**,
//! and from that one fact any arrangement is reachable: a row of columns, a chart beside a table,
//! a header above both.
//!
//! # Slots are named, not counted
//!
//! Each one is added by name in the inspector and appears as a port under that name. A count told
//! you how many there were and nothing about what belonged in them: `slot_1` and `slot_2` are two
//! wires a reader has to trace to understand, while `header` and `body` say it on the canvas.
//!
//! The name is also the thing a renderer can address later — a slot is a place in a document, and a
//! place worth arranging is a place worth naming.
//!
//! Never two edges into one port — `gather` keeps the last and drops the rest without a word, so a
//! fan-in port would lose components silently.

use super::{from_value, to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};

static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

/// The slots this layout has, in order.
///
/// A list of names. A stored graph that has a COUNT still opens — it was a count before, and a
/// document written then must not stop loading because the shape of a config changed.
///
/// Blank names are skipped and repeats are dropped: a port nobody can name is one nothing can be
/// wired to, and two ports with one name collapse into each other, silently losing a component.
pub(super) fn slots(cfg: &Json) -> Vec<String> {
    match cfg.get("slots") {
        Some(Json::Array(items)) => {
            let mut out: Vec<String> = Vec::new();
            for it in items {
                let name = it
                    .get("name")
                    .and_then(Json::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                if !name.is_empty() && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
            }
            out
        }
        // What a graph written before named slots holds.
        other => {
            let n = other
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(2)
                .clamp(0, 24) as usize;
            (1..=n).map(|i| format!("slot_{i}")).collect()
        }
    }
}

fn ports(cfg: &Json) -> Vec<Port> {
    slots(cfg)
        .into_iter()
        .map(|n| Port::new(PortName::new(n), BLOCK, false))
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
        let children: Vec<_> = slots(cx.config)
            .iter()
            .filter_map(|n| cx.input(n))
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
        let names = slots(cx.config);
        let filled = names.iter().filter(|n| cx.input(n).is_some()).count();
        format!("{filled}/{} filled", names.len())
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_layout", "ReportLayout", "Report")
        .with_inputs(Ports::dynamic(ports))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| {
            json!({
                "slots": [{ "name": "header" }, { "name": "body" }],
                "direction": "column",
                "gap": 16,
                "align": "stretch",
                "justify": "start"
            })
        })
        // Declared, so the inspector offers a `+` for a slot and a list for each choice. Left to
        // guess, it drew four free-text boxes over four enumerations and a count over a list.
        .with_fields(Fields::List(vec![
            Field::rows("slots", "Slots", vec![Field::text("name", "Name")]),
            Field::choice("direction", "Direction", ["column", "row"]),
            Field::num("gap", "Gap"),
            Field::choice("align", "Align", ["stretch", "start", "center", "end"]),
            Field::choice("justify", "Justify", ["start", "center", "end", "between"]),
        ]))
        .with_timeout(Timeout::Inline)
        .running(LayoutNode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Direction;

    fn names(cfg: &Json) -> Vec<String> {
        ports(cfg).iter().map(|p| p.name.as_str().to_string()).collect()
    }

    /// A slot is added by name and the port takes that name: `header` says on the canvas what
    /// `slot_1` made a reader trace a wire to find out.
    #[test]
    fn a_slot_is_a_port_under_its_own_name() {
        let cfg = json!({ "slots": [{ "name": "header" }, { "name": "body" }] });
        assert_eq!(names(&cfg), vec!["header", "body"]);
    }

    /// A port nobody can name is one nothing can be wired to.
    #[test]
    fn a_blank_name_is_not_a_slot() {
        assert_eq!(names(&json!({ "slots": [{ "name": "  " }, { "name": "body" }] })), vec!["body"]);
    }

    /// Two ports with one name collapse into each other, and a component disappears without a word.
    #[test]
    fn a_repeated_name_is_dropped() {
        let cfg = json!({ "slots": [{ "name": "a" }, { "name": "a" }] });
        assert_eq!(names(&cfg).len(), 1);
    }

    /// A document written before slots had names must still open. It was a count then.
    #[test]
    fn a_graph_written_before_named_slots_still_loads() {
        assert_eq!(names(&json!({ "slots": 3 })), vec!["slot_1", "slot_2", "slot_3"]);
        assert_eq!(names(&json!({ "slots": "2" })), vec!["slot_1", "slot_2"]);
        assert_eq!(names(&json!({})).len(), 2, "a default that is useful");
    }

    /// A mis-typed count must not draw a node taller than the canvas.
    #[test]
    fn an_old_count_is_clamped() {
        assert_eq!(names(&json!({ "slots": 999 })).len(), 24);
    }

    /// The four decisions are read off the same config the ports came from.
    #[test]
    fn the_layout_reads_its_own_config() {
        let l = layout_of(&json!({ "direction": "row", "gap": 24 }));
        assert_eq!(l.direction, Direction::Row);
        assert_eq!(l.gap, 24);
    }
}
