// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `for_each` — run the body once per item, then carry on.
//!
//! The first node that needs more than "inputs in, outputs out": it fires an arm, gets control
//! back, and fires it again. That is why [`NodeStep`] exists.
//!
//! The index lives in the step's `scratch` — run-scoped, private to this node, gone when the run
//! ends. Not in durable state: a loop that resumed mid-iteration after a restart would double
//! whatever the body already did, which for a body that sends mail is not a recoverable mistake.

use crate::host::Host;
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeError, NodeSpec, NodeStep, Ports, Step, StepCx, Timeout};
use crate::value::{PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 1] = [Port::opt("items", PortType::ANY)];
static OUT: [Port; 2] = [
    Port::opt("item", PortType::ANY),
    Port::opt("index", PortType::NUM),
];
static ARMS: [Port; 2] = [
    Port::opt("loop_body", PortType::EXEC),
    Port::opt("completed", PortType::EXEC),
];

/// What to iterate. A list is a list; text is split on commas, because a list typed into an
/// inspector field is the common case and making people wire a splitter node for it is friction
/// with no payoff.
///
/// A table iterates its ROWS. Its value is a map of columns and rows, so the plain map branch would
/// have walked those two entries — a loop that runs twice over nothing anybody meant, drawable
/// because this port takes anything.
fn items(cx: &StepCx<'_, impl Host>) -> Vec<Value> {
    match cx.input("items") {
        Some(Value::List(v)) => v.clone(),
        Some(v @ Value::Map(_)) if is_table(v) => crate::table::rows(Some(v)),
        Some(Value::Map(m)) => m.iter().map(|(_, v)| v.clone()).collect(),
        Some(Value::Text(s)) => split(s),
        Some(other) => vec![other.clone()],
        None => cx.cfg_str("items").map(split).unwrap_or_default(),
    }
}

/// A map that is a table: it has both of the keys one has, and nothing else claims that pair.
fn is_table(v: &Value) -> bool {
    let Value::Map(pairs) = v else { return false };
    let has = |k: &str| pairs.iter().any(|(name, _)| name == k);
    has("columns") && has("rows")
}

fn split(s: &str) -> Vec<Value> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(Value::text)
        .collect()
}

struct ForEach;

impl<H: Host> NodeStep<H> for ForEach {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        let list = items(cx);
        let i = cx.scratch.get("i").and_then(Json::as_u64).unwrap_or(0) as usize;

        if i >= list.len() {
            // Reset, so a loop reached twice in one run — inside an outer loop — starts over
            // rather than reporting itself finished.
            *cx.scratch = json!({});
            return Ok(Step::outputs(PortValues::new())
                .arm("completed")
                .logged(format!("{} item(s)", list.len())));
        }

        cx.scratch["i"] = json!(i + 1);

        let mut out = PortValues::new();
        out.insert(PortName::new("item"), list[i].clone());
        out.insert(PortName::new("index"), Value::int(i as i64));
        Ok(Step::outputs(out).arm("loop_body").reentering())
    }
}

pub fn spec<H: Host>(_services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("for_each", "For Each", "Control")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_exec_out(crate::spec::ExecOut::Static(&ARMS))
        .with_config(|| json!({ "items": "" }))
        .with_timeout(Timeout::Inline)
        .stepping(ForEach)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;
    use crate::spec::Next;
    use uuid::Uuid;

    /// Drive the node the way a scheduler would: step it until it stops re-entering.
    fn drive(items_cfg: Json, wired: Option<Value>) -> (Vec<String>, Vec<String>) {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            panic!("for_each must cooperate with the scheduler")
        };
        let host = TestHost::new();
        let mut scratch = json!({});
        let mut inputs = PortValues::new();
        if let Some(v) = wired {
            inputs.insert(PortName::new("items"), v);
        }
        let empty = PortValues::new();

        let mut seen = Vec::new();
        let mut arms = Vec::new();
        for _ in 0..50 {
            let mut cx = StepCx {
                vars: Default::default(),
                config: &items_cfg,
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
            if let Some(v) = step.outputs.get(&PortName::new("item")) {
                seen.push(v.summary());
            }
            if !(step.next == Next::Reenter) {
                break;
            }
        }
        (seen, arms)
    }

    #[test]
    fn the_body_runs_once_per_item_then_control_moves_on() {
        let (seen, arms) = drive(json!({ "items": "alpha, beta, gamma" }), None);
        assert_eq!(seen, vec!["alpha", "beta", "gamma"]);
        assert_eq!(
            arms,
            vec!["loop_body", "loop_body", "loop_body", "completed"],
            "completed fires once, and after the body — an off-by-one here is invisible to \
             every other test"
        );
    }

    #[test]
    fn an_empty_list_completes_without_running_the_body() {
        let (seen, arms) = drive(json!({ "items": "" }), None);
        assert!(seen.is_empty());
        assert_eq!(arms, vec!["completed"]);
    }

    #[test]
    fn a_wired_list_wins_over_the_configured_one() {
        let (seen, _) = drive(
            json!({ "items": "ignored" }),
            Some(Value::List(vec![Value::int(1), Value::int(2)])),
        );
        assert_eq!(seen, vec!["1", "2"]);
    }

    /// A table iterates its rows. Its value is a map of columns and rows, so without this the loop
    /// walked those two entries — twice round, over nothing anybody meant.
    #[test]
    fn a_table_iterates_its_rows() {
        let row = |d: &str| Value::Map(vec![("document".into(), Value::text(d))]);
        let cols = [crate::port::Column::new(
            crate::id::PortName::new("document"),
            crate::port::PortType::TEXT,
        )];
        let (seen, _) = drive(
            json!({}),
            Some(crate::table::make(&cols, vec![row("a.pdf"), row("b.pdf")])),
        );
        assert_eq!(seen.len(), 2, "two rows, not two entries of the table's map");
    }

    #[test]
    fn finishing_resets_so_an_outer_loop_can_run_it_again() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "items": "one" });
        let inputs = PortValues::new();
        let empty = PortValues::new();
        let mut scratch = json!({});

        let mut run_once = || {
            let mut arms = Vec::new();
            for _ in 0..10 {
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
                if !(step.next == Next::Reenter) {
                    break;
                }
            }
            arms
        };
        assert_eq!(run_once(), vec!["loop_body", "completed"]);
        assert_eq!(
            run_once(),
            vec!["loop_body", "completed"],
            "a loop nested in another loop must start over on the second pass, not report \
             itself already finished"
        );
    }
}
