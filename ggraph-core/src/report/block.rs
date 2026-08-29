// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What a report is made of.
//!
//! One enum, and adding a component is one variant plus one arm in the renderer. That is the whole
//! extension story, and it is why the catalogue can grow without any node knowing about any other.
//!
//! [`Block::Layout`] holds children and is itself a `Block`. That single fact is what makes any
//! arrangement reachable: a layout takes layouts, so nesting needs no special case anywhere.

use super::layout::Layout;
use serde::{Deserialize, Serialize};

/// One row of a table. Cells are strings because a table in a report is read, not computed — the
/// graph did the arithmetic before it got here.
pub type Row = Vec<String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        text: String,
        /// 1–3. Deeper than that is a document nobody reads on one page.
        #[serde(default = "one")]
        level: u8,
    },
    Paragraph {
        text: String,
    },
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Row>,
    },
    BarChart {
        #[serde(default)]
        title: String,
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        values: Vec<f64>,
        /// How it is drawn, as distinct from what it shows. Flattened, so a block written before
        /// these existed still loads and takes the documented defaults.
        #[serde(default, flatten)]
        style: super::ChartStyle,
    },
    /// A container, and itself a block. Layouts nest because of this and nothing else.
    Layout {
        #[serde(default, flatten)]
        layout: Layout,
        #[serde(default)]
        children: Vec<Block>,
    },
}

fn one() -> u8 {
    1
}

impl Block {
    pub fn heading(text: impl Into<String>, level: u8) -> Self {
        Block::Heading {
            text: text.into(),
            level: level.clamp(1, 3),
        }
    }

    pub fn paragraph(text: impl Into<String>) -> Self {
        Block::Paragraph { text: text.into() }
    }

    pub fn stack(layout: Layout, children: Vec<Block>) -> Self {
        Block::Layout { layout, children }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::layout::Direction;

    /// A layout inside a layout, which is the property the whole design rests on. If this ever
    /// stops round-tripping, every arrangement more complex than a list stops working.
    #[test]
    fn layouts_nest() {
        let tree = Block::stack(
            Layout::column(),
            vec![
                Block::heading("Title", 1),
                Block::stack(
                    Layout::row(),
                    vec![
                        Block::Table {
                            columns: vec!["a".into()],
                            rows: vec![vec!["1".into()]],
                        },
                        Block::BarChart {
                            style: Default::default(),
                            title: "t".into(),
                            labels: vec!["x".into()],
                            values: vec![1.0],
                        },
                    ],
                ),
            ],
        );

        let back: Block = serde_json::from_str(&serde_json::to_string(&tree).unwrap()).unwrap();
        let Block::Layout { children, layout } = back else {
            panic!("root is a layout")
        };
        assert_eq!(layout.direction, Direction::Column);
        let Block::Layout {
            layout: inner,
            children: pair,
        } = &children[1]
        else {
            panic!("the second child is a nested layout")
        };
        assert_eq!(inner.direction, Direction::Row, "a row inside a column");
        assert_eq!(pair.len(), 2, "table beside chart");
    }

    /// The tag is what lets a component be added without the reader knowing the others.
    #[test]
    fn a_block_says_what_it_is() {
        let j = serde_json::to_value(Block::heading("H", 2)).unwrap();
        assert_eq!(j["type"], "heading");
        assert_eq!(j["level"], 2);
    }

    /// Level 9 is a document nobody reads on one page.
    #[test]
    fn heading_levels_are_clamped() {
        let Block::Heading { level, .. } = Block::heading("H", 9) else {
            panic!()
        };
        assert_eq!(level, 3);
    }
}
