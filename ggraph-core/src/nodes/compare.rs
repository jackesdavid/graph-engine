//! `compare` — is one value greater than, equal to, less than another?
//!
//! One node with an operator, rather than six nodes named after their operators. The engine this
//! came from had `num_lt`, `num_lte`, `num_gt`, `num_gte`, `num_eq`, `num_ne` as separate kinds,
//! which meant changing `>` to `>=` was deleting a node and rewiring three edges.
//!
//! Two things it does that the original did not, both of which were quiet bugs:
//!
//! - **Missing operands do not become zero.** `unwrap_or(0)` on an absent input makes
//!   `count > 0` answer `false` when the count could not be read — indistinguishable from a real
//!   zero. Here an absent operand produces no `result`, which the branch's third arm routes as
//!   `unknown`.
//! - **Floats compare as floats.** The original coerced to `i64`, so `1.5 > 1.2` compared 1 to 1
//!   and answered `false`.

use crate::host::Host;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports};
use crate::value::{PortValues, Value};
use serde_json::json;

static IN: [Port; 2] = [Port::opt("a", PortType::ANY), Port::opt("b", PortType::ANY)];
static OUT: [Port; 1] = [Port::opt("result", PortType::BOOL)];

struct Compare;

impl<H: Host> NodeRun<H> for Compare {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let op = cx.cfg_str("operator").unwrap_or("==");
        let (Some(a), Some(b)) = (cx.input_or_cfg("a"), cx.input_or_cfg("b")) else {
            // Deliberately not an error and deliberately not `false`: the absence flows on as an
            // absent result, and the graph decides what to do about it.
            return Ok(PortValues::new());
        };

        let verdict = match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => match op {
                "==" => Some(x == y),
                "!=" => Some(x != y),
                "<" => Some(x < y),
                "<=" => Some(x <= y),
                ">" => Some(x > y),
                ">=" => Some(x >= y),
                _ => None,
            },
            // Not numbers: only equality is meaningful. Ordering text by `<` is a locale
            // question with no right answer at this layer.
            _ => match op {
                "==" => Some(a == b),
                "!=" => Some(a != b),
                _ => None,
            },
        };

        let Some(verdict) = verdict else {
            return Err(NodeError::new(format!(
                "cannot apply {op:?} to {} and {}",
                a.port_type(),
                b.port_type()
            )));
        };

        let mut out = PortValues::new();
        out.insert(crate::id::PortName::new("result"), Value::Bool(verdict));
        Ok(out)
    }
}

pub fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::pure("compare", "Compare", "Logic")
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "operator": "==", "a": "", "b": "" }))
        .running(Compare)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::id::PortName;
    use crate::spec::Behavior;
    use serde_json::Value as Json;

    fn cmp(a: Option<Value>, b: Option<Value>, op: &str) -> Option<bool> {
        let s: NodeSpec<TestHost> = spec();
        let cfg: Json = json!({ "operator": op });
        let mut inputs = PortValues::new();
        if let Some(a) = a {
            inputs.insert(PortName::new("a"), a);
        }
        if let Some(b) = b {
            inputs.insert(PortName::new("b"), b);
        }
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
        };
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        r.run(&cx)
            .unwrap()
            .get(&PortName::new("result"))
            .and_then(Value::as_bool)
    }

    #[test]
    fn numbers_compare_as_numbers() {
        assert_eq!(
            cmp(Some(Value::int(5)), Some(Value::int(3)), ">"),
            Some(true)
        );
        assert_eq!(
            cmp(Some(Value::int(3)), Some(Value::int(3)), ">="),
            Some(true)
        );
    }

    #[test]
    fn floats_are_not_rounded_to_compare() {
        assert_eq!(
            cmp(Some(Value::float(1.5)), Some(Value::float(1.2)), ">"),
            Some(true),
            "coercing to i64 first compares 1 to 1 and answers false"
        );
    }

    #[test]
    fn a_missing_operand_produces_no_answer_rather_than_a_wrong_one() {
        assert_eq!(
            cmp(None, Some(Value::int(0)), ">"),
            None,
            "defaulting to zero makes 'could not read the count' identical to 'the count is zero'"
        );
    }

    #[test]
    fn text_compares_for_equality_only() {
        assert_eq!(
            cmp(Some(Value::text("a")), Some(Value::text("a")), "=="),
            Some(true)
        );
        let s: NodeSpec<TestHost> = spec();
        let cfg = json!({ "operator": "<" });
        let mut inputs = PortValues::new();
        inputs.insert(PortName::new("a"), Value::text("apple"));
        inputs.insert(PortName::new("b"), Value::text("banana"));
        let host = TestHost::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
        };
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        assert!(
            r.run(&cx).is_err(),
            "ordering text is a locale question, and answering it here would answer it wrongly \
             for someone"
        );
    }

    #[test]
    fn a_number_that_arrived_as_text_is_still_a_number() {
        assert_eq!(
            cmp(Some(Value::text("10")), Some(Value::text("9")), ">"),
            Some(true),
            "config literals and HTTP fields arrive as text; refusing them moves the parse into \
             every node"
        );
    }
}
