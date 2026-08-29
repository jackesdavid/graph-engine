// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `format` — build a string from a template and some values.
//!
//! `"{name} owes {amount}"` with `name` and `amount` as ports. A placeholder with no value is
//! left standing rather than replaced with an empty string: `"owes "` looks like a finished
//! sentence about nothing, while `"{amount}"` in the output is visibly a hole.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static OUT: [Port; 1] = [Port::opt("text", PortType::TEXT)];

/// One input per `{placeholder}` in the template, in first-appearance order.
///
/// Derived from the template rather than fixed, so adding a placeholder grows a pin instead of
/// requiring a different node. Duplicates collapse to one port — `"{a} and {a}"` is one value.
fn inputs(cfg: &Json) -> Vec<Port> {
    let template = cfg.get("template").and_then(Json::as_str).unwrap_or("");
    let mut seen: Vec<String> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        if !name.is_empty() && !seen.iter().any(|s| s == name) {
            seen.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    seen.into_iter()
        .map(|n| Port::new(PortName::new(n), PortType::SCALAR, false))
        .collect()
}

struct Format;

impl<H: Host> NodeRun<H> for Format {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let template = cx.cfg_str("template").unwrap_or("").to_string();
        let mut out_text = String::with_capacity(template.len());
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            out_text.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else {
                // An unclosed brace is literal text, not a syntax error. A template is written by
                // hand and half of them mention a brace on purpose.
                out_text.push('{');
                rest = after;
                continue;
            };
            let name = &after[..close];
            match cx.input(name).map(Value::summary) {
                Some(v) => out_text.push_str(&v),
                None => {
                    out_text.push('{');
                    out_text.push_str(name);
                    out_text.push('}');
                }
            }
            rest = &after[close + 1..];
        }
        out_text.push_str(rest);

        let mut out = PortValues::new();
        out.insert(PortName::new("text"), Value::Text(out_text));
        Ok(out)
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::pure("format", "Format", "Text")
        .about(
            r#"Builds a piece of text from a template.

Every `{name}` in the template becomes an input port of that name — so the template decides the
node's shape. This is how values become a sentence.

```
Cell value (model) --> Format ("Model: {model}") --text--> ReportParagraph
```"#,
        )
        .with_inputs(Ports::dynamic(inputs))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "template": "" }))
        .running(Format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    fn render(template: &str, values: &[(&str, Value)]) -> String {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let cfg = json!({ "template": template });
        let mut inputs = PortValues::new();
        for (k, v) in values {
            inputs.insert(PortName::new(*k), v.clone());
        }
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        r.run(&cx)
            .unwrap()
            .get(&PortName::new("text"))
            .and_then(Value::as_text)
            .unwrap()
    }

    #[test]
    fn placeholders_are_replaced() {
        assert_eq!(
            render(
                "{who} owes {amount}",
                &[("who", Value::text("Ana")), ("amount", Value::int(40))]
            ),
            "Ana owes 40"
        );
    }

    #[test]
    fn a_missing_value_leaves_a_visible_hole() {
        assert_eq!(
            render("{who} owes {amount}", &[("who", Value::text("Ana"))]),
            "Ana owes {amount}",
            "'Ana owes ' reads like a finished sentence about nothing"
        );
    }

    #[test]
    fn ports_come_from_the_template() {
        let names: Vec<String> = inputs(&json!({ "template": "{a}-{b}-{a}" }))
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["a", "b"],
            "a repeated placeholder is one value, one pin"
        );
    }

    #[test]
    fn an_unclosed_brace_is_text() {
        assert_eq!(
            render("set {x to 3", &[]),
            "set {x to 3",
            "templates are written by hand and half of them mention a brace on purpose"
        );
    }
}
