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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// One named choice, or its own default. A word nothing recognises costs only its own field.
fn named<T: Default + serde::de::DeserializeOwned>(cfg: &serde_json::Value, key: &str) -> T {
    cfg.get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// By hand, because `#[derive(Default)]` ignores `#[serde(default = "…")]` — it would have given
/// `gap: 0` while the node config, the schema and the documentation all said 16. Every fallback in
/// the report goes through here (`row()`, `column()`, and each `unwrap_or_default`), so the derived
/// version was a default nobody wrote and everybody got.
impl Default for Layout {
    fn default() -> Self {
        Layout {
            direction: Direction::default(),
            gap: default_gap(),
            align: Align::default(),
            justify: Justify::default(),
        }
    }
}

impl Layout {
    /// Read from a node's configuration, one decision at a time.
    ///
    /// Deserialising the whole struct was all-or-nothing: an inspector writes `"gap": "24"` as a
    /// string, `u16` refuses it, and the fallback reset direction, align and justify along with it.
    /// A row silently became a column because somebody typed in the gap field.
    pub fn read(cfg: &serde_json::Value) -> Self {
        Layout {
            direction: named(cfg, "direction"),
            gap: cfg
                .get("gap")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .map(|n| n.min(u16::MAX as u64) as u16)
                .unwrap_or_else(default_gap),
            align: named(cfg, "align"),
            justify: named(cfg, "justify"),
        }
    }

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

    /// The default nobody wrote and everybody got: `#[derive(Default)]` does not see
    /// `#[serde(default = "…")]`, so every fallback in the report used a gap of zero while three
    /// other places said sixteen.
    #[test]
    fn the_default_gap_is_the_documented_one() {
        assert_eq!(Layout::default().gap, 16);
        assert_eq!(Layout::row().gap, 16);
        assert_eq!(Layout::column().gap, 16);
    }

    /// An inspector writes numbers as strings. Refusing one used to reset the other three, so a row
    /// silently became a column because somebody typed in the gap field.
    #[test]
    fn one_field_written_as_a_string_does_not_reset_the_others() {
        let l = Layout::read(&serde_json::json!({ "direction": "row", "gap": "24" }));
        assert_eq!(l.direction, Direction::Row);
        assert_eq!(l.gap, 24);
    }

    /// And a value nothing recognises falls back on its own, not with the rest.
    #[test]
    fn an_unreadable_value_costs_only_its_own_field() {
        let l = Layout::read(&serde_json::json!({ "direction": "row", "align": "sideways" }));
        assert_eq!(l.direction, Direction::Row, "still a row");
        assert_eq!(l.align, Align::Stretch, "and only align fell back");
    }

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
