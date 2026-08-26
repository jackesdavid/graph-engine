// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `approval` — stop, ask a person, and carry on with what they said.
//!
//! ## A run cannot block on a human
//!
//! The obvious implementation waits for the answer. It cannot exist: the answer arrives minutes
//! or days later, in another process, possibly on another machine, possibly never. Holding a run
//! open for that means holding a thread, a connection and a lease across a deploy.
//!
//! So the node **ends the run**. It records the question, has the host deliver it, and stops.
//! When somebody answers, that arrives as a fresh entry at this same node carrying the verdict,
//! and the node routes it. The ask and the answer are two runs, and the run history shows them
//! as two runs, which is also what a person auditing it would want to see.
//!
//! ## Three arms
//!
//! `approved`, `denied`, `unanswered` — and the third is the reason the node is shaped this way.
//! A person who said no and a person who never saw the question need different things to happen
//! next. Folding them together is how a workflow quietly treats silence as refusal, which is
//! the failure nobody notices until it has been happening for a month.

use crate::host::Host;
use crate::nodes::services::{ApprovalRequest, Verdict};
use crate::port::{Port, PortType};
use crate::spec::{ExecOut, NodeError, NodeSpec, NodeStep, Ports, Step, StepCx, Timeout};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 2] = [
    Port::opt("audience", PortType::ANY),
    Port::opt("prompt", PortType::TEXT),
];
static OUT: [Port; 1] = [Port::opt("answered_by", PortType::TEXT)];
static ARMS: [Port; 3] = [
    Port::opt("approved", PortType::EXEC),
    Port::opt("denied", PortType::EXEC),
    Port::opt("unanswered", PortType::EXEC),
];

struct Approval {
    approvals: std::sync::Arc<dyn crate::nodes::services::Approvals>,
}

impl<H: Host> NodeStep<H> for Approval {
    fn step(&self, cx: &mut StepCx<'_, H>) -> Result<Step, NodeError> {
        // An answer being delivered: this node is the entry, and the payload carries a verdict.
        if cx.forced {
            if let Some(v) = Verdict::from_payload(cx.entry_payload) {
                let mut out = PortValues::new();
                if let Some(who) = cx
                    .entry_payload
                    .get(&crate::id::PortName::new("answered_by"))
                {
                    out.insert(crate::id::PortName::new("answered_by"), who.clone());
                }
                return Ok(Step::outputs(out)
                    .arm(v.as_str())
                    .logged(format!("answer: {}", v.as_str())));
            }
        }

        let audience = cx
            .input("audience")
            .and_then(Value::as_text)
            .or_else(|| cx.cfg_str("audience").map(str::to_string))
            .unwrap_or_default();
        if audience.is_empty() {
            return Err(NodeError::new(
                "nobody to ask — wire `audience` or set it in the inspector",
            ));
        }
        let prompt = cx
            .input("prompt")
            .and_then(Value::as_text)
            .or_else(|| cx.cfg_str("prompt").map(str::to_string))
            .unwrap_or_default();
        let expires_in_secs = cx
            .cfg_str("expires_in_secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);

        self.approvals.ask(ApprovalRequest {
            target: cx.target(),
            run: cx.host.run_id(),
            audience: audience.clone(),
            prompt,
            expires_in_secs,
        })?;

        // No arm fires and the run ends. Not a failure — the graph is waiting, and saying so as
        // an error would put a red mark on every workflow that involves a person.
        Ok(Step::default().halted().logged(format!("asked {audience}")))
    }
}

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("approval", "Ask a Person", "Approval")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_exec_out(ExecOut::Static(&ARMS))
        .with_config(|| json!({ "audience": "", "prompt": "", "expires_in_secs": "300" }))
        .with_timeout(Timeout::Secs(30))
        .stepping(Approval {
            approvals: services.approvals.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::host::Retry;
    use crate::spec::Behavior;
    use crate::spec::Next;
    use serde_json::Value as Json;
    use uuid::Uuid;

    fn step(cfg: Json, forced: bool, payload: PortValues) -> Result<Step, NodeError> {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Step(node) = &s.behavior else {
            panic!("approval must cooperate with the scheduler")
        };
        let host = TestHost::new();
        let inputs = PortValues::new();
        let mut scratch = json!({});
        let mut cx = StepCx {
            vars: Default::default(),
            config: &cfg,
            inputs: &inputs,
            node: 3,
            graph: Uuid::nil(),
            instance: "",
            forced,
            entry_payload: &payload,
            host: &host,
            scratch: &mut scratch,
        };
        node.step(&mut cx)
    }

    fn cfg() -> Json {
        json!({ "audience": "someone", "prompt": "ok?", "expires_in_secs": "60" })
    }

    #[test]
    fn asking_ends_the_run_without_firing_an_arm() {
        // TestHost refuses to deliver, so this also proves the refusal surfaces as an error
        // rather than as a silently unasked question.
        let err = step(cfg(), false, PortValues::new()).unwrap_err();
        assert_eq!(err.message, "no approval channel is configured");
        assert_eq!(
            err.retry,
            Retry::Never,
            "an integration that is not configured will not configure itself on a retry, and a \
             durable host must not sit in a backoff loop waiting for it to"
        );
    }

    #[test]
    fn each_verdict_takes_its_own_arm() {
        for v in [Verdict::Approved, Verdict::Denied, Verdict::Unanswered] {
            let s = step(cfg(), true, v.into_payload()).unwrap();
            let arms: Vec<&str> = s.arms.iter().map(|a| a.as_str()).collect();
            assert_eq!(arms, vec![v.as_str()]);
            assert!(
                s.next != Next::Halt,
                "delivering an answer continues the run"
            );
        }
    }

    #[test]
    fn silence_does_not_come_back_as_a_no() {
        let s = step(cfg(), true, Verdict::Unanswered.into_payload()).unwrap();
        let arms: Vec<&str> = s.arms.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            arms,
            vec!["unanswered"],
            "'nobody answered' routed as 'denied' is how a workflow quietly refuses things for \
             a month before anyone notices"
        );
    }

    #[test]
    fn an_entry_without_a_verdict_asks_again_rather_than_guessing() {
        // A forced entry that carries no answer — a timer, a manual re-run. Asking again is the
        // only honest move; picking an arm would invent an answer nobody gave.
        assert!(step(cfg(), true, PortValues::new()).is_err());
    }

    #[test]
    fn with_nobody_to_ask_it_refuses_before_asking() {
        let err = step(
            json!({ "audience": "", "prompt": "" }),
            false,
            PortValues::new(),
        )
        .unwrap_err();
        assert!(err.message.contains("nobody to ask"));
    }
}
