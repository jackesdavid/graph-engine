// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Is this document runnable, and if not, what is wrong with all of it?
//!
//! [`Graph::add_edge`] checks one wire as it is drawn. This checks a document that already
//! exists — one loaded from a database, one arriving from an older deploy, one about to be
//! migrated — against a registry that may no longer look the way it did when the document was
//! saved.
//!
//! Without it the first thing that notices a node kind this build no longer registers is a run,
//! at whatever hour that graph's trigger fires, in front of whoever owns it. That is a bad time
//! and a bad audience for the news.
//!
//! ## Everything, not the first thing
//!
//! [`validate`] returns every problem it finds. "Three unknown kinds and a wire whose port no
//! longer exists" is something a person can act on in one pass; "unknown kind at node 7" is an
//! invitation to fix it, run again, and discover node 12.

use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::port::EXEC_IN;
use crate::registry::NodeRegistry;

/// One thing wrong with a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The document names a kind this build does not register. Usually an older graph meeting a
    /// newer deploy, or the reverse.
    UnknownKind { node: u32, kind: String },

    /// An edge names a node that is not in the document.
    DanglingEdge { from: u32, to: u32, missing: u32 },

    /// The port existed when the wire was drawn and does not now — a node whose ports depend on
    /// its configuration, reconfigured without rewiring.
    UnknownPort {
        node: u32,
        kind: String,
        port: PortName,
        /// `true` for the target end of the wire.
        is_input: bool,
    },

    /// Both ends exist, and the types no longer permit the connection.
    TypeMismatch {
        from: u32,
        from_port: PortName,
        to: u32,
        to_port: PortName,
        from_type: String,
        to_type: String,
    },

    /// Two nodes share an id. The document is corrupt rather than merely stale.
    DuplicateNodeId { id: u32 },

    /// A data input fed by more than one wire. Exec inputs may fan in; data inputs may not,
    /// because there is no rule for which value wins.
    OverfedInput {
        node: u32,
        port: PortName,
        sources: usize,
    },
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::UnknownKind { node, kind } => {
                write!(f, "node {node}: no registered kind {kind:?}")
            }
            Problem::DanglingEdge { from, to, missing } => {
                write!(f, "edge {from} → {to}: node {missing} is not in this graph")
            }
            Problem::UnknownPort {
                node,
                kind,
                port,
                is_input,
            } => write!(
                f,
                "node {node} ({kind}) has no {} port {port:?}",
                if *is_input { "input" } else { "output" }
            ),
            Problem::TypeMismatch {
                from,
                from_port,
                to,
                to_port,
                from_type,
                to_type,
            } => write!(
                f,
                "{from}.{from_port} ({from_type}) cannot feed {to}.{to_port} ({to_type})"
            ),
            Problem::DuplicateNodeId { id } => write!(f, "two nodes share the id {id}"),
            Problem::OverfedInput {
                node,
                port,
                sources,
            } => write!(
                f,
                "node {node}: data input {port:?} is fed by {sources} wires; it takes one"
            ),
        }
    }
}

/// Check a whole document. An empty result means it will run.
pub fn validate<M: GraphMeta, H: Host>(graph: &Graph<M>, reg: &NodeRegistry<H>) -> Vec<Problem> {
    let mut found = Vec::new();

    let mut seen_ids: Vec<u32> = Vec::with_capacity(graph.nodes.len());
    for n in &graph.nodes {
        if seen_ids.contains(&n.id) {
            found.push(Problem::DuplicateNodeId { id: n.id });
        } else {
            seen_ids.push(n.id);
        }
        if reg.get(&n.kind).is_none() {
            found.push(Problem::UnknownKind {
                node: n.id,
                kind: n.kind.as_str().to_string(),
            });
        }
    }

    // How many wires feed each data input, so fan-in can be reported once per port rather than
    // once per extra wire.
    let mut fed: Vec<(u32, PortName, usize)> = Vec::new();

    for e in &graph.edges {
        let (Some(src), Some(dst)) = (graph.node(e.from), graph.node(e.to)) else {
            found.push(Problem::DanglingEdge {
                from: e.from,
                to: e.to,
                missing: if graph.node(e.from).is_none() {
                    e.from
                } else {
                    e.to
                },
            });
            continue;
        };

        // A node of an unknown kind has already been reported; its ports cannot be checked and
        // saying so again for every wire would bury the useful lines.
        let (Some(sspec), Some(dspec)) = (reg.get(&src.kind), reg.get(&dst.kind)) else {
            continue;
        };

        let out = sspec
            .exec_out
            .resolve(&src.config)
            .into_iter()
            .chain(sspec.outputs.resolve(&src.config))
            .find(|p| p.name == e.from_port);
        let Some(out) = out else {
            found.push(Problem::UnknownPort {
                node: e.from,
                kind: src.kind.as_str().to_string(),
                port: e.from_port.clone(),
                is_input: false,
            });
            continue;
        };

        let is_exec_in = e.to_port == EXEC_IN.name;
        let inp = if is_exec_in {
            Some(EXEC_IN.clone())
        } else {
            dspec
                .inputs
                .resolve(&dst.config)
                .into_iter()
                .find(|p| p.name == e.to_port)
        };
        let Some(inp) = inp else {
            found.push(Problem::UnknownPort {
                node: e.to,
                kind: dst.kind.as_str().to_string(),
                port: e.to_port.clone(),
                is_input: true,
            });
            continue;
        };

        if !crate::port::compatible(&out.ty, &inp.ty) {
            found.push(Problem::TypeMismatch {
                from: e.from,
                from_port: e.from_port.clone(),
                to: e.to,
                to_port: e.to_port.clone(),
                from_type: out.ty.as_str().to_string(),
                to_type: inp.ty.as_str().to_string(),
            });
        }

        if !is_exec_in {
            match fed
                .iter_mut()
                .find(|(n, p, _)| *n == e.to && *p == e.to_port)
            {
                Some((_, _, count)) => *count += 1,
                None => fed.push((e.to, e.to_port.clone(), 1)),
            }
        }
    }

    for (node, port, sources) in fed {
        if sources > 1 {
            found.push(Problem::OverfedInput {
                node,
                port,
                sources,
            });
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::nodes::services::Services;
    use crate::{Graph, NodeId};
    use serde_json::json;

    fn reg() -> NodeRegistry<TestHost> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &Services::none());
        r
    }

    #[test]
    fn a_graph_that_runs_reports_nothing() {
        let r = reg();
        let mut g: Graph = Graph::new("fine");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        g.node_mut(each).unwrap().config = json!({ "items": "a,b" });
        let say = g.add_node(NodeId::new("print"), 200, 0);
        g.add_edge(&r, each, "loop_body", say, "exec_in").unwrap();
        g.add_edge(&r, each, "item", say, "message").unwrap();
        assert_eq!(validate(&g, &r), Vec::new());
    }

    #[test]
    fn a_kind_this_build_does_not_know_is_found_before_it_runs() {
        let r = reg();
        let mut g: Graph = Graph::new("stale");
        let id = g.add_node(NodeId::new("a_node_from_a_newer_deploy"), 0, 0);
        assert_eq!(
            validate(&g, &r),
            vec![Problem::UnknownKind {
                node: id,
                kind: "a_node_from_a_newer_deploy".into()
            }],
            "the alternative is finding out during a run, at whatever hour the trigger fires"
        );
    }

    #[test]
    fn every_problem_is_reported_not_the_first() {
        let r = reg();
        let mut g: Graph = Graph::new("several");
        let a = g.add_node(NodeId::new("who_knows"), 0, 0);
        let b = g.add_node(NodeId::new("also_unknown"), 100, 0);
        let c = g.add_node(NodeId::new("still_no"), 200, 0);
        assert_eq!(
            validate(&g, &r).len(),
            3,
            "fixing one and re-running to discover the next is a bisect, not a report"
        );
        let _ = (a, b, c);
    }

    /// A port that existed when the wire was drawn and does not now — the shape a node with
    /// config-derived ports takes after somebody reconfigures it.
    #[test]
    fn a_wire_to_a_port_that_no_longer_exists_is_found() {
        let r = reg();
        let mut g: Graph = Graph::new("reconfigured");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        let say = g.add_node(NodeId::new("format"), 200, 0);
        g.node_mut(say).unwrap().config = json!({ "template": "{item}" });
        g.add_edge(&r, each, "item", say, "item").unwrap();
        // The template changes, so the `item` port is gone — but the wire is still in the file.
        g.node_mut(say).unwrap().config = json!({ "template": "nothing" });

        let problems = validate(&g, &r);
        assert!(
            matches!(
                problems.as_slice(),
                [Problem::UnknownPort { is_input: true, .. }]
            ),
            "got {problems:?}"
        );
    }

    #[test]
    fn an_edge_naming_a_node_that_is_gone_is_found() {
        let r = reg();
        let mut g: Graph = Graph::new("dangling");
        let a = g.add_node(NodeId::new("print"), 0, 0);
        let b = g.add_node(NodeId::new("print"), 200, 0);
        g.add_edge(&r, a, "exec_out", b, "exec_in").unwrap();
        g.remove_node(b);
        // remove_node takes its edges with it, so put the wire back by hand — which is what a
        // hand-edited or partially-migrated document looks like.
        g.edges.push(crate::Edge {
            from: a,
            from_port: PortName::new("exec_out"),
            to: b,
            to_port: PortName::new("exec_in"),
        });
        assert_eq!(
            validate(&g, &r),
            vec![Problem::DanglingEdge {
                from: a,
                to: b,
                missing: b
            }]
        );
    }

    #[test]
    fn a_data_input_fed_twice_is_reported_once() {
        let r = reg();
        let mut g: Graph = Graph::new("overfed");
        let one = g.add_node(NodeId::new("for_each"), 0, -100);
        let two = g.add_node(NodeId::new("for_each"), 0, 100);
        let say = g.add_node(NodeId::new("print"), 200, 0);
        g.add_edge(&r, one, "item", say, "message").unwrap();
        // add_edge refuses the second; a stored document can still contain it.
        g.edges.push(crate::Edge {
            from: two,
            from_port: PortName::new("item"),
            to: say,
            to_port: PortName::new("message"),
        });
        assert_eq!(
            validate(&g, &r),
            vec![Problem::OverfedInput {
                node: say,
                port: PortName::new("message"),
                sources: 2
            }],
            "one line per port, not one per extra wire"
        );
    }
}
