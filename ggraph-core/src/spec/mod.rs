// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a node *is*: one declaration, replacing fourteen.
//!
//! In the engine this was extracted from, a node kind was described by fourteen `match` arms
//! spread across two files — the enum, the palette list, the slug, the label, the category,
//! purity, two port tables, the exec arms, the default config, the timeout, the executor, the
//! log summary, plus three config-derived overrides somewhere else entirely. Adding a node meant
//! finding all fourteen. Forgetting one was silent, and which one you forgot decided whether it
//! broke in the editor, in the scheduler, or a month later in a customer's saved graph.
//!
//! Here a node is one [`NodeSpec`] value, in one file, next to its implementation and its tests.
//!
//! ## The three behaviours
//!
//! Most nodes just compute: inputs in, outputs out. But two other shapes exist and pretending
//! they do not is what forces a scheduler to grow a chain of `if kind == ...` special cases.
//!
//! - [`Behavior::Run`] — computes. The common case.
//! - [`Behavior::Route`] — computes, then says which exec arms fire. A branch, a switch.
//! - [`Behavior::Step`] — cooperates with the scheduler: it can re-enter itself (a loop), end
//!   the run without firing anything (waiting for a person), or read durable state.
//!
//! The engine this was extracted from had eleven nodes whose real semantics lived inline inside
//! the scheduler, because there was no third shape for them. Every one of the eleven ends in the
//! same four moves — outputs, arms, re-enter, log — which is exactly [`Step`].
//!
//! Split by what each part declares:
//!
//! - [`error`] — how a node says it failed, and whether retrying could help;
//! - [`shape`] — its pins, its arms, its timeout, its purity;
//! - [`cx`] — what it is handed when it runs;
//! - [`step`] — what a cooperating node hands back about control.
//!
//! What stays here is [`NodeSpec`] itself and its builder: the one declaration that ties the rest
//! together, and the reason a kind becomes behaviour in one place rather than in fourteen match
//! arms.

pub mod cx;
pub mod error;
pub mod shape;
pub mod step;

pub use cx::*;
pub use error::*;
pub use shape::*;
pub use step::*;

use crate::graph::PortLookup;

/// Builds a port list from a node's configuration.
pub type PortsFn = Arc<dyn Fn(&Json) -> Vec<Port> + Send + Sync>;

/// Builds a node kind's default configuration.
pub type ConfigFn = Arc<dyn Fn() -> Json + Send + Sync>;
use crate::host::Host;
use crate::id::{NodeId, PortName};
use crate::port::Port;
use crate::value::Value;

/// Run-scoped named values, shared across the nodes of one run.
pub type Vars = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<PortName, Value>>>;
use serde_json::Value as Json;
use std::sync::Arc;

/// What a node does when reached.
pub enum Behavior<H: Host> {
    /// Nothing. A comment, a heading, a wire organiser.
    Inert,
    Run(Arc<dyn NodeRun<H>>),
    Route(Arc<dyn NodeRoute<H>>),
    Step(Arc<dyn NodeStep<H>>),
}

impl<H: Host> std::fmt::Debug for Behavior<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Behavior::Inert => "Inert",
            Behavior::Run(_) => "Run",
            Behavior::Route(_) => "Route",
            Behavior::Step(_) => "Step",
        })
    }
}

/// Everything about one kind of node.
pub struct NodeSpec<H: Host> {
    pub id: NodeId,
    /// Names this kind also answers to. Needed the day a node is renamed: without it, every
    /// stored graph containing the old name stops loading, and the failure is at load time in
    /// front of a customer rather than at review time.
    pub aliases: &'static [&'static str],
    pub label: &'static str,
    /// The palette group.
    pub category: &'static str,
    /// Registered, but not offered in the palette. A kind that is real and resolvable but that
    /// nobody should add by hand — a wire organiser the engine collapses before execution.
    /// Seeding a registry from the palette list instead of the full set is how a stored graph
    /// containing one stops loading.
    pub hidden: bool,
    pub inputs: Ports,
    pub outputs: Ports,
    pub exec_out: ExecOut,
    pub default_config: ConfigFn,
    pub purity: Purity,
    pub timeout: Timeout,
    pub behavior: Behavior<H>,
}

impl<H: Host> std::fmt::Debug for NodeSpec<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written because the config builder is a closure. Reports what a person debugging
        // a registry actually wants: which node, and what shape it has.
        f.debug_struct("NodeSpec")
            .field("id", &self.id)
            .field("category", &self.category)
            .field("purity", &self.purity)
            .field("timeout", &self.timeout)
            .field("behavior", &self.behavior)
            .finish()
    }
}

impl<H: Host> NodeSpec<H> {
    /// The common shape: effectful, one exec arm, no aliases, in the palette.
    pub fn effectful(id: &'static str, label: &'static str, category: &'static str) -> Self {
        NodeSpec {
            id: NodeId::new_static(id),
            aliases: &[],
            label,
            category,
            hidden: false,
            inputs: Ports::NONE,
            outputs: Ports::NONE,
            exec_out: ExecOut::DEFAULT,
            default_config: Arc::new(|| Json::Object(Default::default())),
            purity: Purity::EFFECTFUL,
            timeout: Timeout::Secs(30),
            behavior: Behavior::Inert,
        }
    }

    /// A node with no exec pins: it is read, not reached.
    pub fn pure(id: &'static str, label: &'static str, category: &'static str) -> Self {
        NodeSpec {
            exec_out: ExecOut::None,
            purity: Purity::PURE,
            timeout: Timeout::Inline,
            ..Self::effectful(id, label, category)
        }
    }

    pub fn with_inputs(mut self, p: Ports) -> Self {
        self.inputs = p;
        self
    }
    pub fn with_outputs(mut self, p: Ports) -> Self {
        self.outputs = p;
        self
    }
    pub fn with_exec_out(mut self, e: ExecOut) -> Self {
        self.exec_out = e;
        self
    }
    pub fn with_config(mut self, f: impl Fn() -> Json + Send + Sync + 'static) -> Self {
        self.default_config = Arc::new(f);
        self
    }
    pub fn with_timeout(mut self, t: Timeout) -> Self {
        self.timeout = t;
        self
    }
    pub fn with_purity(mut self, p: Purity) -> Self {
        self.purity = p;
        self
    }
    pub fn with_aliases(mut self, a: &'static [&'static str]) -> Self {
        self.aliases = a;
        self
    }
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }
    pub fn running(mut self, r: impl NodeRun<H> + 'static) -> Self {
        self.behavior = Behavior::Run(Arc::new(r));
        self
    }
    pub fn routing(mut self, r: impl NodeRoute<H> + 'static) -> Self {
        self.behavior = Behavior::Route(Arc::new(r));
        self
    }
    pub fn stepping(mut self, s: impl NodeStep<H> + 'static) -> Self {
        self.behavior = Behavior::Step(Arc::new(s));
        self
    }

    /// Whether this node takes control flow in.
    pub fn has_exec_in(&self) -> bool {
        self.purity.has_exec()
    }
}

/// A single spec, viewed as a [`PortLookup`] — for validating a graph made of one kind.
impl<H: Host> PortLookup for NodeSpec<H> {
    fn inputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        (*kind == self.id).then(|| self.inputs.resolve(config))
    }
    fn outputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        (*kind == self.id).then(|| self.outputs.resolve(config))
    }
    fn exec_outs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        (*kind == self.id).then(|| self.exec_out.resolve(config))
    }
    fn has_exec_in(&self, kind: &NodeId) -> bool {
        *kind == self.id && self.purity.has_exec()
    }
}
