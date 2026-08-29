// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `output` — what a graph gives back.
//!
//! A graph could always be RUN and never asked what it produced. That was fine while the only
//! caller was a schedule: the work was the sending of the mail, the writing of the file, the thing
//! that happened. It stops being fine the moment something calls a graph the way you call a
//! function — an agent that needs an answer, a tool that runs one and reads the result.
//!
//! # Why a node, and not a return value
//!
//! Because which value is the answer is the author's decision, not the engine's. A graph has
//! dozens of nodes and every one of them produces something; "the last one to run" is an accident
//! of topology, and a graph with two branches has no last one at all. Wiring the answer to a port
//! says it, on the canvas, where anyone can see what this graph is for.
//!
//! # Its ports are declared, not discovered
//!
//! Each value is named and given a type in the inspector, exactly as a schema declares columns.
//! That means a caller knows the shape of the answer BEFORE the graph runs — the same bargain the
//! rest of the type system makes, and the reason this is not simply "collect whatever reached the
//! end".
//!
//! What arrives on those ports leaves on ports of the same name, so a finished run already holds
//! them in its [`Outputs`](crate::exec::Outputs); [`delivered`](crate::exec::delivered) is the
//! reader that picks them out.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{Field, Fields, NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::PortValues;
use serde_json::{json, Value as Json};

/// The types a value can be given: the scalars, and the containers a caller can actually read.
///
/// No `any`. A caller is told the shape of the answer before the graph runs — that is the whole
/// bargain this node makes — and a port that accepts anything makes no promise at all. An author
/// who genuinely does not know uses `json`, which says so.
pub const VALUE_TYPES: [&str; 7] = ["text", "num", "bool", "table", "json", "list", "file_ref"];

/// The values this graph gives back, in order.
///
/// Blank names are skipped and repeats are dropped, for the reason a layout's slots are: a port
/// nobody can name is one nothing can be wired to, and two ports with one name collapse into each
/// other, silently losing a value.
pub(crate) fn declared(cfg: &Json) -> Vec<(String, PortType)> {
    let mut out: Vec<(String, PortType)> = Vec::new();
    let Some(items) = cfg.get("values").and_then(Json::as_array) else {
        return out;
    };
    for it in items {
        let name = it
            .get("name")
            .and_then(Json::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if name.is_empty() || out.iter().any(|(n, _)| n == name) {
            continue;
        }
        let ty = it
            .get("type")
            .and_then(Json::as_str)
            .filter(|t| VALUE_TYPES.contains(t))
            .unwrap_or("text");
        out.push((name.to_string(), PortType::new(ty)));
    }
    out
}

fn ports(cfg: &Json) -> Vec<Port> {
    declared(cfg)
        .into_iter()
        .map(|(n, ty)| Port::new(PortName::new(n), ty, false))
        .collect()
}

struct Output;

impl<H: Host> NodeRun<H> for Output {
    /// Straight through. The values leave on ports of the same name they arrived on, so the run's
    /// own record of what each node produced is already the answer — no second channel, and
    /// nothing for a resumed run to reconstruct.
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let mut out = PortValues::new();
        for (name, _) in declared(cx.config) {
            let port = PortName::new(name);
            if let Some(v) = cx.input(port.as_str()) {
                out.insert(port, v.clone());
            }
        }
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        let mut names: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
        names.sort();
        if names.is_empty() {
            "nothing".to_string()
        } else {
            names.join(", ")
        }
    }
}

const ABOUT: &str = "\
Ends a graph by naming what it gives back.

Add a value in the inspector, give it a name and a type, and a port of that name appears. Whatever
you wire to it is what a caller receives when the graph finishes — so this is how a graph stops
being a thing that *happens* and becomes a thing you can *ask*.

Everything a caller gets comes from here. A graph with no Output ran, and returned nothing.

```
Chunk Search --results--> Ask --answer--> Output.answer
```
";

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("output", "Output", "Output")
        .about(ABOUT)
        .with_inputs(Ports::dynamic(ports))
        // The same ports on both sides: what arrives leaves, so the run's record of this node IS
        // the answer, and nothing has to be carried alongside it.
        .with_outputs(Ports::dynamic(ports))
        .with_config(|| json!({ "values": [{ "name": "result", "type": "text" }] }))
        .with_fields(Fields::List(vec![Field::rows(
            "values",
            "Values",
            vec![
                Field::text("name", "Name"),
                Field::choice("type", "Type", VALUE_TYPES),
            ],
        )
        .required()]))
        .with_timeout(Timeout::Inline)
        .running(Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(cfg: &Json) -> Vec<String> {
        ports(cfg).iter().map(|p| p.name.as_str().to_string()).collect()
    }

    /// A value is named and typed, and the port takes that name — the caller knows the shape of
    /// the answer before anything runs.
    #[test]
    fn a_declared_value_is_a_port_of_its_own_type() {
        let cfg = json!({ "values": [{ "name": "answer", "type": "text" }] });
        assert_eq!(names(&cfg), vec!["answer"]);
        assert_eq!(ports(&cfg)[0].ty, PortType::TEXT);
    }

    /// A port nobody can name is one nothing can be wired to.
    #[test]
    fn a_blank_name_is_not_a_value() {
        assert_eq!(names(&json!({ "values": [{ "name": " " }, { "name": "a" }] })), vec!["a"]);
    }

    /// Two ports with one name collapse into each other, and a value disappears without a word.
    #[test]
    fn a_repeated_name_is_dropped() {
        let cfg = json!({ "values": [{ "name": "a" }, { "name": "a" }] });
        assert_eq!(names(&cfg).len(), 1);
    }

    /// A type this build does not know falls back rather than refusing: a graph written against a
    /// newer engine must still open. To `text`, not to `any` — a port that accepts anything makes
    /// no promise, and this node exists to make one.
    #[test]
    fn an_unknown_type_falls_back_rather_than_failing() {
        let cfg = json!({ "values": [{ "name": "a", "type": "hologram" }] });
        assert_eq!(ports(&cfg)[0].ty, PortType::TEXT);
    }

    /// Whatever the ports are, they are the same on both sides — that is what makes the run's own
    /// record of this node the answer, with nothing carried alongside it.
    #[test]
    fn what_arrives_leaves_under_the_same_name() {
        let cfg = json!({ "values": [{ "name": "a", "type": "num" }, { "name": "b", "type": "text" }] });
        let ins: Vec<String> = names(&cfg);
        let outs: Vec<String> = ports(&cfg).iter().map(|p| p.name.as_str().to_string()).collect();
        assert_eq!(ins, outs);
    }
}
