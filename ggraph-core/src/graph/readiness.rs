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

/// Something required that nothing fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub node: u32,
    pub kind: String,
    pub port: PortName,
    /// A setting rather than a port. Worth distinguishing because the remedies differ: a port can
    /// be wired OR typed into, a setting can only be typed into.
    pub is_setting: bool,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_setting {
            write!(
                f,
                "node {} ({}) has no {:?} set",
                self.node, self.kind, self.port
            )
        } else {
            write!(
                f,
                "node {} ({}) needs {:?}",
                self.node, self.kind, self.port
            )
        }
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
                is_setting: false,
            });
        }

        // And the settings. On a node whose input IS a setting — a table reader naming its table,
        // a mailer naming its address — this is the only check there is; it has no data port to
        // leave empty.
        for field in spec.fields.resolve(&n.config) {
            if !field.required || config_literal(&n.config, field.key.as_str()).is_some() {
                continue;
            }
            out.push(Missing {
                node: n.id,
                kind: n.kind.as_str().to_string(),
                port: field.key.clone(),
                is_setting: true,
            });
        }
    }
    out
}

/// Everything wrong with a document, in one answer.
///
/// The two questions were always asked together and never in one place, so each caller reached for
/// whichever it remembered — and a graph could pass `validate` and still be unrunnable, which is
/// exactly what happened to a flow saved with one node and no wires.
///
/// The point of collecting them is a DRY RUN: a document can now be judged without being stored.
/// Before this the only way to find out a graph was wrong was to save it, which left the debris of
/// every attempt behind in a list somebody has to read.
pub struct Report {
    /// Is the document coherent — kinds that exist, wires whose ends and types agree.
    pub problems: Vec<crate::validate::Problem>,
    /// Required ports and settings with nothing on them.
    pub missing: Vec<Missing>,
    /// Does anything connect to anything? With no edges every node is an orphan.
    pub wired: bool,
}

impl Report {
    /// Nothing wrong, and something to run.
    pub fn is_ready(&self) -> bool {
        self.problems.is_empty() && self.missing.is_empty() && self.wired
    }

    /// One line per thing wrong, in the order somebody should fix them: a document that does not
    /// hold together first, because the rest is read against a shape that may not survive.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self.problems.iter().map(|p| p.to_string()).collect();
        if !self.wired {
            out.push(
                "nothing is wired, so nothing would run — a flow is a chain, and every node in                  this one is an orphan"
                    .to_string(),
            );
        }
        out.extend(self.missing.iter().map(|m| m.to_string()));
        out
    }
}

/// Judge a document without storing it.
pub fn inspect<M: GraphMeta, H: Host>(graph: &Graph<M>, reg: &NodeRegistry<H>) -> Report {
    Report {
        problems: crate::validate::validate(graph, reg),
        missing: unfilled(graph, reg),
        wired: wired(graph),
    }
}

/// Does this graph give anything back?
///
/// Separate from [`wired`] and from [`Report`] on purpose: a graph whose work IS the effect — a
/// schedule that sends the mail, a trigger that writes the file — is complete without one, and
/// every graph written before `output` existed is such a graph. It is a REQUIREMENT only where
/// somebody is calling the graph to get an answer, and that caller is the one who should say so.
pub fn answers<M: GraphMeta>(graph: &Graph<M>) -> bool {
    graph.nodes.iter().any(|n| n.kind.as_str() == "output")
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
                is_setting: false,
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

    /// The gap that let a table reader with no table named be called ready by everyone who asked.
    /// On a node whose input IS a setting there is no port to leave empty, so a check that only
    /// looked at ports saw nothing wrong.
    #[test]
    fn a_required_setting_with_nothing_in_it_is_missing() {
        let mut g: Graph = Graph::new("unset");
        let id = g.add_node(NodeId::new("table_schema"), 0, 0);
        let m = unfilled(&g, &reg());
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(
            m[0].is_setting,
            "a setting, not a port — the remedies differ"
        );
        assert_eq!(m[0].port, PortName::new("columns"));

        g.node_mut(id).unwrap().config = json!({ "columns": [{ "name": "n", "type": "text" }] });
        assert_eq!(unfilled(&g, &reg()), Vec::new());
    }

    /// A setting with a working default is not news. Marking those would put a warning on nearly
    /// every graph, which is the same as marking none.
    #[test]
    fn a_setting_that_has_a_default_is_not_required() {
        let mut g: Graph = Graph::new("defaults");
        g.add_node(NodeId::new("report_render"), 0, 0);
        assert!(
            unfilled(&g, &reg()).iter().all(|m| !m.is_setting),
            "title and format both have defaults"
        );
    }

    /// One answer, because the two questions were always asked together: a graph could pass
    /// `validate` and still be unrunnable, and each caller reached for whichever it remembered.
    #[test]
    fn one_report_carries_everything_wrong() {
        let r = reg();
        let mut g: Graph = Graph::new("bad");
        g.add_node(NodeId::new("http_request"), 0, 0);
        let rep = inspect(&g, &r);
        assert!(!rep.wired, "no edges");
        assert_eq!(rep.missing.len(), 1, "the url");
        assert!(!rep.is_ready());
        assert_eq!(rep.lines().len(), 2, "both, in one list: {:?}", rep.lines());
    }

    /// A graph that will run says so, and says it once.
    #[test]
    fn a_finished_document_reports_nothing() {
        let r = reg();
        let mut g: Graph = Graph::new("fine");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        g.node_mut(each).unwrap().config = json!({ "items": "a,b" });
        let say = g.add_node(NodeId::new("print"), 200, 0);
        g.add_edge(&r, each, "loop_body", say, "exec_in").unwrap();
        assert!(inspect(&g, &r).is_ready());
    }

    /// A graph that gives something back is one somebody can call. Not required of every graph —
    /// one whose work is the effect is complete without it — so this is asked, never assumed.
    #[test]
    fn a_graph_says_whether_it_answers() {
        let r = reg();
        let mut g: Graph = Graph::new("effect only");
        let make = g.add_node(NodeId::new("format"), 0, 0);
        let say = g.add_node(NodeId::new("print"), 200, 0);
        g.add_edge(&r, make, "text", say, "message").unwrap();
        assert!(!answers(&g), "printing is an effect, not an answer");

        let end = g.add_node(NodeId::new("output"), 400, 0);
        g.node_mut(end).unwrap().config =
            json!({ "values": [{ "name": "greeting", "type": "text" }] });
        g.add_edge(&r, make, "text", end, "greeting").unwrap();
        assert!(answers(&g));
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
