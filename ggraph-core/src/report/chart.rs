// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Charts, as static SVG.
//!
//! No JavaScript and no external asset. A report that pulls a `<script>` from a CDN breaks on the
//! first reader without internet, and a report is a thing people email, print and archive.
//!
//! The interactive counterpart is not a second renderer here: `report_render` can emit the block
//! tree as JSON and let a viewer draw it. Same tree, two audiences.

use plotters::prelude::*;

/// A bar chart sized for a report column.
///
/// Returns the SVG as a string so the caller can inline it. Inlining rather than linking is what
/// makes the file self-contained, which is the whole point.
pub fn bar_chart(
    title: &str,
    labels: &[String],
    values: &[f64],
    style: &super::ChartStyle,
) -> String {
    // Nothing to draw is not an error: a search that found no numbers should produce an empty
    // chart with its title, not fail the run that built the report around it.
    if labels.is_empty() || values.is_empty() {
        return empty(title);
    }

    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (640, 360)).into_drawing_area();
        let _ = root.fill(&WHITE);

        // Headroom above the tallest bar so the top one is not flush with the frame, and a floor at
        // zero so a chart of small differences does not exaggerate them by cropping the axis.
        let max = values.iter().cloned().fold(f64::MIN, f64::max).max(0.0) * 1.15;
        let n = labels.len() as f64;

        // The bar's thickness inside the slot it was given; the rest of the slot is the gap.
        let pad = (1.0 - style.bar_width()) / 2.0;
        let name = |v: &f64| labels.get(*v as usize).cloned().unwrap_or_default();

        // Room for the axes only when something is drawn in it. Reserving it either way leaves a
        // chart floating in a margin nothing occupies.
        let horizontal = style.bars == super::Bars::Horizontal;
        let across = if style.show_axes || style.show_labels {
            if horizontal { 120 } else { 56 }
        } else {
            0
        };
        let along = if style.show_axes { if horizontal { 40 } else { 48 } } else { 0 };

        let mut builder = ChartBuilder::on(&root);
        builder
            .caption(title, ("sans-serif", 16))
            .margin(12)
            .x_label_area_size(if horizontal { along } else { across })
            .y_label_area_size(if horizontal { across } else { along });

        if horizontal {
            let Ok(mut chart) = builder.build_cartesian_2d(0.0..max, 0.0..n) else {
                // A chart that cannot be built is still not worth failing a report for.
                return empty(title);
            };
            if style.show_axes || style.show_labels {
                let mut mesh = chart.configure_mesh();
                mesh.disable_y_mesh()
                    .y_labels(labels.len().min(12))
                    .y_label_formatter(&name);
                // The names ARE an axis here — in a drawing there is no line of text without the
                // line it sits against — so each setting turns off the axis it names.
                if !style.show_labels {
                    mesh.disable_y_axis();
                }
                if !style.show_axes {
                    mesh.disable_x_axis().disable_x_mesh();
                }
                let _ = mesh.draw();
            }
            let _ = chart.draw_series(values.iter().enumerate().map(|(i, v)| {
                let (a, b) = (i as f64 + pad, i as f64 + 1.0 - pad);
                Rectangle::new([(0.0, a), (*v, b)], BLUE.mix(0.75).filled())
            }));
        } else {
            let Ok(mut chart) = builder.build_cartesian_2d(0.0..n, 0.0..max) else {
                return empty(title);
            };
            if style.show_axes || style.show_labels {
                let mut mesh = chart.configure_mesh();
                mesh.disable_x_mesh()
                    .x_labels(labels.len().min(12))
                    .x_label_formatter(&name);
                // The names ARE an axis here — in a drawing there is no line of text without the
                // line it sits against — so each setting turns off the axis it names.
                if !style.show_labels {
                    mesh.disable_x_axis();
                }
                if !style.show_axes {
                    mesh.disable_y_axis().disable_y_mesh();
                }
                let _ = mesh.draw();
            }
            let _ = chart.draw_series(values.iter().enumerate().map(|(i, v)| {
                let (a, b) = (i as f64 + pad, i as f64 + 1.0 - pad);
                Rectangle::new([(a, 0.0), (b, *v)], BLUE.mix(0.75).filled())
            }));
        }

        let _ = root.present();
    }
    svg
}

/// The frame, with the title and nothing in it.
fn empty(title: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"120\" role=\"img\">\
         <text x=\"12\" y=\"24\" font-family=\"sans-serif\" font-size=\"16\">{}</text>\
         <text x=\"12\" y=\"56\" font-family=\"sans-serif\" font-size=\"13\" fill=\"#888\">\
         no data</text></svg>",
        super::html::escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_chart_is_self_contained_svg() {
        let svg = bar_chart("Relevance", &["a.pdf".into(), "b.pdf".into()], &[0.8, 0.4], &Default::default());
        assert!(svg.contains("<svg"), "{svg}");

        // The property that matters more than any drawing detail: nothing is FETCHED. Checking for
        // "http" would fail on the XML namespace, which is an identifier and not a request — an
        // easy way to write a test that looks strict and only tests spelling.
        assert!(!svg.contains("<script"), "no javascript");
        assert!(!svg.contains("src="), "nothing loaded");
        assert!(!svg.contains("<image"), "no external image");
        assert!(!svg.contains("@import"), "no imported stylesheet");
    }

    fn svg(style: super::super::ChartStyle) -> String {
        bar_chart("T", &["aaa".into(), "bbb".into()], &[1.0, 2.0], &style)
    }

    /// Each of these is a thing somebody looks at a chart and asks for. A setting that does not
    /// reach the drawing is a knob that turns and does nothing.
    #[test]
    fn the_bars_turn_on_their_side() {
        let up = svg(super::super::ChartStyle::default());
        let across = svg(super::super::ChartStyle {
            bars: super::super::Bars::Horizontal,
            ..Default::default()
        });
        assert_ne!(up, across, "orientation reaches the drawing");
    }

    #[test]
    fn the_gap_changes_how_wide_a_bar_is() {
        let tight = svg(super::super::ChartStyle { gap: 0, ..Default::default() });
        let loose = svg(super::super::ChartStyle { gap: 60, ..Default::default() });
        assert_ne!(tight, loose);
    }

    /// The axes and the names come off independently: a chart read as a shape wants neither, and
    /// one beside a paragraph often wants the names without the numbers.
    #[test]
    fn the_axes_and_the_names_come_off_separately() {
        let full = svg(super::super::ChartStyle::default());
        let no_axes = svg(super::super::ChartStyle { show_axes: false, ..Default::default() });
        let bare = svg(super::super::ChartStyle {
            show_axes: false,
            show_labels: false,
            ..Default::default()
        });
        assert!(full.contains("aaa"), "the names are drawn by default");
        assert!(no_axes.contains("aaa"), "and survive the axes coming off");
        assert!(!bare.contains("aaa"), "until they are turned off themselves");
    }

    /// A search that found no numbers must not fail the report built around it.
    #[test]
    fn no_data_still_draws_a_frame() {
        let svg = bar_chart("Empty", &[], &[], &Default::default());
        assert!(svg.contains("Empty"));
        assert!(svg.contains("no data"));
    }

    /// The axis floors at zero even when every value is far from it.
    ///
    /// Cropping it would turn a 1% difference into a towering one, which is the most common way a
    /// chart lies — and the values here (100 and 101) are exactly the shape that tempts it.
    #[test]
    fn the_axis_starts_at_zero() {
        let svg = bar_chart("t", &["a".into(), "b".into()], &[100.0, 101.0], &Default::default());
        // The label sits on its own line inside the <text> element.
        assert!(
            svg.contains("\n0.0\n"),
            "a zero tick exists, so the bars are read against zero"
        );
    }
}
