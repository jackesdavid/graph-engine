// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_paragraph` — prose.
//!
//! Text in, a block out, and nothing else. It carried a second port for a citation, which made it
//! two things: a paragraph, and a paragraph-with-a-source. A citation belongs inside the text that
//! makes the claim — that is where it is written, and where it is checked.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::json;

static IN: [Port; 1] = [Port::opt("text", PortType::TEXT)];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

struct Paragraph;

impl<H: Host> NodeRun<H> for Paragraph {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let text = cx
            .input("text")
            .and_then(|v| v.as_text())
            .or_else(|| cx.cfg_str("text").map(str::to_string))
            .unwrap_or_default();

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::paragraph(text)),
        );
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        String::new()
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_paragraph", "ReportParagraph", "Report")
        .about(
            r#"A paragraph of text in a report.

```
Ask --answer--> ReportParagraph --block--> ReportLayout.body
```"#,
        )
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "text": "" }))
        .with_timeout(Timeout::Inline)
        .running(Paragraph)
}
