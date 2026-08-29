// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! How a chart is drawn, as distinct from what it shows.
//!
//! Beside [`Layout`](super::Layout) and for the same reason: two renderers have to honour this —
//! the static SVG one here and a client-side one drawing from the same tree — so the contract has
//! to be a small set of named decisions rather than "whatever the drawing library does".
//!
//! Each of these is a thing somebody looks at a chart and wants changed: the bars are too fat, the
//! axis numbers are noise, the labels are too long to read sideways.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bars {
    /// Bars rise from a baseline. The default, and what "bar chart" means to most readers.
    #[default]
    Vertical,
    /// Bars run left to right. The one to reach for when the labels are file names: a horizontal
    /// bar has a whole line for its name, and a vertical one has the width of a bar.
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartStyle {
    #[serde(default)]
    pub bars: Bars,
    /// Space between bars, as a percentage of the space each one is given. Zero is bars that touch.
    #[serde(default = "default_gap")]
    pub gap: u8,
    /// The VALUE axis: its line, its ticks and its numbers. Off is right for a chart read as a
    /// shape rather than as measurements — a slide, a sparkline beside a paragraph.
    #[serde(default = "yes")]
    pub show_axes: bool,
    /// The NAME axis: the labels under a vertical chart or beside a horizontal one. Its own
    /// setting, because a chart of five documents often wants the names without the numbers.
    ///
    /// Named `show_labels` and not `labels`, because the
    /// chart it is flattened into already has a `labels` — and a duplicate key makes the whole
    /// block fail to load, which is a shape change nothing else would have caught.
    #[serde(default = "yes")]
    pub show_labels: bool,
}

fn default_gap() -> u8 {
    20
}

fn yes() -> bool {
    true
}

/// By hand, because `#[derive(Default)]` ignores `#[serde(default = "…")]` — the same trap the
/// layout fell into, where every fallback used a gap of zero while three other places said sixteen.
impl Default for ChartStyle {
    fn default() -> Self {
        ChartStyle {
            bars: Bars::default(),
            gap: default_gap(),
            show_axes: true,
            show_labels: true,
        }
    }
}

impl ChartStyle {
    /// Read from a node's configuration, one decision at a time, so a field an inspector wrote as a
    /// string costs only itself.
    pub fn read(cfg: &serde_json::Value) -> Self {
        let flag = |k: &str, d: bool| {
            cfg.get(k)
                .and_then(|v| {
                    v.as_bool()
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .unwrap_or(d)
        };
        ChartStyle {
            bars: cfg
                .get("bars")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            gap: cfg
                .get("gap")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .map(|n| n.min(90) as u8)
                .unwrap_or_else(default_gap),
            show_axes: flag("show_axes", true),
            show_labels: flag("show_labels", true),
        }
    }

    /// How wide a bar is, within the slot it was given. Clamped so a gap of ninety still leaves
    /// something to look at.
    pub(crate) fn bar_width(&self) -> f64 {
        1.0 - (self.gap.min(90) as f64 / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The trap the layout already fell into: a derived default that ignores serde's.
    #[test]
    fn the_defaults_are_the_documented_ones() {
        let d = ChartStyle::default();
        assert_eq!(d.gap, 20);
        assert!(d.show_axes && d.show_labels);
        assert_eq!(d.bars, Bars::Vertical);
    }

    /// An inspector writes numbers and booleans as strings. Refusing one must not reset the rest.
    #[test]
    fn one_field_written_as_a_string_does_not_reset_the_others() {
        let s =
            ChartStyle::read(&json!({ "bars": "horizontal", "gap": "40", "show_axes": "false" }));
        assert_eq!(s.bars, Bars::Horizontal);
        assert_eq!(s.gap, 40);
        assert!(!s.show_axes);
        assert!(s.show_labels, "untouched, and still on");
    }

    /// A gap of ninety still leaves a bar. A gap of a hundred would leave a chart of nothing.
    #[test]
    fn the_gap_leaves_something_to_look_at() {
        assert!(ChartStyle::read(&json!({ "gap": 100 })).bar_width() > 0.0);
        assert_eq!(ChartStyle::default().bar_width(), 0.8);
    }
}
