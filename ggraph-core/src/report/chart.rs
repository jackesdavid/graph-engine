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
pub fn bar_chart(title: &str, labels: &[String], values: &[f64]) -> String {
    // Nothing to draw is not an error: a search that found no numbers should produce an empty
    // chart with its title, not fail the run that built the report around it.
    if labels.is_empty() || values.is_empty() {
        return empty(title);
    }

    let (w, h) = (640u32, 360u32);
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (w, h)).into_drawing_area();
        let _ = root.fill(&WHITE);

        // Headroom above the tallest bar so the top one is not flush with the frame, and a floor at
        // zero so a chart of small differences does not exaggerate them by cropping the axis.
        let max = values.iter().cloned().fold(f64::MIN, f64::max).max(0.0) * 1.15;

        let mut chart = match ChartBuilder::on(&root)
            .caption(title, ("sans-serif", 16))
            .margin(12)
            .x_label_area_size(56)
            .y_label_area_size(48)
            .build_cartesian_2d(0..labels.len(), 0.0..max)
        {
            Ok(c) => c,
            // A chart that cannot be built is still not worth failing a report for.
            Err(_) => return empty(title),
        };

        let _ = chart
            .configure_mesh()
            .disable_x_mesh()
            .x_labels(labels.len().min(12))
            .x_label_formatter(&|i| labels.get(*i).cloned().unwrap_or_default())
            .draw();

        let _ = chart.draw_series(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| Rectangle::new([(i, 0.0), (i + 1, *v)], BLUE.mix(0.75).filled())),
        );

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
        let svg = bar_chart("Relevance", &["a.pdf".into(), "b.pdf".into()], &[0.8, 0.4]);
        assert!(svg.contains("<svg"), "{svg}");

        // The property that matters more than any drawing detail: nothing is FETCHED. Checking for
        // "http" would fail on the XML namespace, which is an identifier and not a request — an
        // easy way to write a test that looks strict and only tests spelling.
        assert!(!svg.contains("<script"), "no javascript");
        assert!(!svg.contains("src="), "nothing loaded");
        assert!(!svg.contains("<image"), "no external image");
        assert!(!svg.contains("@import"), "no imported stylesheet");
    }

    /// A search that found no numbers must not fail the report built around it.
    #[test]
    fn no_data_still_draws_a_frame() {
        let svg = bar_chart("Empty", &[], &[]);
        assert!(svg.contains("Empty"));
        assert!(svg.contains("no data"));
    }

    /// The axis floors at zero even when every value is far from it.
    ///
    /// Cropping it would turn a 1% difference into a towering one, which is the most common way a
    /// chart lies — and the values here (100 and 101) are exactly the shape that tempts it.
    #[test]
    fn the_axis_starts_at_zero() {
        let svg = bar_chart("t", &["a".into(), "b".into()], &[100.0, 101.0]);
        // The label sits on its own line inside the <text> element.
        assert!(
            svg.contains("\n0.0\n"),
            "a zero tick exists, so the bars are read against zero"
        );
    }
}
