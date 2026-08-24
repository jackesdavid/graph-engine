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
//! Sentinel had eleven nodes whose real semantics lived inline inside the scheduler because
//! there was no third shape for them. Every one of the eleven ends in the same four moves —
//! outputs, arms, re-enter, log — which is exactly [`Step`].

use crate::graph::PortLookup;
use crate::host::Host;
use crate::id::{NodeId, PortName};
use crate::port::Port;
use crate::value::{PortValues, Value};
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

/// A port set that may depend on the node's configuration.
///
/// The dynamic case is not exotic — a node that emits an event grows one input per field of
/// that event, a switch grows one arm per configured label. Making it part of the declaration
/// is what stops half the callers from reading the static table and getting a different answer
/// than the scheduler.
pub enum Ports {
    Static(&'static [Port]),
    Dynamic(fn(&Json) -> Vec<Port>),
}

impl Ports {
    pub const NONE: Ports = Ports::Static(&[]);

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

/// When a node runs relative to control flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purity {
    /// Has exec pins; runs when control reaches it.
    Effectful,
    /// No exec pins. Runs when something reads it, once per run.
    Pure,
    /// Pure, but re-reads the world: evaluated afresh each time it is read.
    ///
    /// This exists because of a real failure. A pure source with no incoming edges, feeding a
    /// required input inside a sub-run, never ran — the consumer's input resolved to nothing and
    /// the engine skipped the whole branch as dead while reporting the run as `ok`. Silent, and
    /// only on event-driven runs.
    PureSource,
}

impl Purity {
    pub fn has_exec(self) -> bool {
        matches!(self, Purity::Effectful)
    }
}

/// How long a node may take before the engine gives up on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    /// Runs inline. For nodes that cannot block — arithmetic, string formatting.
    Inline,
    Secs(u64),
}

/// What fires after a node runs.
pub enum ExecOut {
    /// Pure nodes.
    None,
    Static(&'static [Port]),
    /// Arms declared by configuration — a switch.
    Dynamic(fn(&Json) -> Vec<Port>),
}

impl ExecOut {
    /// The single "and then" arm most nodes have.
    pub const DEFAULT: ExecOut = ExecOut::Static(&[crate::port::EXEC_OUT]);

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

/// What a node is handed when it runs.
pub struct NodeCx<'a, H: Host> {
    pub config: &'a Json,
    pub inputs: &'a PortValues,
    /// This node's id within the graph.
    pub node: u32,
    pub host: &'a H,
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

/// What a cooperating node hands back to the scheduler.
#[derive(Debug, Default)]
pub struct Step {
    pub outputs: PortValues,
    /// Which exec arms fire.
    pub arms: Vec<PortName>,
    /// Run me again in the next epoch — a loop.
    pub reenter: bool,
    /// A line for the run log.
    pub log: Option<String>,
    /// End the run here without failing it. A node waiting on a person: the answer will arrive
    /// as a fresh entry at this node, in a different run.
    pub halt: bool,
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
        self.reenter = true;
        self
    }

    pub fn logged(mut self, msg: impl Into<String>) -> Self {
        self.log = Some(msg.into());
        self
    }

    pub fn halted(mut self) -> Self {
        self.halt = true;
        self
    }
}

/// What a cooperating node is handed. Everything [`NodeCx`] has, plus the scheduler's view.
pub struct StepCx<'a, H: Host> {
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
#[derive(Debug)]
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
    pub default_config: fn() -> Json,
    pub purity: Purity,
    pub timeout: Timeout,
    pub behavior: Behavior<H>,
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
            default_config: || Json::Object(Default::default()),
            purity: Purity::Effectful,
            timeout: Timeout::Secs(30),
            behavior: Behavior::Inert,
        }
    }

    /// A node with no exec pins: it is read, not reached.
    pub fn pure(id: &'static str, label: &'static str, category: &'static str) -> Self {
        NodeSpec {
            exec_out: ExecOut::None,
            purity: Purity::Pure,
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
    pub fn with_config(mut self, f: fn() -> Json) -> Self {
        self.default_config = f;
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
