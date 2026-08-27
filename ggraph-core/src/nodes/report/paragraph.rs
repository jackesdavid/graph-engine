// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_paragraph` — prose, with where it came from.
//!
//! `source` is a port and not just a config field because the interesting case is a claim built
//! from something the graph found: the text comes from one wire and its citation from another, and
//! they arrive together or the paragraph is not worth printing.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::json;

static IN: [Port; 2] = [
    Port::opt("text", PortType::TEXT),
    Port::opt("source", PortType::TEXT),
];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

struct Paragraph;

impl<H: Host> NodeRun<H> for Paragraph {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let text = cx
            .input("text")
            .and_then(|v| v.as_text())
            .or_else(|| cx.cfg_str("text").map(str::to_string))
            .unwrap_or_default();

        let source = cx
            .input("source")
            .and_then(|v| v.as_text())
            .filter(|s| !s.trim().is_empty());

        let block = match source {
            Some(s) => crate::report::Block::cited(text, s),
            None => crate::report::Block::paragraph(text),
        };

        let mut out = PortValues::new();
        out.insert(PortName::new("block"), to_value(&block));
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        String::new()
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_paragraph", "Paragraph", "Report")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "text": "" }))
        .with_timeout(Timeout::Inline)
        .running(Paragraph)
}
