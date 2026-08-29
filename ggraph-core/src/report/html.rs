// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! A block tree as a self-contained HTML file.
//!
//! Self-contained is the requirement, not a preference: a report is emailed, printed and archived,
//! and one that fetches anything is broken for the reader without a network — which is every reader
//! of an offline installation.

use super::block::Block;
use super::chart;

/// The default look, used when the caller supplies no theme.
///
/// Deliberately plain. This is a fallback, not a design: a client who cares supplies a stylesheet,
/// and one who does not gets something readable rather than something opinionated.
const DEFAULT_THEME: &str = "\
body{max-width:60rem;margin:3rem auto;padding:0 1.5rem;\
font:16px/1.6 system-ui,-apple-system,sans-serif;color:#1a1a1a}
h1{font-size:1.8rem;margin:0 0 2rem}
h2{font-size:1.2rem;margin:2rem 0 .5rem}
h3{font-size:1rem;margin:1.5rem 0 .5rem}
p{margin:0 0 .75rem}
table{border-collapse:collapse;width:100%;font-size:14px}
th,td{border-bottom:1px solid #e4e4e4;padding:.45rem .6rem;text-align:left}
th{font-weight:600;color:#444}
figure{margin:0}
svg{max-width:100%;height:auto}";

/// Renders a tree into one file.
pub fn render_html(root: &Block, title: &str, theme: Option<&str>) -> String {
    format!(
        "<!doctype html>\n<html lang=\"pt\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>{t}</title>\n<style>\n{css}\n</style>\n</head>\n<body>\n\
         <h1>{t}</h1>\n{body}\n</body>\n</html>\n",
        t = escape(title),
        css = theme.unwrap_or(DEFAULT_THEME),
        body = block(root),
    )
}

fn block(b: &Block) -> String {
    match b {
        Block::Heading { text, level } => {
            let l = (*level).clamp(1, 3) + 1; // h1 is the report title
            format!("<h{l}>{}</h{l}>\n", escape(text))
        }

        Block::Paragraph { text } => format!(
            "<p>{}</p>
",
            escape(text)
        ),

        Block::Table { columns, rows } => {
            let head = if columns.is_empty() {
                String::new()
            } else {
                let th: String = columns
                    .iter()
                    .map(|c| format!("<th>{}</th>", escape(c)))
                    .collect();
                format!("<thead><tr>{th}</tr></thead>")
            };
            let body: String = rows
                .iter()
                .map(|r| {
                    let td: String = r
                        .iter()
                        .map(|c| format!("<td>{}</td>", escape(c)))
                        .collect();
                    format!("<tr>{td}</tr>")
                })
                .collect();
            format!("<table>{head}<tbody>{body}</tbody></table>\n")
        }

        Block::BarChart {
            title,
            labels,
            values,
            style,
        } => format!(
            "<figure>{}</figure>\n",
            chart::bar_chart(title, labels, values, style)
        ),

        // The recursive case, and the only one there is. A layout renders its children the same way
        // the root was rendered, so nesting costs nothing here either.
        Block::Layout { layout, children } => {
            // In a ROW each child is given an equal share and allowed to shrink. Left to themselves
            // they size to their content — a chart is a fixed 640px and a table asks for all of it —
            // so "a chart beside a table", the arrangement layouts exist for, was the one that
            // overflowed. `min-width:0` is what lets a table narrower than its columns still fit.
            let inner: String = match layout.direction {
                crate::report::Direction::Row => children
                    .iter()
                    .map(|c| {
                        format!(
                            "<div style=\"flex:1 1 0;min-width:0\">\n{}</div>\n",
                            block(c)
                        )
                    })
                    .collect(),
                crate::report::Direction::Column => children.iter().map(block).collect(),
            };
            format!("<div style=\"{}\">\n{inner}</div>\n", layout.css())
        }
    }
}

/// Text from a corpus is not trusted markup.
///
/// Documents are full of `<` and `&`, and one of them in a heading would silently swallow the rest
/// of the report.
pub(crate) fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {

    /// The arrangement layouts exist for, and the one that used to overflow: left to size
    /// themselves, a chart fixed at 640px and a table asking for all of it do not fit in 60rem.
    #[test]
    fn a_row_shares_its_width_between_its_children() {
        let row = Block::stack(
            crate::report::Layout::row(),
            vec![
                Block::Table {
                    columns: vec!["a".into()],
                    rows: vec![],
                },
                Block::BarChart {
                    style: Default::default(),
                    title: String::new(),
                    labels: vec!["x".into()],
                    values: vec![1.0],
                },
            ],
        );
        let html = render_html(&row, "t", None);
        assert_eq!(html.matches("flex:1 1 0").count(), 2, "one share each");
        assert!(
            html.contains("min-width:0"),
            "and a table may shrink below its columns, or it claims the row"
        );
    }

    /// A column leaves its children alone: they stack, and each is as wide as the page.
    #[test]
    fn a_column_wraps_nothing() {
        let col = Block::stack(
            crate::report::Layout::column(),
            vec![Block::Heading {
                text: "x".into(),
                level: 1,
            }],
        );
        assert!(!render_html(&col, "t", None).contains("flex:1 1 0"));
    }
    use super::*;
    use crate::report::layout::{Direction, Layout};

    #[test]
    fn document_text_is_escaped_not_trusted() {
        let html = render_html(&Block::heading("R&D <urgent>", 1), "T", None);
        assert!(html.contains("R&amp;D &lt;urgent&gt;"), "{html}");
        assert!(!html.contains("<urgent>"));
    }

    /// A citation rides inside the claim it supports, because that is where it is written and
    /// where it is checked. The paragraph carried it in a field of its own for a while; nothing
    /// ever filled it, and a claim separated from its source reads as unsourced.
    #[test]
    fn the_citation_travels_with_the_claim() {
        let html = render_html(
            &Block::paragraph("The term is 30 days [c7f9-s4-0000]."),
            "T",
            None,
        );
        let claim = html.find("30 days").expect("claim");
        let cite = html.find("c7f9-s4-0000").expect("citation");
        assert!(cite > claim);
    }

    /// The property the whole design rests on: a row inside a column, rendered as nested flex.
    #[test]
    fn a_nested_layout_renders_as_nested_flex() {
        let tree = Block::stack(
            Layout::column(),
            vec![
                Block::heading("Findings", 1),
                Block::stack(
                    Layout {
                        direction: Direction::Row,
                        gap: 24,
                        ..Default::default()
                    },
                    vec![
                        Block::Table {
                            columns: vec!["doc".into()],
                            rows: vec![vec!["a.pdf".into()]],
                        },
                        Block::BarChart {
                            style: Default::default(),
                            title: "scores".into(),
                            labels: vec!["a.pdf".into()],
                            values: vec![0.9],
                        },
                    ],
                ),
            ],
        );
        let html = render_html(&tree, "Report", None);
        assert!(html.contains("flex-direction:column"));
        assert!(
            html.contains("flex-direction:row"),
            "the inner row survived"
        );
        assert!(html.contains("gap:24px"));
        // Table and chart both inside, which is what "side by side" means.
        assert!(html.contains("<table"));
        assert!(html.contains("<svg"));
    }

    /// Nothing is fetched. The one property that makes a report survive being emailed.
    #[test]
    fn the_file_is_self_contained() {
        let tree = Block::stack(
            Layout::column(),
            vec![Block::BarChart {
                style: Default::default(),
                title: "t".into(),
                labels: vec!["a".into()],
                values: vec![1.0],
            }],
        );
        let html = render_html(&tree, "R", None);
        assert!(!html.contains("<script"), "no javascript");
        assert!(!html.contains("<link"), "no external stylesheet");
        assert!(!html.contains("src=\"http"), "nothing fetched");
    }

    /// A client theme replaces the fallback rather than being appended to it — half a letterhead
    /// over somebody else's defaults looks worse than either alone.
    #[test]
    fn a_theme_replaces_the_default() {
        let html = render_html(&Block::paragraph("x"), "T", Some("body{color:red}"));
        assert!(html.contains("body{color:red}"));
        assert!(!html.contains("system-ui"));
    }
}
