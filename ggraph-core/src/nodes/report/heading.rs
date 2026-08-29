// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `report_heading` — a section title.

use super::{to_value, BLOCK};
use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::json;

static IN: [Port; 1] = [Port::opt("text", PortType::TEXT)];
static OUT: [Port; 1] = [Port::opt("block", BLOCK)];

struct Heading;

impl<H: Host> NodeRun<H> for Heading {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let text = cx
            .input("text")
            .and_then(|v| v.as_text())
            .or_else(|| cx.cfg_str("text").map(str::to_string))
            .unwrap_or_default();

        let level = cx
            .cfg_str("level")
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(1);

        let mut out = PortValues::new();
        out.insert(
            PortName::new("block"),
            to_value(&crate::report::Block::heading(text, level)),
        );
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        cx.input("text")
            .and_then(|v| v.as_text())
            .or_else(|| cx.cfg_str("text").map(str::to_string))
            .unwrap_or_default()
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("report_heading", "ReportHeading", "Report")
        .about(
            r#"A heading in a report.

```
Format --text--> ReportHeading --block--> ReportLayout.header
```"#,
        )
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "text": "", "level": "1" }))
        .with_timeout(Timeout::Inline)
        .running(Heading)
}
