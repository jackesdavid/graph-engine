// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The report a graph WOULD produce, read off the graph itself.
//!
//! An author arranging components wants to see the arrangement. Waiting for a run — which needs a
//! corpus, a model and a minute — to find out that the chart should have been beside the table is a
//! slow way to learn something the drawing already knows.
//!
//! So this walks the block wires backwards from a render node and builds the same tree the run
//! would build, with the content left empty. [`sample`](super::sample) then fills the leaves, and
//! the result is a real document whose STRUCTURE is real: every heading, every nesting, every
//! column comes from the graph, and only the words and numbers are invented.
//!
//! # An unwired slot shows its name
//!
//! That is the whole reason slots are named. A layout with `header` and `body` draws two labelled
//! places, so an author sees the arrangement before anything is connected to it — which is exactly
//! when the arrangement is being decided.

use super::block::Block;
use super::layout::Layout;
use crate::graph::{Graph, GraphMeta, GraphNode};
use crate::id::PortName;
use serde_json::Value as Json;

/// How deep a layout may nest before this stops following. A graph cannot contain a cycle of pure
/// nodes, but a document that arrived by hand or by migration can, and a preview must not hang.
const MAX_DEPTH: usize = 12;

/// The document this node would produce, ready to render.
pub fn preview<M: GraphMeta>(graph: &Graph<M>, node: u32) -> Block {
    let root = of(graph, node, 0);
    super::sample(&root)
}

fn of<M: GraphMeta>(graph: &Graph<M>, node: u32, depth: usize) -> Block {
    let Some(n) = graph.nodes.iter().find(|n| n.id == node) else {
        return placeholder("missing");
    };
    if depth > MAX_DEPTH {
        return placeholder("too deep");
    }

    match n.kind.as_str() {
        "report_render" => match wired(graph, node, "report_layout") {
            Some(src) => of(graph, src, depth + 1),
            None => placeholder("nothing wired"),
        },

        "report_layout" => Block::Layout {
            layout: Layout::read(&n.config),
            children: super::super::nodes::report::slot_names(&n.config)
                .into_iter()
                .map(|name| match wired(graph, node, &name) {
                    Some(src) => of(graph, src, depth + 1),
                    // The named, empty place. Seeing it is the point of naming it.
                    None => placeholder(&name),
                })
                .collect(),
        },

        "report_heading" => Block::Heading {
            text: cfg(&n.config, "text"),
            level: cfg(&n.config, "level").parse().unwrap_or(1),
        },
        "report_paragraph" => Block::Paragraph {
            text: cfg(&n.config, "text"),
        },
        "report_table" => Block::Table {
            columns: Vec::new(),
            rows: Vec::new(),
        },
        // The style comes from the config, so gap, orientation and axes are visible in a preview
        // — which is when somebody is deciding them.
        "report_bar_chart" => Block::BarChart {
            title: cfg(&n.config, "title"),
            labels: Vec::new(),
            values: Vec::new(),
            style: super::ChartStyle::read(&n.config),
        },

        // Something that is not a report component wired into one. Named rather than skipped: a
        // silent gap looks like a layout with a hole in it, which is a different mistake.
        other => placeholder(other),
    }
}

/// The node feeding this input port, if any.
fn wired<M: GraphMeta>(graph: &Graph<M>, node: u32, port: &str) -> Option<u32> {
    let port = PortName::new(port);
    graph
        .edges
        .iter()
        .find(|e| e.to == node && e.to_port == port)
        .map(|e| e.from)
}

fn cfg(config: &Json, key: &str) -> String {
    config
        .get(key)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string()
}

/// An empty place, labelled. Rendered as a paragraph so it lays out like the content it stands in
/// for — a box drawn some other way would move when the real thing arrived.
fn placeholder(label: &str) -> Block {
    Block::Paragraph {
        text: format!("⟨ {label} ⟩"),
    }
}

/// The render nodes in a graph — what a preview can be asked for.
pub fn renders<M: GraphMeta>(graph: &Graph<M>) -> Vec<&GraphNode> {
    graph
        .nodes
        .iter()
        .filter(|n| n.kind.as_str() == "report_render")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graph(nodes: Json, edges: Json) -> Graph {
        serde_json::from_value(json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "name": "g", "nodes": nodes, "edges": edges
        }))
        .unwrap()
    }

    /// The structure is the graph's; only the words are invented.
    #[test]
    fn the_arrangement_comes_from_the_wires() {
        let g = graph(
            json!([
                { "id": 1, "kind": "report_render", "x": 0, "y": 0, "config": {} },
                { "id": 2, "kind": "report_layout", "x": 0, "y": 0,
                  "config": { "direction": "row", "slots": [{"name":"left"},{"name":"right"}] } },
                { "id": 3, "kind": "report_heading", "x": 0, "y": 0, "config": { "text": "Título" } }
            ]),
            json!([
                { "from": 2, "from_port": "report_layout", "to": 1, "to_port": "report_layout" },
                { "from": 3, "from_port": "block", "to": 2, "to_port": "left" }
            ]),
        );
        let Block::Layout { layout, children } = preview(&g, 1) else {
            panic!("a render shows what it was given")
        };
        assert_eq!(layout.direction, super::super::Direction::Row);
        assert_eq!(children.len(), 2);
        assert!(matches!(&children[0], Block::Heading { text, .. } if text == "Título"));
    }

    /// The reason slots are named: an empty one shows its own name, so an arrangement can be judged
    /// before anything is connected to it — which is when it is being decided.
    #[test]
    fn an_unwired_slot_shows_its_name() {
        let g = graph(
            json!([
                { "id": 1, "kind": "report_render", "x": 0, "y": 0, "config": {} },
                { "id": 2, "kind": "report_layout", "x": 0, "y": 0,
                  "config": { "slots": [{"name":"cabeçalho"}] } }
            ]),
            json!([{ "from": 2, "from_port": "report_layout", "to": 1, "to_port": "report_layout" }]),
        );
        let Block::Layout { children, .. } = preview(&g, 1) else {
            panic!()
        };
        assert!(matches!(&children[0], Block::Paragraph { text } if text.contains("cabeçalho")));
    }

    /// A render with nothing wired says so, rather than drawing a blank page that looks finished.
    #[test]
    fn an_empty_render_says_so() {
        let g = graph(
            json!([{ "id": 1, "kind": "report_render", "x": 0, "y": 0, "config": {} }]),
            json!([]),
        );
        assert!(
            matches!(preview(&g, 1), Block::Paragraph { text } if text.contains("nothing wired"))
        );
    }

    /// A document that arrived by hand can contain a cycle, and a preview must not hang on it.
    #[test]
    fn a_cycle_stops_rather_than_hangs() {
        let g = graph(
            json!([
                { "id": 1, "kind": "report_layout", "x": 0, "y": 0, "config": { "slots": [{"name":"a"}] } },
                { "id": 2, "kind": "report_layout", "x": 0, "y": 0, "config": { "slots": [{"name":"b"}] } }
            ]),
            json!([
                { "from": 2, "from_port": "block", "to": 1, "to_port": "a" },
                { "from": 1, "from_port": "block", "to": 2, "to_port": "b" }
            ]),
        );
        let _ = preview(&g, 1); // returns
    }
}
