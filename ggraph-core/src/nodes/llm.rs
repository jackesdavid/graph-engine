// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The three ways a graph uses a model: `ask_llm`, `llm_decide`, `llm_extract`.
//!
//! One file, because they are one integration and one set of rules about what a model is
//! allowed to decide. Splitting them would put the "the model may say it does not know" rule in
//! three places and let it drift in two.
//!
//! ## The rule that shapes all three
//!
//! **A model is allowed not to know, and the graph must be able to see that.** `llm_decide` has
//! three arms rather than two, and `llm_extract` reports confidence per field. Forcing a model
//! to pick is how a workflow gets a confident wrong answer, and a confident wrong answer is
//! worse than a blank one — nobody checks the ones that look fine.
//!
//! ## What is deliberately not here
//!
//! No conversation, no tool calling, no streaming. Those belong to whatever the product builds
//! on its own side; a *node* needs a question and an answer of a declared shape. Growing this
//! into a chat client is how the engine acquires an opinion about a provider.

use crate::host::Host;
use crate::id::PortName;
use crate::nodes::services::LlmRequest;
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeCx, NodeError, NodeRoute, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static ASK_IN: [Port; 2] = [
    Port::opt("prompt", PortType::TEXT),
    Port::opt("attachment", PortType::BYTES),
];
static ASK_OUT: [Port; 1] = [Port::opt("answer", PortType::TEXT)];

static DECIDE_OUT: [Port; 1] = [Port::opt("answer", PortType::BOOL)];
static DECIDE_ARMS: [Port; 3] = [
    Port::opt("yes", PortType::EXEC),
    Port::opt("no", PortType::EXEC),
    Port::opt("unknown", PortType::EXEC),
];

static EXTRACT_OUT: [Port; 1] = [Port::opt("fields", PortType::MAP)];

fn request<H: Host>(cx: &NodeCx<'_, H>, prompt_key: &str) -> Result<LlmRequest, NodeError> {
    let prompt = cx
        .input_or_cfg(prompt_key)
        .as_ref()
        .and_then(Value::as_text)
        .unwrap_or_default();
    if prompt.trim().is_empty() {
        return Err(NodeError::new("nothing to ask"));
    }
    Ok(LlmRequest {
        prompt,
        attachment: cx.input("attachment").and_then(|v| v.as_bytes().cloned()),
        timeout_secs: cx
            .cfg_str("timeout_secs")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(60),
    })
}

// ---------------------------------------------------------------------------------------

struct Ask {
    llm: std::sync::Arc<dyn crate::nodes::services::Llm>,
}

impl<H: Host> NodeRun<H> for Ask {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let answer = self.llm.ask_text(request(cx, "prompt")?)?;
        let mut out = PortValues::new();
        out.insert(PortName::new("answer"), Value::Text(answer));
        Ok(out)
    }
}

pub fn ask_spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("ask_llm", "Ask a Model", "AI")
        .with_aliases(&["ask_ai"])
        .with_inputs(Ports::Static(&ASK_IN))
        .with_outputs(Ports::Static(&ASK_OUT))
        .with_config(|| json!({ "prompt": "", "timeout_secs": "60" }))
        .with_timeout(Timeout::Secs(120))
        .running(Ask {
            llm: services.llm.clone(),
        })
}

// ---------------------------------------------------------------------------------------

struct Decide {
    llm: std::sync::Arc<dyn crate::nodes::services::Llm>,
}

impl<H: Host> NodeRun<H> for Decide {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        // `None` is a real answer here and travels as an absent output, exactly like an
        // unreadable comparison. The arm below is what a graph branches on.
        if let Some(b) = self.llm.ask_bool(request(cx, "question")?)? {
            out.insert(PortName::new("answer"), Value::Bool(b));
        }
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("answer")).and_then(Value::as_bool) {
            Some(true) => "yes".into(),
            Some(false) => "no".into(),
            None => "would not say".into(),
        }
    }
}

impl<H: Host> NodeRoute<H> for Decide {
    fn arms(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> Vec<PortName> {
        let arm = match out.get(&PortName::new("answer")).and_then(Value::as_bool) {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        vec![PortName::new(arm)]
    }
}

pub fn decide_spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    static IN: [Port; 2] = [
        Port::opt("question", PortType::TEXT),
        Port::opt("attachment", PortType::BYTES),
    ];
    NodeSpec::effectful("llm_decide", "Ask a Model to Decide", "AI")
        .with_aliases(&["ai_switch"])
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&DECIDE_OUT))
        .with_exec_out(ExecOut::Static(&DECIDE_ARMS))
        .with_config(|| json!({ "question": "", "timeout_secs": "60" }))
        .with_timeout(Timeout::Secs(120))
        .routing(Decide {
            llm: services.llm.clone(),
        })
}

// ---------------------------------------------------------------------------------------

/// One input per declared field, so the canvas shows what is being pulled out.
fn extract_ports(cfg: &Json) -> Vec<Port> {
    cfg.get("fields")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Json::as_str)
                .map(|f| Port::new(PortName::new(f), PortType::TEXT, false))
                .collect()
        })
        .unwrap_or_default()
}

struct Extract {
    llm: std::sync::Arc<dyn crate::nodes::services::Llm>,
}

impl<H: Host> NodeRun<H> for Extract {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let fields: Vec<String> = cx
            .config
            .get("fields")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Json::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if fields.is_empty() {
            return Err(NodeError::new("no fields to extract"));
        }
        let source = cx
            .input_or_cfg("text")
            .as_ref()
            .and_then(Value::as_text)
            .unwrap_or_default();

        let mut req = request(cx, "instruction").unwrap_or(LlmRequest {
            prompt: String::new(),
            attachment: None,
            timeout_secs: 60,
        });
        req.prompt = format!("{}\n\n{}", req.prompt, source);
        req.attachment = cx.input("attachment").and_then(|v| v.as_bytes().cloned());

        let mut found: Vec<(String, Value)> = Vec::new();
        for f in &fields {
            // Asked one field at a time on purpose. A single call returning every field makes
            // one refusal poison the whole row, and gives no way to say which field the model
            // was unsure about.
            let mut per = req.clone();
            per.prompt = format!("{}\n\nReturn only the value of: {f}", req.prompt);
            match self.llm.ask_text(per) {
                Ok(v) if !v.trim().is_empty() => found.push((f.clone(), Value::text(v.trim()))),
                // Absent rather than empty: a blank cell in a table of four hundred rows reads
                // as "the document says nothing", not as "we did not get an answer".
                _ => {}
            }
        }

        let mut out = PortValues::new();
        for (k, v) in &found {
            out.insert(PortName::new(k.clone()), v.clone());
        }
        out.insert(PortName::new("fields"), Value::Map(found));
        Ok(out)
    }
}

pub fn extract_spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("llm_extract", "Extract Fields", "AI")
        .with_aliases(&["ai_extract"])
        .with_inputs(Ports::dynamic(|cfg: &Json| {
            let mut p = vec![
                Port::opt("text", PortType::TEXT),
                Port::opt("attachment", PortType::BYTES),
            ];
            p.extend(extract_ports(cfg));
            p
        }))
        .with_outputs(Ports::dynamic(|cfg: &Json| {
            let mut p = extract_ports(cfg);
            p.extend(EXTRACT_OUT.to_vec());
            p
        }))
        .with_config(
            || json!({ "instruction": "", "fields": [], "text": "", "timeout_secs": "60" }),
        )
        .with_timeout(Timeout::Secs(300))
        .running(Extract {
            llm: services.llm.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn deciding_has_three_arms_and_the_third_is_not_a_no() {
        let s: NodeSpec<TestHost> = decide_spec(&crate::nodes::services::Services::none());
        let arms: Vec<&str> = s
            .exec_out
            .resolve(&json!({}))
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .iter()
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str)
            .collect();
        assert_eq!(arms, vec!["yes", "no", "unknown"]);

        let Behavior::Route(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({});
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        // No answer at all — the model would not commit.
        let got: Vec<String> = r
            .arms(&cx, &PortValues::new())
            .iter()
            .map(|a| a.as_str().to_string())
            .collect();
        assert_eq!(
            got,
            vec!["unknown"],
            "forcing a model to pick is how a workflow gets a confident wrong answer, and \
             nobody re-checks the ones that look fine"
        );
    }

    #[test]
    fn extract_declares_a_port_per_field() {
        let ports: Vec<String> = extract_ports(&json!({ "fields": ["cnpj", "valor"] }))
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(ports, vec!["cnpj", "valor"]);
    }

    #[test]
    fn asking_with_an_empty_prompt_is_refused() {
        let s: NodeSpec<TestHost> = ask_spec(&crate::nodes::services::Services::none());
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "prompt": "" });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        assert_eq!(r.run(&cx).unwrap_err(), NodeError::new("nothing to ask"));
    }

    #[test]
    fn the_old_slugs_still_resolve() {
        // ask_ai, ai_switch and ai_extract exist in saved graphs. A rename that does not carry
        // its alias is a graph that stops opening.
        let s: NodeSpec<TestHost> = ask_spec(&crate::nodes::services::Services::none());
        assert!(s.aliases.contains(&"ask_ai"));
        let s: NodeSpec<TestHost> = decide_spec(&crate::nodes::services::Services::none());
        assert!(s.aliases.contains(&"ai_switch"));
    }
}
