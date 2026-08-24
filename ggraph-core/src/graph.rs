//! The graph document: nodes, edges, and the rules for wiring them.
//!
//! ## Why `Graph<M>`
//!
//! A graph document carries two very different things. Some of it is topology — nodes, edges,
//! ids — and is the same in every product. The rest is *policy*: what triggers this graph, how
//! many may run at once, what a run is one *of*. That half is not shared, and the moment it
//! lives in a shared struct the shared struct knows about cameras.
//!
//! So the policy is a type parameter, spliced back into the same JSON object with
//! `#[serde(flatten)]`. The document on disk is byte-identical to one with the fields inline,
//! the typing stays compile-time, and each product declares its own.
//!
//! ## Isolation
//!
//! **A node never names another node.** Nodes declare typed ports; edges map an output port to
//! an input port; the engine resolves the wiring. That is what lets a node be tested with a map
//! of inputs and nothing else, and what lets the editor rewire a graph without any node knowing.

use crate::id::{NodeId, PortName};
use crate::port::{compatible, Port};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A product's per-graph policy. `()` for a product that has none.
pub trait GraphMeta:
    Serialize + DeserializeOwned + Clone + Default + Send + Sync + 'static
{
}
impl<T: Serialize + DeserializeOwned + Clone + Default + Send + Sync + 'static> GraphMeta for T {}

/// How ports are resolved for a node. Implemented by the registry.
///
/// Edge validation has to ask, because ports are not always static: a node that emits an event
/// grows one input per field of that event, a switch grows one exec arm per configured label.
/// A validator that reads only the static table gives a different answer than the engine does —
/// which is a real bug, not a hypothetical one: Sentinel's catalog endpoint and its scheduler
/// disagreed about exactly those nodes.
pub trait PortLookup {
    fn inputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>>;
    fn outputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>>;
    /// Exec arms. Empty for a pure node.
    fn exec_outs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>>;
    fn has_exec_in(&self, kind: &NodeId) -> bool;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: u32,
    pub kind: NodeId,
    pub x: i32,
    pub y: i32,
    #[serde(default = "empty_object")]
    pub config: Json,
    /// Run once per run and reuse the result, even if reached again through a loop.
    #[serde(default)]
    pub memoize: bool,
}

fn empty_object() -> Json {
    json!({})
}

impl GraphNode {
    pub fn new(id: u32, kind: NodeId, x: i32, y: i32) -> Self {
        GraphNode {
            id,
            kind,
            x,
            y,
            config: json!({}),
            memoize: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: u32,
    pub from_port: PortName,
    pub to: u32,
    pub to_port: PortName,
}

/// Where the canvas was left. View state, not behaviour — but it belongs to the document,
/// because reopening a graph somewhere else and finding it scrolled to the origin is a bug.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// The blanket `impl<T: ..> GraphMeta for T` leaves serde unable to pick a path to
// `M: Deserialize`; naming the bound resolves it. It is also the honest bound: `M` is
// deserialized from the same object, so it must own its data.
#[serde(bound(
    serialize = "M: Serialize",
    deserialize = "M: serde::de::DeserializeOwned"
))]
pub struct Graph<M: GraphMeta = ()> {
    pub id: Uuid,
    pub name: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
    /// The next node id to hand out. Node ids are never reused: a stale reference must fail to
    /// resolve rather than silently point at a different node.
    pub next_id: u32,
    #[serde(default)]
    pub viewport: Viewport,
    /// Hand-routed waypoints per edge, for readability on a busy canvas. Pure geometry.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub edge_vias: HashMap<String, Vec<[i32; 2]>>,
    #[serde(flatten)]
    pub meta: M,
}

/// Why an edge was refused. Typed rather than a string so a caller can react to the reason —
/// the editor greys out an incompatible pin instead of letting the drop fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    SelfLoop,
    UnknownNode(u32),
    UnknownKind(NodeId),
    UnknownPort { node: u32, port: PortName },
    TypeMismatch { from: String, to: String },
    InputAlreadyWired(PortName),
    WouldCycle,
    Duplicate,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::SelfLoop => write!(f, "a node cannot link to itself"),
            WireError::UnknownNode(id) => write!(f, "no node {id} in this graph"),
            WireError::UnknownKind(k) => write!(f, "unknown node kind {k:?}"),
            WireError::UnknownPort { node, port } => {
                write!(f, "node {node} has no port {:?}", port.as_str())
            }
            WireError::TypeMismatch { from, to } => {
                write!(f, "cannot wire {from} into {to}")
            }
            WireError::InputAlreadyWired(p) => {
                write!(f, "input {:?} is already wired", p.as_str())
            }
            WireError::WouldCycle => write!(f, "that link would create a data cycle"),
            WireError::Duplicate => write!(f, "those pins are already linked"),
        }
    }
}

impl std::error::Error for WireError {}

impl<M: GraphMeta> Graph<M> {
    pub fn new(name: impl Into<String>) -> Self {
        Graph {
            id: Uuid::new_v4(),
            name: name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            next_id: 1,
            viewport: Viewport::default(),
            edge_vias: HashMap::new(),
            meta: M::default(),
        }
    }

    pub fn node(&self, id: u32) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn add_node(&mut self, kind: NodeId, x: i32, y: i32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(GraphNode::new(id, kind, x, y));
        id
    }

    /// Remove a node and every edge touching it. An edge to a node that is gone is not a
    /// recoverable state, so it is never left behind to be discovered at run time.
    pub fn remove_node(&mut self, id: u32) {
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
    }

    /// Wire an output port to an input port, refusing anything that could not run.
    ///
    /// The order of the checks matters for the message the author sees: identity, then
    /// existence, then type, then arity, then acyclicity. Reporting "would create a cycle" for
    /// a pin that does not exist sends people looking in the wrong place.
    pub fn add_edge(
        &mut self,
        reg: &dyn PortLookup,
        from: u32,
        from_port: &str,
        to: u32,
        to_port: &str,
    ) -> Result<(), WireError> {
        if from == to {
            return Err(WireError::SelfLoop);
        }
        let from_port = PortName::new(from_port);
        let to_port = PortName::new(to_port);

        let out = self.resolve_out(reg, from, &from_port)?;
        let inp = self.resolve_in(reg, to, &to_port)?;

        if !compatible(&out.ty, &inp.ty) {
            return Err(WireError::TypeMismatch {
                from: out.ty.to_string(),
                to: inp.ty.to_string(),
            });
        }

        let is_exec = inp.ty.is_exec();

        if self.edges.iter().any(|e| {
            e.from == from && e.from_port == from_port && e.to == to && e.to_port == to_port
        }) {
            return Err(WireError::Duplicate);
        }

        // A data input takes exactly one source — two would make the value depend on evaluation
        // order. An exec input takes many: convergence is how branches rejoin.
        if !is_exec
            && self
                .edges
                .iter()
                .any(|e| e.to == to && e.to_port == to_port)
        {
            return Err(WireError::InputAlreadyWired(to_port));
        }

        // Only exec may close a loop. A data cycle has no defined value; an exec cycle is a
        // loop, which is a feature — see `back_edges`.
        if !is_exec && self.reaches(to, from) {
            return Err(WireError::WouldCycle);
        }

        self.edges.push(Edge {
            from,
            from_port,
            to,
            to_port,
        });
        Ok(())
    }

    fn resolve_out(
        &self,
        reg: &dyn PortLookup,
        id: u32,
        port: &PortName,
    ) -> Result<Port, WireError> {
        let n = self.node(id).ok_or(WireError::UnknownNode(id))?;
        let outs = reg
            .outputs(&n.kind, &n.config)
            .ok_or_else(|| WireError::UnknownKind(n.kind.clone()))?;
        let execs = reg.exec_outs(&n.kind, &n.config).unwrap_or_default();
        outs.into_iter()
            .chain(execs)
            .find(|p| p.name == *port)
            .ok_or_else(|| WireError::UnknownPort {
                node: id,
                port: port.clone(),
            })
    }

    fn resolve_in(
        &self,
        reg: &dyn PortLookup,
        id: u32,
        port: &PortName,
    ) -> Result<Port, WireError> {
        let n = self.node(id).ok_or(WireError::UnknownNode(id))?;
        if reg.has_exec_in(&n.kind) && *port == crate::port::EXEC_IN.name {
            return Ok(crate::port::EXEC_IN);
        }
        let ins = reg
            .inputs(&n.kind, &n.config)
            .ok_or_else(|| WireError::UnknownKind(n.kind.clone()))?;
        ins.into_iter()
            .find(|p| p.name == *port)
            .ok_or_else(|| WireError::UnknownPort {
                node: id,
                port: port.clone(),
            })
    }

    /// Is `to` reachable from `from` along any edge?
    pub fn reaches(&self, from: u32, to: u32) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if !seen.insert(n) {
                continue;
            }
            stack.extend(self.edges.iter().filter(|e| e.from == n).map(|e| e.to));
        }
        false
    }

    /// The nodes `id` feeds, deduplicated.
    pub fn children(&self, id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for e in self.edges.iter().filter(|e| e.from == id) {
            if !out.contains(&e.to) {
                out.push(e.to);
            }
        }
        out
    }

    /// Which node feeds `port` on `id`, if any.
    pub fn wired_source(&self, id: u32, port: &str) -> Option<u32> {
        let port = PortName::new(port);
        self.edges
            .iter()
            .find(|e| e.to == id && e.to_port == port)
            .map(|e| e.from)
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    //! A `PortLookup` for tests, so topology can be tested without a registry.
    use super::*;
    use crate::port::PortType;

    pub struct Fake;

    /// `a`,`b` in / `result` out on any kind; plus exec pins and a `true`/`false` pair so a
    /// branch can be wired.
    impl PortLookup for Fake {
        fn inputs(&self, _k: &NodeId, _c: &Json) -> Option<Vec<Port>> {
            Some(vec![
                Port::opt("a", PortType::NUM),
                Port::opt("b", PortType::NUM),
                Port::req("condition", PortType::BOOL),
                Port::opt("text", PortType::TEXT),
            ])
        }
        fn outputs(&self, _k: &NodeId, _c: &Json) -> Option<Vec<Port>> {
            Some(vec![
                Port::opt("result", PortType::BOOL),
                Port::opt("value", PortType::NUM),
                Port::opt("text", PortType::TEXT),
            ])
        }
        fn exec_outs(&self, _k: &NodeId, _c: &Json) -> Option<Vec<Port>> {
            Some(vec![
                crate::port::EXEC_OUT,
                Port::opt("true", PortType::EXEC),
                Port::opt("false", PortType::EXEC),
            ])
        }
        fn has_exec_in(&self, _k: &NodeId) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::Fake;
    use super::*;

    fn g() -> Graph {
        let mut g: Graph = Graph::new("t");
        g.id = Uuid::from_bytes([7; 16]);
        for _ in 0..4 {
            g.add_node(NodeId::new_static("k"), 0, 0);
        }
        g
    }

    #[test]
    fn a_node_cannot_wire_to_itself() {
        let mut g = g();
        assert_eq!(
            g.add_edge(&Fake, 1, "result", 1, "condition"),
            Err(WireError::SelfLoop)
        );
    }

    #[test]
    fn an_unknown_port_names_the_port() {
        let mut g = g();
        assert_eq!(
            g.add_edge(&Fake, 1, "nope", 2, "condition"),
            Err(WireError::UnknownPort {
                node: 1,
                port: PortName::new("nope")
            })
        );
    }

    #[test]
    fn incompatible_types_do_not_wire() {
        let mut g = g();
        // `value` is num, `condition` is bool.
        assert!(matches!(
            g.add_edge(&Fake, 1, "value", 2, "condition"),
            Err(WireError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn a_data_input_takes_one_source_but_exec_takes_many() {
        let mut g = g();
        g.add_edge(&Fake, 1, "result", 3, "condition").unwrap();
        assert_eq!(
            g.add_edge(&Fake, 2, "result", 3, "condition"),
            Err(WireError::InputAlreadyWired(PortName::new("condition")))
        );
        // exec_in is the exception, and it is the exception on purpose: convergence.
        g.add_edge(&Fake, 1, "exec_out", 4, "exec_in").unwrap();
        g.add_edge(&Fake, 2, "exec_out", 4, "exec_in").unwrap();
    }

    #[test]
    fn a_data_cycle_is_refused_and_an_exec_cycle_is_not() {
        let mut g = g();
        g.add_edge(&Fake, 1, "value", 2, "a").unwrap();
        g.add_edge(&Fake, 2, "value", 3, "a").unwrap();
        assert_eq!(
            g.add_edge(&Fake, 3, "value", 1, "a"),
            Err(WireError::WouldCycle)
        );
        // The same shape in exec is a loop, which is allowed.
        g.add_edge(&Fake, 1, "exec_out", 2, "exec_in").unwrap();
        g.add_edge(&Fake, 2, "exec_out", 3, "exec_in").unwrap();
        g.add_edge(&Fake, 3, "exec_out", 1, "exec_in").unwrap();
    }

    #[test]
    fn the_same_pins_do_not_link_twice() {
        let mut g = g();
        g.add_edge(&Fake, 1, "exec_out", 2, "exec_in").unwrap();
        assert_eq!(
            g.add_edge(&Fake, 1, "exec_out", 2, "exec_in"),
            Err(WireError::Duplicate)
        );
    }

    #[test]
    fn removing_a_node_takes_its_edges_with_it() {
        let mut g = g();
        g.add_edge(&Fake, 1, "exec_out", 2, "exec_in").unwrap();
        g.add_edge(&Fake, 2, "exec_out", 3, "exec_in").unwrap();
        g.remove_node(2);
        assert!(g.edges.is_empty(), "a dangling edge is never left behind");
    }

    #[test]
    fn meta_flattens_into_the_same_object() {
        #[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
        struct Meta {
            enabled: bool,
            scope: String,
        }
        let mut g: Graph<Meta> = Graph::new("t");
        g.id = Uuid::from_bytes([1; 16]);
        g.meta = Meta {
            enabled: true,
            scope: "per_camera".into(),
        };
        let v: Json = serde_json::to_value(&g).unwrap();
        assert_eq!(
            v.get("enabled"),
            Some(&json!(true)),
            "product policy must sit at the top level, not nested under `meta` — that is what \
             keeps stored documents readable by the version that had the fields inline"
        );
        assert_eq!(v.get("scope"), Some(&json!("per_camera")));
        let back: Graph<Meta> = serde_json::from_value(v).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn node_ids_are_never_reused() {
        let mut g = g();
        g.remove_node(4);
        let fresh = g.add_node(NodeId::new_static("k"), 0, 0);
        assert_eq!(fresh, 5, "reusing 4 would make a stale reference resolve");
    }
}
