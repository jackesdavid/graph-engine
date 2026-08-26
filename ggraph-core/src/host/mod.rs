// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The seam: everything the SCHEDULER needs from the world, and nothing else.
//!
//! Four capabilities and four facts. Durable state, a blob store, an observer, a reader of
//! configuration literals — plus a run id, a clock, an instance key and a way to ask for control
//! back later.
//!
//! What is deliberately NOT here: an approval channel, a network, a model, a table store. The
//! scheduler calls none of them; they are what the bundled node library needs, and they live in
//! [`nodes::services`](crate::nodes::services), handed over at registration. A product that does
//! not register those nodes implements nothing for them.
//!
//! The seam is smaller than it first looks. The engine this was extracted from threaded a
//! fourteen-field context through every node; the scheduler used six of those fields, and of the
//! six, the tenant id was only ever an argument to a store call and one more was only ever read
//! inside a single node.

pub mod blobs;
pub mod error;
pub mod literals;
pub mod observe;
pub mod state;
pub mod testkit;

pub use blobs::{Disabled, ValueIo};
pub use error::{HostError, Retry};
pub use literals::{Literals, NoLiterals};
pub use observe::{Observer, UiEvent};
pub use state::{NodeTarget, RunKey, Slot, StateKey, StateStore};

use crate::value::{PortValues, Value};
use smol_str::SmolStr;
use uuid::Uuid;

/// Everything the engine needs from its product.
///
/// One trait, one type parameter, chosen once at the binary's root. Nodes written against the
/// concrete host reach whatever else that host offers — cameras, a corpus, a face index —
/// without any of it appearing here.
pub trait Host: Send + Sync + Clone + 'static {
    /// The product's per-graph policy. See [`Graph`](crate::Graph).
    type Meta: crate::graph::GraphMeta;

    fn state(&self) -> &dyn StateStore;
    fn io(&self) -> &dyn ValueIo;
    fn observer(&self) -> &dyn Observer;

    fn run_id(&self) -> Uuid;

    /// Seconds since the epoch. The engine never calls the clock directly, so a test host can
    /// make a time window pass instantly instead of sleeping through it.
    fn now_epoch_secs(&self) -> i64;

    /// Which instance of this graph a signal belongs to.
    ///
    /// This is the whole of instance scoping, from the core's point of view: an opaque string.
    /// "One run per camera" is a rule about cameras and lives in the product; the engine only
    /// needs to know that two signals with different keys do not share node state.
    fn instance_key(&self, _meta: &Self::Meta, _payload: &PortValues) -> SmolStr {
        SmolStr::default()
    }

    /// How a configuration literal becomes a value on an unwired input port.
    ///
    /// Defaults to [`NoLiterals`], under which nodes read their own configuration themselves.
    fn literals(&self) -> &dyn Literals {
        &NoLiterals
    }

    /// Re-enter `target` at `at_epoch_secs`. The durable timer, and the retry.
    fn schedule(&self, at_epoch_secs: i64, target: NodeTarget) -> Result<(), HostError>;
}

/// A convenience for nodes: read an input by name.
pub fn input<'a>(inputs: &'a PortValues, name: &str) -> Option<&'a Value> {
    inputs.get(name)
}
