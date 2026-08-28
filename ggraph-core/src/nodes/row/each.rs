// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `for_each_row` — the body once per row of a table.
//!
//! [`for_each`](crate::nodes::for_each) already iterates anything, and a table wired into it works.
//! What it cannot do is say what came out: its `item` is `any`, so the row arrives untyped and
//! everything downstream has to accept everything. That is the pin this exists to type.
//!
//! Two nodes rather than a mode on one, for the reason the rounding pair exists: a node whose
//! output type depends on what it was handed is a node whose wires cannot be checked while they
//! are being drawn.
//!
//! The index lives in the step's `scratch` — run-scoped and gone when the run ends. Not in durable
//! state: a loop that resumed mid-iteration after a restart would repeat whatever the body already
//! did, which for a body that sends mail is not a recoverable mistake.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeError, NodeSpec, NodeStep, Ports, Step, StepCx, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::req("table", PortType::TABLE)];
static OUT: [Port; 2] = [
    Port::opt("row", PortType::TABLE_ROW),
    Port::opt("index", PortType::NUM),
];
static ARMS: [Port; 2] = [
    Port::opt("loop_body", PortType::EXEC),
    Port::opt("completed", PortType::EXEC),
];

struct ForEachRow;

impl<H: Host> NodeStep<H> for ForEachRow {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        let list = crate::table::rows(cx.input("table"));
        let i = cx.scratch.get("i").and_then(Json::as_u64).unwrap_or(0) as usize;

        if i >= list.len() {
            // Reset, so a loop reached twice in one run — inside an outer loop — starts over
            // rather than reporting itself finished.
            *cx.scratch = json!({});
            return Ok(Step::outputs(PortValues::new())
                .arm("completed")
                .logged(format!("{} row(s)", list.len())));
        }

        cx.scratch["i"] = json!(i + 1);

        let mut out = PortValues::new();
        out.insert(PortName::new("row"), list[i].clone());
        out.insert(PortName::new("index"), Value::int(i as i64));
        Ok(Step::outputs(out).arm("loop_body").reentering())
    }
}

pub(super) fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("for_each_row", "For each row", "Data")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_exec_out(ExecOut::Static(&ARMS))
        .with_config(|| json!({}))
        .with_timeout(Timeout::Inline)
        .stepping(ForEachRow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::{Behavior, Next};
    use uuid::Uuid;

    fn row(doc: &str) -> Value {
        Value::Map(vec![("document".into(), Value::text(doc))])
    }

    /// Drive it the way a scheduler would: step until it stops re-entering.
    fn drive(table: Option<Value>) -> (Vec<String>, Vec<String>) {
        let s: NodeSpec<TestHost> = spec();
        let Behavior::Step(node) = &s.behavior else {
            panic!("a loop must cooperate with the scheduler")
        };
        let host = TestHost::new();
        let cfg = json!({});
        let mut scratch = json!({});
        let mut inputs = PortValues::new();
        if let Some(v) = table {
            inputs.insert(PortName::new("table"), v);
        }
        let empty = PortValues::new();

        let (mut seen, mut arms) = (Vec::new(), Vec::new());
        for _ in 0..50 {
            let mut cx = StepCx {
                vars: Default::default(),
                config: &cfg,
                inputs: &inputs,
                node: 1,
                graph: Uuid::nil(),
                instance: "",
                forced: false,
                entry_payload: &empty,
                host: &host,
                scratch: &mut scratch,
            };
            let step = node.step(&mut cx).unwrap();
            arms.extend(step.arms.iter().map(|a| a.as_str().to_string()));
            if let Some(v) = step.outputs.get(&PortName::new("row")) {
                seen.push(v.summary());
            }
            if step.next != Next::Reenter {
                break;
            }
        }
        (seen, arms)
    }

    #[test]
    fn every_row_takes_the_body_once_and_then_it_completes() {
        let (seen, arms) = drive(Some(Value::List(vec![row("a.pdf"), row("b.pdf")])));
        assert_eq!(seen.len(), 2);
        assert_eq!(arms, vec!["loop_body", "loop_body", "completed"]);
    }

    /// An empty table completes without entering the body. A loop that ran once over nothing is
    /// how a graph comes to act on a row that was never there.
    #[test]
    fn an_empty_table_goes_straight_to_completed() {
        let (seen, arms) = drive(Some(Value::List(vec![])));
        assert!(seen.is_empty());
        assert_eq!(arms, vec!["completed"]);
    }
}
