// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a cooperating node hands back.
//!
//! Most nodes just return outputs. A few need to say something about CONTROL: which arms fire,
//! whether to run again on the next epoch, or whether the run should stop here and wait for the
//! world to answer.
//!
//! Those four moves — outputs, arms, re-enter, log — are all that the eleven engine-driven blocks
//! of a real product's scheduler ever did. Naming them is what let those blocks become ordinary
//! nodes instead of special cases inside the loop.

/// Builds a port list from a node's configuration.
pub type PortsFn = Arc<dyn Fn(&Json) -> Vec<Port> + Send + Sync>;

/// Builds a node kind's default configuration.
pub type ConfigFn = Arc<dyn Fn() -> Json + Send + Sync>;
use super::*;
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::value::{PortValues, Value};

/// Run-scoped named values, shared across the nodes of one run.
pub type Vars = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<PortName, Value>>>;
use serde_json::Value as Json;
use std::sync::Arc;

/// What a cooperating node hands back to the scheduler.
#[derive(Debug, Default)]
pub struct Step {
    pub outputs: PortValues,
    /// Which exec arms fire.
    pub arms: Vec<PortName>,
    /// What happens to the run after this node.
    pub next: Next,
    /// A line for the run log.
    pub log: Option<String>,
}

/// What a cooperating node asks the run to do next.
///
/// One value rather than two booleans, because two booleans have four states and only three of
/// them mean anything. "Run me again next epoch AND end the run here" had no reading, nothing
/// stopped a node saying it, and the scheduler would have picked one silently — the twentieth
/// node somebody writes is where that happens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Next {
    /// Control flows on down the arms that fired. The ordinary case.
    #[default]
    Onward,
    /// Reach me again in the next epoch. A loop asking for its next pass.
    Reenter,
    /// End the run here, without failing it. A node waiting on a person or a timer: the answer
    /// arrives as a fresh entry at this node, in a different run.
    Halt,
}

impl Step {
    pub fn outputs(outputs: PortValues) -> Self {
        Step {
            outputs,
            ..Step::default()
        }
    }

    pub fn arm(mut self, name: &str) -> Self {
        self.arms.push(PortName::new(name));
        self
    }

    pub fn reentering(mut self) -> Self {
        self.next = Next::Reenter;
        self
    }

    pub fn logged(mut self, msg: impl Into<String>) -> Self {
        self.log = Some(msg.into());
        self
    }

    pub fn halted(mut self) -> Self {
        self.next = Next::Halt;
        self
    }
}

/// What a cooperating node is handed. Everything [`NodeCx`] has, plus the scheduler's view.
pub struct StepCx<'a, H: Host> {
    /// Run-scoped named values. See [`NodeCx::vars`].
    pub vars: Vars,
    pub config: &'a Json,
    pub inputs: &'a PortValues,
    pub node: u32,
    pub graph: uuid::Uuid,
    pub instance: &'a str,
    /// This node is the entry point of the current pass, rather than being reached through
    /// control flow. How a resumption is distinguished from an ordinary run.
    pub forced: bool,
    /// What the entry carried. For a resumption, the answer being delivered.
    pub entry_payload: &'a PortValues,
    pub host: &'a H,
    /// Run-scoped scratch, private to this node. A loop's index lives here.
    pub scratch: &'a mut Json,
}

impl<H: Host> StepCx<'_, H> {
    pub fn input(&self, name: &str) -> Option<&Value> {
        self.inputs.get(name)
    }

    pub fn cfg_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Json::as_str)
    }

    pub fn target(&self) -> crate::host::NodeTarget {
        crate::host::NodeTarget {
            graph: self.graph,
            node: self.node,
            instance: smol_str::SmolStr::new(self.instance),
        }
    }

    pub fn state_key(&self, slot: crate::host::Slot) -> crate::host::StateKey {
        crate::host::StateKey {
            target: self.target(),
            slot,
        }
    }
}

impl<H: Host> std::fmt::Debug for StepCx<'_, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StepCx {{ node: {}, forced: {} }}",
            self.node, self.forced
        )
    }
}

/// A node that cooperates with the scheduler.
pub trait NodeStep<H: Host>: Send + Sync {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError>;
}
