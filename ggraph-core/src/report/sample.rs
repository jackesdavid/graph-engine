// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What a report will look like, before it has any data.
//!
//! An author arranging components wants to see the arrangement. Waiting for a run to find out that
//! the chart should have been beside the table, not under it, is a slow way to learn something the
//! layout already knows.
//!
//! So: the same tree, with placeholder content. Structure is real — every heading, every column,
//! every nesting comes from the graph — and only the words and numbers are invented. What the
//! author is judging is the shape, and the shape is not a guess.

use super::block::Block;

/// Fills a tree's empty leaves so it can be rendered.
///
/// Content that IS present is kept: a heading whose text was configured shows that text, because
/// seeing your own title is part of judging the page. Only what would arrive on a wire is invented.
pub fn sample(block: &Block) -> Block {
    match block {
        Block::Heading { text, level } if text.trim().is_empty() => Block::Heading {
            text: "Section title".into(),
            level: *level,
        },

        Block::Paragraph { text } if text.trim().is_empty() => Block::Paragraph {
            // Ending in a citation, because that is how a real one ends and its width is what
            // decides whether the line wraps.
            text: format!("{LOREM} [c7f9-s4-0000]"),
        },

        Block::Table { columns, rows } if rows.is_empty() => Block::Table {
            columns: if columns.is_empty() {
                vec!["Column A".into(), "Column B".into()]
            } else {
                columns.clone()
            },
            // Three rows: enough to show alternating rhythm and the header's relationship to the
            // body, few enough not to pretend it is real data.
            rows: (1..=3)
                .map(|i| {
                    let n = columns.len().max(2);
                    (1..=n).map(|c| format!("value {i}.{c}")).collect()
                })
                .collect(),
        },

        Block::BarChart {
            title,
            labels,
            values,
        } if values.is_empty() => Block::BarChart {
            title: if title.trim().is_empty() {
                "Chart title".into()
            } else {
                title.clone()
            },
            labels: if labels.is_empty() {
                vec!["one".into(), "two".into(), "three".into(), "four".into()]
            } else {
                labels.clone()
            },
            // Uneven on purpose: four equal bars would hide a scaling bug and make the chart look
            // right when it is not.
            values: vec![0.82, 0.41, 0.63, 0.28],
        },

        Block::Layout { layout, children } => Block::Layout {
            layout: layout.clone(),
            // An empty layout still shows its shape: two boxes in the configured direction say more
            // about the page than an empty div does.
            children: if children.is_empty() {
                vec![Block::paragraph(LOREM), Block::paragraph(LOREM)]
            } else {
                children.iter().map(sample).collect()
            },
        },

        // Already has content — kept as it is.
        other => other.clone(),
    }
}

const LOREM: &str =
    "Placeholder text, long enough to show how a paragraph wraps inside its column \
and where the source line sits beneath it.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::layout::Layout;

    /// Structure is real; only content is invented. That is what makes the preview worth looking at.
    #[test]
    fn the_arrangement_is_kept_and_only_content_is_filled() {
        let tree = Block::stack(
            Layout::row(),
            vec![
                Block::Table {
                    columns: vec!["Doc".into()],
                    rows: vec![],
                },
                Block::BarChart {
                    title: String::new(),
                    labels: vec![],
                    values: vec![],
                },
            ],
        );
        let s = sample(&tree);

        let Block::Layout { layout, children } = &s else {
            panic!()
        };
        assert_eq!(
            layout.direction,
            crate::report::Direction::Row,
            "the row survived"
        );
        assert_eq!(children.len(), 2, "still two, still side by side");

        let Block::Table { columns, rows } = &children[0] else {
            panic!()
        };
        assert_eq!(
            columns,
            &vec!["Doc".to_string()],
            "the author's column kept"
        );
        assert_eq!(rows.len(), 3, "and filled with enough rows to show rhythm");
    }

    /// Seeing your own title is part of judging the page.
    #[test]
    fn content_that_exists_is_not_replaced() {
        let s = sample(&Block::heading("Real title", 1));
        let Block::Heading { text, .. } = s else {
            panic!()
        };
        assert_eq!(text, "Real title");
    }

    /// Four equal bars would hide a scaling bug and look right when it is not.
    #[test]
    fn the_sample_chart_has_uneven_bars() {
        let s = sample(&Block::BarChart {
            title: String::new(),
            labels: vec![],
            values: vec![],
        });
        let Block::BarChart { values, .. } = s else {
            panic!()
        };
        assert!(
            values.windows(2).any(|w| w[0] != w[1]),
            "not all the same height"
        );
    }

    /// An empty layout still shows its shape.
    #[test]
    fn an_empty_layout_previews_as_two_boxes() {
        let s = sample(&Block::stack(Layout::row(), vec![]));
        let Block::Layout { children, .. } = s else {
            panic!()
        };
        assert_eq!(children.len(), 2);
    }
}
