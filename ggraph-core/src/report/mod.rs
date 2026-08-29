// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Documents assembled by a graph.
//!
//! A report is a tree of [`Block`]s, and a graph builds that tree with nodes and wires. It lives in
//! the engine rather than in a product because a heading, a table and a bar chart know nothing about
//! what they describe — as domain-free as `if` and `for_each`.
//!
//! The proof that they belong here: **the nodes need nothing new from the host.** They are pure, and
//! the only write — the finished file — goes through the [`ValueIo`](crate::host::ValueIo) that
//! already exists. The engine renders; the host decides where the bytes land.

mod block;
mod chart;
mod chart_style;
mod html;
mod layout;
mod preview;
mod sample;
mod schema;

pub use block::{Block, Row};
pub use chart_style::{Bars, ChartStyle};
pub use html::render_html;
pub use layout::{Align, Direction, Justify, Layout};
pub use preview::{preview, renders};
pub use sample::sample;
pub use schema::schema;
