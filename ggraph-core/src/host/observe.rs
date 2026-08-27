// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What the engine reports while a run is happening.

use crate::value::{PortValues, Value};

/// What the engine reports as it runs: to a live editor, to a run log, to a trace.
///
/// Every method has a default no-op, so a product that wants none of it implements nothing and
/// a test host stays three lines long.
pub trait Observer: Send + Sync {
    /// A node is about to execute.
    fn node_started(&self, _node: u32) {}
    /// A node finished, with a one-line summary and how long it took.
    fn node_finished(&self, _node: u32, _summary: &str, _elapsed_ms: u128) {}
    /// A node emitted a cross-graph event.
    fn emitted(&self, _event: &str, _payload: &PortValues) {}
    /// A node produced something for a person to look at right now.
    fn ui(&self, _node: u32, _event: UiEvent) {}
    /// An exec arm fired: control left `node` through `port`.
    ///
    /// The only signal that says WHICH edge control travelled along. Without it an editor has to
    /// infer the edge from the node it arrives at, which is right by accident until two branches
    /// converge on one node and both edges light.
    fn arm(&self, _node: u32, _port: &str) {}
    /// A node's declaration disagrees with what it did — a defect in the node, not in the graph.
    ///
    /// Reported here rather than through a log crate: a library that picks a logging framework
    /// picks it for every consumer. This hands the decision to the host, which can log it, count
    /// it, or fail a test on it.
    ///
    /// The run continues. Taking down a running installation over a node that returns one port
    /// too many is worse than the defect.
    fn defect(&self, _node: u32, _message: &str) {}
    /// The run is over — nothing else will be reported for it.
    ///
    /// Only interesting to an observer that BUFFERS. One that decides whether to report a node
    /// based on what happened later in the run has no other moment at which to make that call,
    /// and without this hook it would have to be flushed by hand at every call site of
    /// [`run`](crate::run) — which is the kind of thing somebody forgets once and then debugs
    /// as missing output.
    fn run_finished(&self) {}
}

/// Something a node wants shown, live, while the graph runs.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Value { label: String, value: Value },
    Image { label: String, bytes: Vec<u8> },
}
