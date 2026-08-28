// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Is this graph finished?
//!
//! [`validate`](super::validate) asks whether a document is coherent — kinds that exist, wires
//! whose ends exist and whose types agree. This asks the other question, the one a list of graphs
//! wants: press Run, and does anything happen?
//!
//! Two different answers, and they are separate because their inputs are. Coherence is a property
//! of the document and a registry. Readiness also depends on what the host reads out of a node's
//! configuration, which is a capability rather than a fact.
//!
//! # Why this is not in the editor
//!
//! It was, in both products, as a walk over `nodes` deciding per port type what counts as a typed
//! value. That is why every list had to ship every node and every edge of every graph: the badge
//! saying "this one is not ready" could not be drawn without the whole document. Answering here
//! lets a list carry names.
//!
//! It is also the more correct answer. An editor checks against the catalogue's DEFAULT ports; a
//! node whose ports depend on its configuration is checked here against the ports it actually has.

use crate::graph::{Graph, GraphMeta};
use crate::host::{Host, Literals, NoLiterals};
use crate::id::PortName;
use crate::port::PortType;
use crate::registry::NodeRegistry;
use crate::spec::config_literal;

/// A required input with neither a wire nor a value in the inspector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub node: u32,
    pub kind: String,
    pub port: PortName,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "node {} ({}) needs {:?}", self.node, self.kind, self.port)
    }
}

/// Every required input nothing fills, for a host that reads no literals of its own.
///
/// The common case: nodes that take their unwired inputs from their own configuration through
/// [`NodeCx::input_or_cfg`](crate::spec::NodeCx::input_or_cfg).
pub fn unfilled<M: GraphMeta, H: Host>(graph: &Graph<M>, reg: &NodeRegistry<H>) -> Vec<Missing> {
    unfilled_with(graph, reg, &NoLiterals)
}

/// The same question for a host that interprets configuration itself.
///
/// Both are consulted, never one: a host's [`Literals`] answers for the ports it knows about, and
/// the plain rule answers for the rest. Asking only the host would call filled ports empty in
/// every product that does not implement one.
pub fn unfilled_with<M: GraphMeta, H: Host>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    literals: &dyn Literals,
) -> Vec<Missing> {
    let mut out = Vec::new();
    for n in &graph.nodes {
        // A kind this build does not register is `validate`'s news to break, and reporting every
        // port of it here would bury the one line that matters.
        let Some(spec) = reg.get(&n.kind) else {
            continue;
        };
        for port in spec.inputs.resolve(&n.config) {
            if !port.required || port.ty == PortType::EXEC {
                continue;
            }
            let wired = graph
                .edges
                .iter()
                .any(|e| e.to == n.id && e.to_port == port.name);
            if wired
                || literals.read(&n.kind, &port, &n.config).is_some()
                || config_literal(&n.config, port.name.as_str()).is_some()
            {
                continue;
            }
            out.push(Missing {
                node: n.id,
                kind: n.kind.as_str().to_string(),
                port: port.name.clone(),
            });
        }
    }
    out
}

/// Does this document describe anything to run?
///
/// One wired node is the bar. With no edges every node is an orphan, and whether orphans run is a
/// product's decision recorded in its own metadata — so a product ORs this with its own flag
/// rather than the engine guessing at a field name.
pub fn wired<M: GraphMeta>(graph: &Graph<M>) -> bool {
    !graph.edges.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::nodes::services::Services;
    use crate::NodeId;
    use serde_json::json;

    fn reg() -> NodeRegistry<TestHost> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &Services::none());
        r
    }

    /// The whole point: a required port with nothing on it is what makes a graph un-runnable, and
    /// saying so no longer needs the document in the reader's hands.
    #[test]
    fn a_required_port_with_nothing_on_it_is_missing() {
        let mut g: Graph = Graph::new("empty");
        let id = g.add_node(NodeId::new("http_request"), 0, 0);
        assert_eq!(
            unfilled(&g, &reg()),
            vec![Missing {
                node: id,
                kind: "http_request".into(),
                port: PortName::new("url"),
            }]
        );
    }

    /// Typed into the inspector counts. In most editors that is the ORDINARY way to fill a port,
    /// and calling those graphs unfinished would put a warning on nearly all of them.
    #[test]
    fn a_value_in_the_inspector_fills_it() {
        let mut g: Graph = Graph::new("typed");
        let id = g.add_node(NodeId::new("http_request"), 0, 0);
        g.node_mut(id).unwrap().config = json!({ "url": "https://example.test" });
        assert_eq!(unfilled(&g, &reg()), Vec::new());
    }

    /// A field somebody clicked into and left is empty, whatever whitespace it holds.
    #[test]
    fn a_blank_field_does_not_fill_it() {
        let mut g: Graph = Graph::new("blank");
        let id = g.add_node(NodeId::new("http_request"), 0, 0);
        g.node_mut(id).unwrap().config = json!({ "url": "   " });
        assert_eq!(unfilled(&g, &reg()).len(), 1);
    }

    /// And a wire fills it — the case the inspector rule must not shadow.
    #[test]
    fn a_wire_fills_it() {
        let r = reg();
        let mut g: Graph = Graph::new("wired");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        let req = g.add_node(NodeId::new("http_request"), 200, 0);
        g.add_edge(&r, each, "item", req, "url").unwrap();
        assert_eq!(unfilled(&g, &r), Vec::new());
    }

    /// Optional ports are not news. `print` takes a message it is happy without, and reporting it
    /// would put a warning on a graph that runs perfectly.
    #[test]
    fn optional_ports_are_not_reported() {
        let mut g: Graph = Graph::new("optional");
        g.add_node(NodeId::new("print"), 0, 0);
        assert_eq!(unfilled(&g, &reg()), Vec::new());
    }

    /// A kind this build does not register belongs to `validate`. Listing every port of it here
    /// would bury the one line that says why.
    #[test]
    fn an_unknown_kind_is_not_reported_here() {
        let mut g: Graph = Graph::new("stale");
        g.add_node(NodeId::new("a_node_from_a_newer_deploy"), 0, 0);
        assert_eq!(unfilled(&g, &reg()), Vec::new());
    }

    /// No edges means every node is an orphan, and a run does nothing.
    #[test]
    fn a_document_with_no_wires_is_not_wired() {
        let r = reg();
        let mut g: Graph = Graph::new("loose");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        let say = g.add_node(NodeId::new("print"), 200, 0);
        assert!(!wired(&g));
        g.add_edge(&r, each, "loop_body", say, "exec_in").unwrap();
        assert!(wired(&g));
    }
}
