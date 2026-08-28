// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Building a report from a graph.
//!
//! One node per component, and every one of them returns the same thing: a `block`. That single
//! fact is what makes any arrangement reachable — a layout takes blocks and IS a block, so it takes
//! layouts, and nesting needs no special case in any node.
//!
//! # The ports are report types, not lists
//!
//! A table component takes a `table`, a chart takes `numbers` and `texts`. None of them takes a bare
//! list, so the editor refuses a wrong wire while it is being drawn rather than producing a nonsense
//! report at run time. `texts` is a different type from `numbers` for one reason: swapping a chart's
//! values and names is the mistake that yields a chart which is wrong and looks fine.
//!
//! All three are the ENGINE's types, not the report's. A component fed only by a report-specific
//! type could never be fed by anything else, and there is nothing about a table or a list of numbers
//! that belongs to reporting.
//!
//! Data reaches a report as a `table` — rows of named cells, in the author's column order — and a
//! component takes it as it is. There used to be an adapter between the two; it went away when the
//! source started declaring its own schema, which is where that decision belongs.
//!
//! **The components are pure**, and that is not a detail: a heading is a function from text to a
//! block, not an action. Being pure they are PULLED — `report_render` is reached by exec, and every
//! block it needs resolves backwards through the data wires on its own. So a report is drawn with
//! data wires only, and the exec line runs straight to the render.
//!
//! Making them effectful was the first attempt, and it meant wiring exec through every component in
//! order — the graph then carried the layout twice, once in the data wires and once in an exec
//! sequence that had to agree with them.
//!
//! `report_render` is the exception and the only one: it writes, through the
//! [`ValueIo`](crate::host::ValueIo) the host already provides.

mod bar_chart;
mod heading;
mod layout;
mod paragraph;
mod render;
mod table;

use crate::host::Host;
use crate::port::PortType;
use crate::registry::NodeRegistry;
use crate::report::Block;
use crate::value::Value;

/// What every component returns and every container accepts.
///
/// An open identifier rather than a variant of the engine's type list: the report set is one
/// vocabulary among several a product may add, and reserving a slot in a closed enum for each would
/// make the engine grow with its consumers.
pub const BLOCK: PortType = PortType::new_static("block");

/// The slots a layout node declares, in order. Asked of the node rather than re-read from its
/// config, because a preview that parsed it a second time would be a second answer to keep in step.
pub fn slot_names(cfg: &serde_json::Value) -> Vec<String> {
    layout::slots(cfg)
}

/// A block on a wire.
///
/// Carried as JSON rather than an `Extern`: a value the editor can show and a run log can record is
/// worth more than one that only Rust can open, and a report tree is small.
pub(crate) fn to_value(b: &Block) -> Value {
    Value::Json(serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
}

/// Reads a block off a wire, or explains what arrived instead.
pub(crate) fn from_value(v: &Value) -> Option<Block> {
    match v {
        Value::Json(j) => serde_json::from_value(j.clone()).ok(),
        _ => None,
    }
}

pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(heading::spec());
    reg.register(paragraph::spec());
    reg.register(table::spec());
    reg.register(bar_chart::spec());
    reg.register(layout::spec());
    reg.register(render::spec());
}
