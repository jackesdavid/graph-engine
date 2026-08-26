// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Durable per-node state: where it lives, and the compare-and-set operations that stop two
//! pods from both deciding they won.

use serde_json::Value as Json;
use smol_str::SmolStr;
use uuid::Uuid;

/// Which node of which graph, in which instance. The address a durable wake-up is delivered to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeTarget {
    pub graph: Uuid,
    pub node: u32,
    /// Which instance of the graph this belongs to. `""` is the single-instance case, and is
    /// what a graph that has never heard of instances stores.
    pub instance: SmolStr,
}

/// Which run. Distinct from [`NodeTarget`]: a target may be re-entered by many runs over time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub graph: Uuid,
    pub run: Uuid,
    pub instance: SmolStr,
}

/// Which slot of a node's durable state is being addressed.
///
/// Three, because they have three different lifetimes and merging them was a bug waiting to
/// happen: `State` is the node's own machine (armed, cooling down), `Values` is what it last
/// produced, `Variables` is the run's variable map at a suspension point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    State,
    Values,
    Variables,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateKey {
    pub target: NodeTarget,
    pub slot: Slot,
}

/// Durable per-node state.
///
/// The three conditional operations are not conveniences — they are the whole point. Two pods
/// run the same graph; `try_arm` is how exactly one of them opens a window, and
/// `try_disarm_expired` is how exactly one of them closes it. A `get` followed by a `set` is a
/// race that shows up as a duplicated notification once a month and is never reproducible.
pub trait StateStore: Send + Sync {
    fn get(&self, key: &StateKey) -> Option<Json>;
    fn set(&self, key: &StateKey, value: &Json);
    fn clear(&self, key: &StateKey);

    /// Move to armed, only if not already armed. `true` means this caller won.
    ///
    /// Deadlines are epoch seconds rather than a formatted timestamp: a string deadline means
    /// two components have to agree on a format and a zone, and the day they disagree the window
    /// closes at the wrong time with nothing in the logs to say why.
    fn try_arm(&self, key: &StateKey, expires_at: i64) -> bool;

    /// Push an armed window's deadline out. A no-op if not armed.
    fn extend(&self, key: &StateKey, expires_at: i64);

    /// Move an armed-and-expired window to idle. `true` means this caller won.
    ///
    /// `now` comes from the engine's clock. A store whose backend has an authoritative clock —
    /// a database doing this as one conditional UPDATE — should prefer its own, since that is
    /// the only one every pod agrees on.
    fn try_disarm_expired(&self, key: &StateKey, now: i64) -> bool;
}
