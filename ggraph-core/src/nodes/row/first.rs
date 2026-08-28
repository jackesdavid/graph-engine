// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `first_row` — the first row of a table.
//!
//! The common shape after a search that was asked for one result, or after a sort: one row, read
//! directly, without a loop around it.
//!
//! An empty table produces NO row rather than an empty one. A graph reading a cell of an empty row
//! would get a blank that looks exactly like a blank cell, and those are different facts.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::json;

static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 1] = [Port::opt("row", PortType::TABLE_ROW)];

struct FirstRow;

impl<H: Host> NodeRun<H> for FirstRow {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        if let Some(row) = crate::table::rows(cx.input("table")).into_iter().next() {
            out.insert(PortName::new("row"), row);
        }
        Ok(out)
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        match crate::table::rows(cx.input("table")).first() {
            Some(_) => "1 row".into(),
            None => "no rows".into(),
        }
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("first_row", "First row", "Data")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({}))
        .with_timeout(Timeout::Inline)
        .running(FirstRow)
}
