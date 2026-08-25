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
}

/// Something a node wants shown, live, while the graph runs.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Value { label: String, value: Value },
    Image { label: String, bytes: Vec<u8> },
}
