// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What a run is asked for, and how it can fail.
//!
//! Everything here is decided BEFORE a run starts: where it begins, how much work it may do, and
//! whether anything is written down as it goes. Nothing in this file executes anything.
//!
//! [`Entry`] is the interesting one. An empty `at` means "start wherever control can start" — a
//! fresh run. A non-empty one is a RESUMPTION: an answer arriving at the node that asked for it, a
//! timer firing at the node that set it. `restore` is a separate question from the checkpoint
//! policy on purpose; conflating them made a whole product's policy inexpressible.
use crate::value::PortValues;
use std::collections::HashMap;

/// Why a run stopped early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// A node named in the document is not registered. Named, because "unknown node kind" with
    /// no name sends people reading the whole graph.
    ///
    /// Never worth retrying: the build will not learn the kind by being asked again. Reach for
    /// [`validate`](crate::validate) to find these before a run rather than during one.
    UnknownKind { node: u32, kind: String },
    /// A node refused. Carries the node's own judgement about whether trying again could help,
    /// so a durable host does not have to match on the message to decide.
    Node {
        node: u32,
        kind: String,
        message: String,
        retry: crate::host::Retry,
    },
    /// The step ceiling was reached — almost always a loop with no exit.
    ///
    /// Never worth retrying: the same graph will reach the same ceiling.
    Budget { limit: u32 },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::UnknownKind { node, kind } => {
                write!(f, "node {node}: no registered kind {kind:?}")
            }
            RunError::Node {
                node,
                kind,
                message,
                ..
            } => {
                write!(f, "node {node} ({kind}): {message}")
            }
            RunError::Budget { limit } => {
                write!(
                    f,
                    "stopped after {limit} node executions — a loop with no exit?"
                )
            }
        }
    }
}

impl RunError {
    /// Whether running this again could produce a different answer.
    ///
    /// On the error type rather than only on the `Node` variant, so a durable host asks the
    /// question instead of inferring it from which variant it got. Inferring is exactly what the
    /// judgement was added to stop, and a host that has to know which variants are permanent
    /// knows something about the engine's internals that will change without telling it.
    pub fn retry(&self) -> crate::host::Retry {
        match self {
            // An unregistered kind and a runaway loop are both properties of the document. It
            // will be the same document next time.
            RunError::UnknownKind { .. } | RunError::Budget { .. } => crate::host::Retry::Never,
            RunError::Node { retry, .. } => *retry,
        }
    }
}

impl std::error::Error for RunError {}

/// How much a run may do before the engine assumes it will never finish.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub max_steps: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { max_steps: 10_000 }
    }
}

/// When a run's progress is written down.
///
/// The two schedulers the plan called for — one for continuous dataflow, one for durable task
/// runs — turned out to be one scheduler and this enum. What actually differs between them is
/// not how nodes are ordered but how often the run is committed, and that is a policy the host
/// chooses rather than a second implementation to keep in step with the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Checkpoint {
    /// Nothing is written while the run is in flight.
    ///
    /// Right for runs measured in milliseconds, where a lost run is simply the next frame's
    /// problem, and where a write per node would cost more than the work.
    #[default]
    None,

    /// Each node's outputs are written as it produces them, and a resumption reads them back.
    ///
    /// This is what makes a run survive the process it started in. A node that already ran is
    /// restored rather than re-executed, which is the difference between resuming a workflow
    /// and running it again — and for a workflow that sends mail, running it again is not a
    /// recoverable mistake.
    ///
    /// The checkpoints are cleared when the run finishes, so what is on disk is always either a
    /// run in flight or nothing.
    EveryNode,
}

/// How to run.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    pub budget: Budget,
    pub checkpoint: Checkpoint,
    /// What to do with a node that is wired to nothing at all.
    pub isolated: Isolated,
}

/// Whether a node with no edges whatsoever takes part in a run.
///
/// It has no incoming exec edge, so it looks like somewhere control can start — and by that
/// reading it runs. But a canvas collects leftovers: a node dropped while trying something out,
/// unwired, forgotten. Running those is how a graph sends a notification nobody asked for.
///
/// Having no edges is not the same as having no INCOMING edges: a node feeding another is wired,
/// and a node fed by another is wired. This is only about the ones connected to nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Isolated {
    /// Leave them out. The safe default — an unwired node did not ask to run.
    #[default]
    Skip,
    /// Run them. For a graph that is a bag of independent nodes on purpose.
    Run,
}

impl RunOptions {
    /// Every node committed as it completes — a run that survives a restart.
    pub fn durable() -> Self {
        RunOptions {
            checkpoint: Checkpoint::EveryNode,
            ..Self::default()
        }
    }
}

/// Where a run begins.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// Start at these nodes specifically. Empty means every node with no incoming exec edge.
    ///
    /// A non-empty set is a **resumption**: an answer arriving at the node that asked for it, a
    /// timer firing at the node that set it.
    pub at: Vec<u32>,
    /// What the entry carries — the answer, the event payload.
    pub payload: PortValues,
    /// Read back what earlier runs of this instance produced, for every node this one does not
    /// run itself.
    ///
    /// Separate from the checkpoint policy on purpose, because they are separate questions that
    /// were conflated. Writing checkpoints is about surviving the process a run started in;
    /// restoring is about a run that resumes something an earlier one left open — a window closing
    /// hours later, on a timer, with no payload of its own, needing the values the arming run saw.
    /// A product can want the second without the first, and while these were one flag it could
    /// not say so.
    pub restore: bool,
}

/// What a finished run produced, per node.
pub type Outputs = HashMap<u32, PortValues>;
