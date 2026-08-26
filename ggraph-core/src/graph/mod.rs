// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The graph itself: what one is, what order it implies, and whether it makes sense.
//!
//! Three things that are easy to confuse and are deliberately separate:
//!
//! - [`document`] — the stored shape. Nodes, edges, and a product's own metadata. This is what is
//!   serialised, so a change here is a change to every graph anybody has saved.
//! - [`topo`] — the order execution implies, and the back-edges that make a loop a loop rather
//!   than a cycle nobody can run.
//! - [`validate`] — whether a graph is coherent, answered ahead of running it. A wire whose port
//!   types disagree is refused when it is drawn; this catches what only shows up once the whole
//!   document is in view.

pub mod document;
pub mod topo;
pub mod validate;

pub use document::*;
