// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_render` — the finished document.
//!
//! The only node here that touches the world, and it does so through the
//! [`ValueIo`](crate::host::ValueIo) the host already provides: the engine renders bytes and the
//! host decides where they land. A product writing to disk and one writing to an object store need
//! no change on this side.
//!
//! Two formats from one tree. `html` is self-contained — charts inlined as SVG, no script, nothing
//! fetched — because a report is emailed, printed and archived. `json` emits the tree itself, for a
//! viewer that wants to draw it interactively. Same blocks, two audiences, and neither is a second
//! renderer in Rust.

use super::{from_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static OUT: [Port; 2] = [
    Port::opt("key", PortType::TEXT),
    Port::opt("bytes", PortType::NUM),
];

fn slot_count(cfg: &Json) -> usize {
    cfg.get("slots")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(1)
        .clamp(1, 24) as usize
}

/// Slots, plus the theme. Same mechanic as the layout node: the report IS a layout that also
/// writes, which is what "a report is made of layouts" means when drawn.
fn ports(cfg: &Json) -> Vec<Port> {
    let mut p: Vec<Port> = (1..=slot_count(cfg))
        .map(|i| Port::new(PortName::new(format!("slot_{i}")), BLOCK, false))
        .collect();
    p.push(Port::opt("theme", PortType::TEXT));
    p
}

struct Render;

impl<H: Host> NodeRun<H> for Render {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        if !cx.host.io().enabled() {
            return Err(NodeError::new(
                "this installation has nowhere to put a rendered report",
            ));
        }

        let children: Vec<_> = (1..=slot_count(cx.config))
            .filter_map(|i| cx.input(&format!("slot_{i}")))
            .filter_map(from_value)
            .collect();

        let layout = serde_json::from_value(cx.config.clone()).unwrap_or_default();
        let root = crate::report::Block::stack(layout, children);
        let title = cx.cfg_str("title").unwrap_or("Report");

        let (bytes, mime) = match cx.cfg_str("format").unwrap_or("html") {
            "json" => (
                serde_json::to_vec_pretty(&json!({ "title": title, "root": root }))
                    .map_err(|e| NodeError::new(e.to_string()))?,
                "application/json",
            ),
            _ => {
                let theme = cx.input("theme").and_then(|v| v.as_text());
                let html = crate::report::render_html(&root, title, theme.as_deref());
                (html.into_bytes(), "text/html")
            }
        };

        let n = bytes.len();
        let key = cx
            .host
            .io()
            .put(&bytes, mime)
            .map_err(|e| NodeError::new(e.to_string()))?;

        let mut out = PortValues::new();
        out.insert(PortName::new("key"), Value::text(key));
        out.insert(PortName::new("bytes"), Value::int(n as i64));
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        let n = out
            .get(&PortName::new("bytes"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        format!("{} KB", (n + 512) / 1024)
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("report_render", "Render report", "Report")
        .with_inputs(Ports::dynamic(ports))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| {
            json!({
                "title": "Report",
                "format": "html",
                "slots": 1,
                "direction": "column",
                "gap": 16,
                "align": "stretch",
                "justify": "start"
            })
        })
        .with_timeout(Timeout::Secs(60))
        .running(Render)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_port_sits_beside_the_slots() {
        let names: Vec<String> = ports(&json!({ "slots": 2 }))
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(names, vec!["slot_1", "slot_2", "theme"]);
    }

    /// One slot by default: the common shape is a single root layout, and a report with two
    /// unwired slots looks broken before anyone has done anything wrong.
    #[test]
    fn one_slot_by_default() {
        assert_eq!(slot_count(&json!({})), 1);
    }
}
