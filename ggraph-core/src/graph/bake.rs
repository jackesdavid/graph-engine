// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Configuration a node can only learn from what is wired into it.
//!
//! Some nodes have ports whose types come from their own settings, and those settings are not
//! something an author types — they are what the wire brought. `For Each` produces items of the
//! element type of the list it was given; a node reading a row produces the columns of the schema
//! it was given. The value is on the far end of an edge, and [`Ports::dynamic`](crate::spec::Ports)
//! deliberately cannot see edges: a port that changed because something upstream was rewired is a
//! port that silently invalidates every wire already attached to it.
//!
//! So the wire's contribution is COPIED INTO the config, once, and the ports resolve from the
//! config as they always did. That copy is baking.
//!
//! # Why this is not the editor's job
//!
//! It was. Both products did it in the canvas, on the drop of a wire, per node kind — which meant a
//! graph could only be BUILT by hands on a canvas. Anything else assembling one, a migration or a
//! model, produced a document the engine then refused: `for_each.item (text) cannot feed
//! break_chunk_result.passage (chunk_result)`, correct and unfixable, because the fix was a config
//! key nothing outside the editor knew to write.
//!
//! Here, it is a property of the node kind, declared beside its ports, and everything that
//! assembles a graph gets it.

use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::port::Port;
use crate::registry::NodeRegistry;
use serde_json::Value as Json;

/// How many times the whole graph is walked before giving up.
///
/// Baking propagates: a `For Each` that learns its element type publishes a different `item` port,
/// which is what the node after it bakes from. One pass would settle the first node of a chain and
/// leave the rest. The ceiling is a backstop against a cycle of nodes baking each other, which a
/// loop's back-edge makes possible.
const PASSES: usize = 8;

/// The ports feeding a node, by the name of the input each arrives on.
pub struct Wired(Vec<(PortName, Port)>);

impl Wired {
    /// What is arriving, as a list of (input port name, the source port feeding it).
    ///
    /// Public because baking is not only for a document that exists: a SEARCH over which kinds may
    /// follow which has to ask what a kind would give once something reached it, and there is no
    /// graph at that point to read the answer off.
    pub fn from(pairs: Vec<(PortName, Port)>) -> Self {
        Wired(pairs)
    }

    /// The source port feeding this input, if anything is.
    pub fn on(&self, input: &str) -> Option<&Port> {
        self.0
            .iter()
            .find(|(n, _)| n.as_str() == input)
            .map(|(_, p)| p)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Given a node's current config and what is wired into it, the config it should have.
///
/// `None` means nothing to change — which is the answer for most nodes and every unwired one.
pub type BakeFn = std::sync::Arc<dyn Fn(&Json, &Wired) -> Option<Json> + Send + Sync>;

/// Write into every node the configuration its wiring implies. Returns how many changed.
///
/// Idempotent: running it twice changes nothing the second time, which is what lets it be called on
/// every save without a graph drifting.
pub fn bake<M: GraphMeta, H: Host>(graph: &mut Graph<M>, reg: &NodeRegistry<H>) -> usize {
    let mut changed = 0;
    for _ in 0..PASSES {
        let mut this_pass = 0;
        for i in 0..graph.nodes.len() {
            let id = graph.nodes[i].id;
            let Some(spec) = reg.get(&graph.nodes[i].kind).cloned() else {
                continue;
            };
            let Some(bake) = spec.bake.clone() else {
                continue;
            };

            let wired = incoming(graph, reg, id);
            if wired.is_empty() {
                continue;
            }
            if let Some(next) = bake(&graph.nodes[i].config, &wired) {
                if next != graph.nodes[i].config {
                    graph.nodes[i].config = next;
                    this_pass += 1;
                }
            }
        }
        changed += this_pass;
        if this_pass == 0 {
            break;
        }
    }
    changed
}

/// The source port of every wire arriving at this node, resolved against the source's own config —
/// which may itself have just been baked, and is the reason this is recomputed each pass.
fn incoming<M: GraphMeta, H: Host>(graph: &Graph<M>, reg: &NodeRegistry<H>, node: u32) -> Wired {
    let mut out = Vec::new();
    for e in &graph.edges {
        if e.to != node {
            continue;
        }
        let Some(src) = graph.node(e.from) else {
            continue;
        };
        let Some(spec) = reg.get(&src.kind) else {
            continue;
        };
        if let Some(p) = spec
            .outputs
            .resolve(&src.config)
            .into_iter()
            .find(|p| p.name == e.from_port)
        {
            out.push((e.to_port.clone(), p));
        }
    }
    Wired(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::nodes::services::Services;
    use crate::{NodeId, PortType};
    use serde_json::json;

    fn reg() -> NodeRegistry<TestHost> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &Services::none());
        r
    }

    /// The case that made this exist: a loop over a product's own list type. Its `item` port is
    /// `text` until something tells it otherwise, and the only thing that knows is the wire.
    #[test]
    fn a_loop_learns_its_element_type_from_what_it_was_given() {
        let r = reg();
        let mut g: Graph = Graph::new("loop");
        let rows = g.add_node(NodeId::new("get_table_rows"), 0, 0);
        let each = g.add_node(NodeId::new("for_each"), 200, 0);
        g.edges.push(crate::graph::Edge {
            from: rows,
            from_port: PortName::new("rows"),
            to: each,
            to_port: PortName::new("items"),
        });

        let item = |g: &Graph| {
            r.get(&NodeId::new("for_each"))
                .unwrap()
                .outputs
                .resolve(&g.node(each).unwrap().config)
                .into_iter()
                .find(|p| p.name.as_str() == "item")
                .unwrap()
                .ty
        };
        assert_eq!(item(&g), PortType::TEXT, "nothing has told it yet");

        assert_eq!(bake(&mut g, &r), 1);
        assert_eq!(
            item(&g),
            PortType::TABLE_ROW,
            "the wire said what one of them is"
        );
    }

    /// The other case: a schema declared in one node and needed as columns in another. The editor
    /// copied this on the drop of a wire, which is why a graph could only be built by hands.
    #[test]
    fn a_schema_reaches_the_node_that_needs_its_columns() {
        let r = reg();
        let mut g: Graph = Graph::new("declared");
        let schema = g.add_node(NodeId::new("table_schema"), 0, 0);
        g.node_mut(schema).unwrap().config =
            json!({ "columns": [{ "name": "price", "type": "num" }] });
        let cell = g.add_node(NodeId::new("cell"), 200, 0);
        g.edges.push(crate::graph::Edge {
            from: schema,
            from_port: PortName::new("schema"),
            to: cell,
            to_port: PortName::new("schema"),
        });

        assert_eq!(bake(&mut g, &r), 1);
        assert_eq!(
            g.node(cell).unwrap().config["columns"][0]["name"],
            json!("price"),
            "the node that has to offer the column now knows it exists"
        );
    }

    /// Called on every save, so running it twice must be the same as running it once.
    #[test]
    fn baking_a_baked_graph_changes_nothing() {
        let r = reg();
        let mut g: Graph = Graph::new("twice");
        let rows = g.add_node(NodeId::new("get_table_rows"), 0, 0);
        let each = g.add_node(NodeId::new("for_each"), 200, 0);
        g.edges.push(crate::graph::Edge {
            from: rows,
            from_port: PortName::new("rows"),
            to: each,
            to_port: PortName::new("items"),
        });
        assert_eq!(bake(&mut g, &r), 1);
        assert_eq!(bake(&mut g, &r), 0);
    }

    /// An unwired node is left alone. Its author may have typed something in, and a bake with
    /// nothing to say must not reach in and overwrite it.
    #[test]
    fn an_unwired_node_is_not_touched() {
        let r = reg();
        let mut g: Graph = Graph::new("loose");
        let each = g.add_node(NodeId::new("for_each"), 0, 0);
        g.node_mut(each).unwrap().config = json!({ "items": "a,b", "items_type": "text" });
        assert_eq!(bake(&mut g, &r), 0);
        assert_eq!(
            g.node(each).unwrap().config["items_type"],
            json!("text"),
            "what the author typed survives"
        );
    }

    /// Baking propagates along a chain: the second node bakes from a port the first one only
    /// published after IT was baked. One pass would settle the head and leave the tail.
    #[test]
    fn it_settles_a_chain_not_just_the_first_node() {
        let r = reg();
        let mut g: Graph = Graph::new("chain");
        let rows = g.add_node(NodeId::new("get_table_rows"), 0, 0);
        let each = g.add_node(NodeId::new("for_each"), 200, 0);
        let cell = g.add_node(NodeId::new("cell"), 400, 0);
        for (f, fp, t, tp) in [(rows, "rows", each, "items"), (each, "item", cell, "row")] {
            g.edges.push(crate::graph::Edge {
                from: f,
                from_port: PortName::new(fp),
                to: t,
                to_port: PortName::new(tp),
            });
        }
        // The loop has to bake before `cell` can see a `table_row` arriving at all.
        assert!(bake(&mut g, &r) >= 1);
        assert_eq!(
            r.get(&NodeId::new("for_each"))
                .unwrap()
                .outputs
                .resolve(&g.node(each).unwrap().config)
                .into_iter()
                .find(|p| p.name.as_str() == "item")
                .unwrap()
                .ty,
            PortType::TABLE_ROW
        );
    }
}
