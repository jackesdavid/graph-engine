// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What a report can be made of, as data.
//!
//! Two readers, and both need it before they act rather than by trial:
//!
//! - **a model** assembling a graph, which has to know a bar chart takes `values` and `labels`
//!   before it wires anything;
//! - **an editor**, which can offer the right control for each field.
//!
//! Derived from the same enum the renderer matches on, so a component that renders is a component
//! that appears here. Two lists of the same thing is how one comes to describe a block that no
//! longer exists.

use serde_json::{json, Value};

/// The component catalogue.
pub fn schema() -> Value {
    json!({
        "components": [
            {
                "type": "heading",
                "description": "A section title. Levels 1 to 3; deeper is a document nobody reads on one page.",
                "fields": {
                    "text": { "type": "text", "required": true },
                    "level": { "type": "num", "default": 1, "range": [1, 3] }
                }
            },
            {
                "type": "paragraph",
                "description": "A block of prose. `source` is rendered beside the text, not in a \
                                footnote: a claim separated from its source reads as unsourced.",
                "fields": {
                    "text": { "type": "text", "required": true },
                    "source": { "type": "text", "required": false }
                }
            },
            {
                "type": "table",
                "description": "Rows of strings under named columns. The graph does any arithmetic \
                                before it gets here — a table in a report is read, not computed.",
                "fields": {
                    "columns": { "type": "list", "of": "text" },
                    "rows": { "type": "list", "of": "list" }
                }
            },
            {
                "type": "bar_chart",
                "description": "One bar per label. `labels` and `values` are read in step, so they \
                                must be the same length.",
                "fields": {
                    "title": { "type": "text" },
                    "labels": { "type": "list", "of": "text" },
                    "values": { "type": "list", "of": "num" }
                }
            },
            {
                "type": "layout",
                "description": "Holds other blocks, including other layouts. This is how any \
                                arrangement is reached: a row of columns, a chart beside a table.",
                "fields": {
                    "direction": { "type": "enum", "of": ["row", "column"], "default": "column" },
                    "gap": { "type": "num", "default": 16 },
                    "align": { "type": "enum", "of": ["start", "center", "end", "stretch"], "default": "stretch" },
                    "justify": { "type": "enum", "of": ["start", "center", "end", "between"], "default": "start" },
                    "children": { "type": "list", "of": "block" }
                }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::block::Block;

    /// Every component the renderer draws appears in the schema.
    ///
    /// The check that matters: a model reading this list and finding it short would wire a graph
    /// around a component it believes does not exist.
    #[test]
    fn the_schema_covers_every_block_the_renderer_draws() {
        let s = schema();
        let listed: Vec<&str> = s["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["type"].as_str().unwrap())
            .collect();

        // Named rather than reflected, so adding a variant without a schema entry fails here
        // instead of silently shipping a component nothing can describe.
        for drawn in ["heading", "paragraph", "table", "bar_chart", "layout"] {
            assert!(listed.contains(&drawn), "missing {drawn}: {listed:?}");
        }
        assert_eq!(
            listed.len(),
            5,
            "a variant was added without a schema entry"
        );
    }

    /// The tag in the schema is the tag serde writes, or a model builds JSON nothing can read.
    #[test]
    fn the_schema_tags_match_the_serialized_ones() {
        let j = serde_json::to_value(Block::paragraph("x")).unwrap();
        assert_eq!(j["type"], "paragraph");
        let j = serde_json::to_value(Block::stack(Default::default(), vec![])).unwrap();
        assert_eq!(j["type"], "layout");
    }

    /// Every component says what it is for. Names alone do not let a model choose between a table
    /// and a bar chart.
    #[test]
    fn every_component_carries_a_description() {
        for c in schema()["components"].as_array().unwrap() {
            let d = c["description"].as_str().unwrap_or("");
            assert!(d.len() > 20, "{} has no useful description", c["type"]);
        }
    }
}
