// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a node declares about itself before it ever runs.
//!
//! Its pins, its arms, how long it may take, and whether it can be skipped when nothing changed.
//!
//! [`Ports`] being able to be dynamic is not exotic and is load-bearing: a node that emits an
//! event grows one input per field of that event, a switch grows one arm per configured label.
//! Making that part of the declaration is what stops half the callers from reading a static table
//! and getting a different answer than the scheduler does — which is a bug that shows up as an
//! editor drawing a pin the run does not have.

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

/// A port set that may depend on the node's configuration.
///
/// The dynamic case is not exotic — a node that emits an event grows one input per field of
/// that event, a switch grows one arm per configured label. Making it part of the declaration
/// is what stops half the callers from reading the static table and getting a different answer
/// than the scheduler.
pub enum Ports {
    Static(&'static [Port]),
    /// A closure rather than a plain `fn` pointer, so a product can build the port list from
    /// whatever it already has — a table, an enum, a catalogue loaded at boot — instead of
    /// having to reach everything it needs through a function that captures nothing.
    Dynamic(PortsFn),
}

impl Ports {
    pub const NONE: Ports = Ports::Static(&[]);

    pub fn dynamic(f: impl Fn(&Json) -> Vec<Port> + Send + Sync + 'static) -> Self {
        Ports::Dynamic(Arc::new(f))
    }

    pub fn resolve(&self, config: &Json) -> Vec<Port> {
        match self {
            Ports::Static(p) => p.to_vec(),
            Ports::Dynamic(f) => f(config),
        }
    }
}

impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ports::Static(p) => write!(f, "Static({} port(s))", p.len()),
            Ports::Dynamic(_) => write!(f, "Dynamic"),
        }
    }
}
/// Two independent questions about a node, asked separately.
///
/// They were one enum — `Effectful` / `Pure` / `PureSource` — which answered "does it have exec
/// pins?" and "is it re-read on every access?" with a single value, as though the second only
/// applied when the first was "no".
///
/// The first consumer had a third property under a colliding name. Its `is_pure_source` means
/// *"run this even as an orphan entry inside a sub-run"* — a seeding rule, from a real incident.
/// Ours meant *"re-evaluate on every read"* — a caching rule. Several of its kinds were the
/// first without being the second, and deriving purity from the wrong one silently stripped a
/// node's exec pin out of a published catalog. A snapshot test caught it; review had not.
///
/// Two fields cannot collide like that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Purity {
    /// Has exec pins, and so runs when control reaches it. `false` means it is pulled by
    /// whoever reads it instead.
    pub has_exec: bool,
    /// Re-read on every access rather than once per run. Only meaningful when `has_exec` is
    /// false — a node control reaches runs every time control reaches it, and there is no cached
    /// answer to opt out of.
    pub reevaluates: bool,
}

impl Purity {
    /// Runs when control reaches it. The ordinary node.
    pub const EFFECTFUL: Purity = Purity {
        has_exec: true,
        reevaluates: false,
    };

    /// Pulled by whoever reads it, once per run.
    pub const PURE: Purity = Purity {
        has_exec: false,
        reevaluates: false,
    };

    /// Pulled, and re-read every time — a clock, a sensor, a variable a loop keeps changing.
    pub const PURE_SOURCE: Purity = Purity {
        has_exec: false,
        reevaluates: true,
    };

    pub fn has_exec(self) -> bool {
        self.has_exec
    }
}

impl Default for Purity {
    fn default() -> Self {
        Purity::EFFECTFUL
    }
}

/// How long a node may take before the engine gives up on it.
#[derive(Clone)]
pub enum Timeout {
    /// Runs inline. For nodes that cannot block — arithmetic, string formatting.
    Inline,
    Secs(u64),
    /// How long this node may take, read from its own configuration.
    ///
    /// The same reason [`Ports::Dynamic`] exists: a declaration that depends on how the user
    /// filled the node in cannot be a constant, and a scheduler asking the constant while an
    /// editor asks the node gives two answers about one node. "Too long" is exactly that kind of
    /// property — a person who sets a slow endpoint to sixty seconds means it, and a spec that
    /// can only state the default silently gives them fifteen.
    ///
    /// Returning `None` means inline.
    FromConfig(TimeoutFn),
}

/// Reads a node's timeout out of its configuration. `None` means inline.
pub type TimeoutFn = Arc<dyn Fn(&Json) -> Option<u64> + Send + Sync>;

impl std::fmt::Debug for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Timeout::Inline => write!(f, "Inline"),
            Timeout::Secs(s) => write!(f, "Secs({s})"),
            Timeout::FromConfig(_) => write!(f, "FromConfig(..)"),
        }
    }
}

impl Timeout {
    /// Settle it against a node's actual configuration.
    pub fn resolve(&self, config: &Json) -> Option<u64> {
        match self {
            Timeout::Inline => None,
            Timeout::Secs(s) => Some(*s),
            Timeout::FromConfig(f) => f(config),
        }
    }

    /// Build one from a closure over configuration.
    pub fn from_config(f: impl Fn(&Json) -> Option<u64> + Send + Sync + 'static) -> Timeout {
        Timeout::FromConfig(Arc::new(f))
    }
}

/// What fires after a node runs.
pub enum ExecOut {
    /// Pure nodes.
    None,
    Static(&'static [Port]),
    /// Arms declared by configuration — a switch.
    Dynamic(PortsFn),
}

impl ExecOut {
    /// The single "and then" arm most nodes have.
    pub const DEFAULT: ExecOut = ExecOut::Static(&[crate::port::EXEC_OUT]);

    pub fn dynamic(f: impl Fn(&Json) -> Vec<Port> + Send + Sync + 'static) -> Self {
        ExecOut::Dynamic(Arc::new(f))
    }

    pub fn resolve(&self, config: &Json) -> Vec<Port> {
        match self {
            ExecOut::None => Vec::new(),
            ExecOut::Static(p) => p.to_vec(),
            ExecOut::Dynamic(f) => f(config),
        }
    }
}

impl std::fmt::Debug for ExecOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecOut::None => write!(f, "None"),
            ExecOut::Static(p) => write!(f, "Static({} arm(s))", p.len()),
            ExecOut::Dynamic(_) => write!(f, "Dynamic"),
        }
    }
}
