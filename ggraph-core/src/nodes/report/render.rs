// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_render` — the finished document.
//!
//! The only node here that touches the world, and it does so through the
//! [`ValueIo`](crate::host::ValueIo) the host already provides: the engine renders bytes and the
//! host decides where they land. A product writing to disk and one writing to an object store need
//! no change on this side.
//!
//! # It writes; it does not arrange
//!
//! One `block` in. It used to carry slots and the same four layout properties as
//! [`layout`](super::layout), which made it two nodes wearing one name — and any property added to
//! a layout had to be added here too, or the two quietly disagreed. Stacking is what the layout
//! node is for, and putting one in front costs a wire and says what it does.
//!
//! Two formats from one tree. `html` is self-contained — charts inlined as SVG, no script, nothing
//! fetched — because a report is emailed, printed and archived. `json` emits the tree itself, for a
//! viewer that wants to draw it interactively. Same blocks, two audiences, and neither is a second
//! renderer in Rust.

use super::{from_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static OUT: [Port; 2] = [
    Port::opt("key", PortType::TEXT),
    Port::opt("bytes", PortType::NUM),
];

/// The document, and the theme it is drawn with.
static IN: [Port; 2] = [
    Port::req("block", BLOCK),
    Port::opt("theme", PortType::TEXT),
];

struct Render;

impl<H: Host> NodeRun<H> for Render {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        if !cx.host.io().enabled() {
            return Err(NodeError::new(
                "this installation has nowhere to put a rendered report",
            ));
        }

        let root = cx
            .input("block")
            .and_then(from_value)
            .ok_or_else(|| NodeError::new("nothing to render"))?;
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
    NodeSpec::effectful("report_render", "ReportRender", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| {
            json!({ "title": "Report", "format": "html" })
        })
        .with_fields(Fields::List(vec![
            Field::text("title", "Title"),
            Field::choice("format", "Format", ["html", "json"]),
        ]))
        .with_timeout(Timeout::Secs(60))
        .running(Render)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One block in, and the layout properties are gone: they belonged to the layout node, and
    /// carrying them here made any new one a thing to add in two places or quietly disagree about.
    #[test]
    fn it_takes_a_document_and_a_theme() {
        let names: Vec<&str> = IN.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["block", "theme"]);
    }

    /// Nothing to render is an error, not an empty document. A report nobody wired is a graph that
    /// is not finished, and writing a blank page hides that.
    #[test]
    fn the_document_is_required() {
        assert!(IN[0].required);
    }
}
