//! What a node is handed when it runs.
//!
//! Deliberately narrow. A node sees its own configuration, its own resolved inputs, its own id,
//! and the host. It cannot see the graph, the other nodes, or what ran before it — which is the
//! isolation the whole design rests on, and the reason a node can be tested by calling it.
//!
//! [`StepCx`] is the wider one, for a node that cooperates with the scheduler: it also reaches
//! the run's variables and its own scratch space across epochs.

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

/// What a node is handed when it runs.
pub struct NodeCx<'a, H: Host> {
    pub config: &'a Json,
    pub inputs: &'a PortValues,
    /// This node's id within the graph.
    pub node: u32,
    pub host: &'a H,
    /// Run-scoped named values, shared by every node in this run.
    ///
    /// Owned by the run rather than by the host, because that is what they are: working state
    /// that dies when the run does. A graph wanting a value to outlive its run wants a table,
    /// and that difference should be visible in the palette rather than hidden in a lifetime.
    pub vars: Vars,
}

impl<H: Host> NodeCx<'_, H> {
    pub fn input(&self, name: &str) -> Option<&Value> {
        self.inputs.get(name)
    }

    /// An input that must be there. The error names the port, because "missing input" without
    /// the name is a support ticket.
    pub fn require(&self, name: &str) -> Result<&Value, NodeError> {
        self.input(name)
            .ok_or_else(|| NodeError::new(format!("missing required input {name:?}")))
    }

    /// A configuration string.
    pub fn cfg_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Json::as_str)
    }

    /// An input, falling back to the config literal of the same name. This is how a port that
    /// is not wired takes its value from the inspector.
    pub fn input_or_cfg(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.input(name) {
            return Some(v.clone());
        }
        match self.config.get(name)? {
            Json::String(s) if s.is_empty() => None,
            Json::String(s) => Some(Value::text(s.clone())),
            Json::Bool(b) => Some(Value::Bool(*b)),
            Json::Number(n) => n
                .as_i64()
                .map(Value::int)
                .or_else(|| n.as_f64().map(Value::float)),
            other => Some(Value::Json(other.clone())),
        }
    }
}

impl<H: Host> std::fmt::Debug for NodeCx<'_, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeCx {{ node: {} }}", self.node)
    }
}

/// A node that computes.
pub trait NodeRun<H: Host>: Send + Sync {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError>;

    /// One line for the run log. The default reports the outputs; override when the node knows
    /// something more useful to say.
    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        if out.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = out
            .iter()
            .map(|(k, v)| format!("{k}={}", v.summary()))
            .collect();
        parts.sort();
        parts.join(" ")
    }
}

/// A node that computes and then decides where control goes.
pub trait NodeRoute<H: Host>: NodeRun<H> {
    /// Which exec arms fire. Empty means control stops here.
    fn arms(&self, cx: &NodeCx<'_, H>, out: &PortValues) -> Vec<PortName>;
}
