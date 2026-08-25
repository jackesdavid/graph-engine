//! Turning a configuration literal into a value on an unwired input port.

use crate::value::Value;
use serde_json::Value as Json;

/// Turns a configuration literal into a value on an **unwired** input port.
///
/// Most graph editors let a port be either wired or typed into an inspector, and the second is
/// by far the more common. The engine cannot interpret those itself: what `"front door"` means
/// on a port of the product's own type is the product's business, and answering may take a
/// lookup — a device id resolved against a device store, a table id against a run-start
/// snapshot.
///
/// The engine asks **before** anything checks whether a required input is present, and that
/// ordering is the point rather than an implementation detail. A scheduler that validated first
/// would decide the branch was dead, skip everything under it, and report the run `ok` — a
/// failure with nothing red anywhere.
pub trait Literals: Send + Sync {
    /// `None` leaves the port unfilled, which downstream reads as "not provided".
    fn read(&self, kind: &crate::NodeId, port: &crate::port::Port, config: &Json) -> Option<Value>;
}

/// Nothing is read from configuration: every value arrives on a wire, or the node reads its own
/// config through [`NodeCx::input_or_cfg`](crate::spec::NodeCx::input_or_cfg). Fine for a small
/// node set; it stops scaling at roughly the point where every node has to remember to do it.
#[derive(Debug)]
pub struct NoLiterals;

impl Literals for NoLiterals {
    fn read(&self, _: &crate::NodeId, _: &crate::port::Port, _: &Json) -> Option<Value> {
        None
    }
}
