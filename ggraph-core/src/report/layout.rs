// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! How blocks sit next to each other.
//!
//! The core of flexbox and deliberately not all of it. Two renderers have to implement this — the
//! static HTML one here and a client-side one drawing from the same tree — so the contract cannot be
//! "whatever CSS does". Four properties reach any arrangement worth drawing, and each maps to
//! something a second renderer can honour.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Side by side. The reason layouts exist at all — a column of stacked things is what a linear
    /// chain already produced.
    Row,
    #[default]
    Column,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Start,
    Center,
    End,
    /// Children fill the cross axis. The default because a table beside a chart should share a
    /// height rather than float at different tops.
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    Between,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default)]
    pub direction: Direction,
    /// Space between children, in CSS pixels.
    #[serde(default = "default_gap")]
    pub gap: u16,
    #[serde(default)]
    pub align: Align,
    #[serde(default)]
    pub justify: Justify,
}

fn default_gap() -> u16 {
    16
}

impl Layout {
    pub fn row() -> Self {
        Layout {
            direction: Direction::Row,
            ..Default::default()
        }
    }

    pub fn column() -> Self {
        Layout::default()
    }

    /// The CSS this layout means.
    ///
    /// Kept beside the model rather than inside the renderer: a second renderer reads the same four
    /// decisions, and separating them is how one of the two comes to ignore a property.
    pub(crate) fn css(&self) -> String {
        let dir = match self.direction {
            Direction::Row => "row",
            Direction::Column => "column",
        };
        let align = match self.align {
            Align::Start => "flex-start",
            Align::Center => "center",
            Align::End => "flex-end",
            Align::Stretch => "stretch",
        };
        let justify = match self.justify {
            Justify::Start => "flex-start",
            Justify::Center => "center",
            Justify::End => "flex-end",
            Justify::Between => "space-between",
        };
        format!(
            "display:flex;flex-direction:{dir};gap:{}px;align-items:{align};justify-content:{justify}",
            self.gap
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row is what a linear chain could never produce, so it is the property that has to survive
    /// a round trip intact.
    #[test]
    fn a_row_survives_a_round_trip() {
        let l = Layout {
            direction: Direction::Row,
            gap: 24,
            ..Default::default()
        };
        let back: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back.direction, Direction::Row);
        assert_eq!(back.gap, 24);
    }

    /// Every field defaults, so a graph that configures nothing still lays out.
    #[test]
    fn an_empty_object_is_a_column() {
        let l: Layout = serde_json::from_str("{}").unwrap();
        assert_eq!(l.direction, Direction::Column);
        assert_eq!(l.align, Align::Stretch);
        assert_eq!(l.gap, 16);
    }

    #[test]
    fn the_css_carries_all_four_decisions() {
        let css = Layout {
            direction: Direction::Row,
            gap: 8,
            align: Align::Center,
            justify: Justify::Between,
        }
        .css();
        assert!(css.contains("flex-direction:row"));
        assert!(css.contains("gap:8px"));
        assert!(css.contains("align-items:center"));
        assert!(css.contains("justify-content:space-between"));
    }
}
