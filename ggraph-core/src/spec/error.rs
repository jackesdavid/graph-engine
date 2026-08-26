// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! How a node says it failed, and whether trying again could help.
//!
//! The default is [`Retry::Never`], and the asymmetry with a host failure is the whole point. A
//! NODE failing is normally about its inputs — a missing field, a value it cannot parse, an
//! operator that makes no sense for the types — and none of that changes on a second attempt. A
//! HOST failing is normally about the world, which does. Each default points the safe way for its
//! side, and a caller who knows better overrides it.

/// Builds a port list from a node's configuration.
pub type PortsFn = Arc<dyn Fn(&Json) -> Vec<Port> + Send + Sync>;

/// Builds a node kind's default configuration.
pub type ConfigFn = Arc<dyn Fn() -> Json + Send + Sync>;
use crate::id::PortName;
use crate::port::Port;
use crate::value::Value;

/// Run-scoped named values, shared across the nodes of one run.
pub type Vars = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<PortName, Value>>>;
use serde_json::Value as Json;
use std::sync::Arc;

/// Something a node refused to do, and whether trying again could help.
///
/// The default is [`Retry::Never`], and the asymmetry with [`HostError`] is the point: a *node*
/// failing is normally about its inputs — a missing field, a value it cannot parse, an operator
/// that makes no sense for the types — and none of that changes on a second attempt. A *host*
/// failing is normally about the world, which does change. Both defaults are the safe direction
/// for their side, and a caller that knows better overrides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeError {
    pub retry: crate::host::Retry,
    pub message: String,
}

impl NodeError {
    /// It will fail the same way with the same inputs. The default.
    pub fn new(message: impl Into<String>) -> Self {
        NodeError {
            retry: crate::host::Retry::Never,
            message: message.into(),
        }
    }

    /// The world got in the way; the same node with the same inputs might succeed later.
    pub fn transient(message: impl Into<String>) -> Self {
        NodeError {
            retry: crate::host::Retry::Maybe,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NodeError {}

impl From<String> for NodeError {
    fn from(s: String) -> Self {
        NodeError::new(s)
    }
}
impl From<&str> for NodeError {
    fn from(s: &str) -> Self {
        NodeError::new(s)
    }
}
impl From<crate::host::HostError> for NodeError {
    /// A host failure keeps the host's own judgement. The node did nothing wrong; the world did.
    fn from(e: crate::host::HostError) -> Self {
        NodeError {
            retry: e.retry,
            message: e.message,
        }
    }
}
