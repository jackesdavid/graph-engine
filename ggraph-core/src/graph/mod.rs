// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

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
//! - [`bake`] — the configuration a node can only learn from what is wired into it, copied in so
//!   the ports can resolve from config alone.
//! - [`route`] — which kinds may follow which, and what chains connect two of them. The first
//!   mile of building a graph, answered by search rather than by guessing.
//! - [`readiness`] — whether it is FINISHED: wired up, with every required input filled. The
//!   question a list of graphs asks, and the reason a list no longer has to carry every node and
//!   edge of every graph to answer it.

pub mod bake;
pub mod document;
pub mod readiness;
pub mod route;
pub mod topo;
pub mod validate;

pub use document::*;
