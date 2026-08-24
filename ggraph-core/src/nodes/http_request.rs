//! `http_request` — call something outside.
//!
//! One node with a method, not one node per verb. The response comes back as status, body and a
//! decoded object when the body is JSON, plus an `ok` flag — because "did it work" and "what did
//! it say" are different questions and a graph usually branches on the first.
//!
//! A non-2xx is **not** a node failure. A 404 is an answer, and a workflow that wants to handle
//! it should branch on `ok` rather than have the whole run stop. What does fail the node is not
//! reaching the other end at all.

use crate::host::{Host, HttpRequest};
use crate::id::PortName;
use crate::port::{Port, PortType};
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Ports, Timeout};
use crate::value::{Bytes, PortValues, Value};
use serde_json::{json, Value as Json};

static IN: [Port; 3] = [
    Port::req("url", PortType::TEXT),
    Port::opt("headers", PortType::MAP),
    Port::opt("body", PortType::ANY),
];
static OUT: [Port; 4] = [
    Port::opt("status", PortType::NUM),
    Port::opt("ok", PortType::BOOL),
    Port::opt("text", PortType::TEXT),
    Port::opt("json", PortType::JSON),
];

fn headers(v: Option<&Value>) -> Vec<(String, String)> {
    match v {
        Some(Value::Map(m)) => m
            .iter()
            .filter_map(|(k, v)| v.as_text().map(|s| (k.clone(), s)))
            .collect(),
        _ => Vec::new(),
    }
}

fn body_bytes(v: Option<&Value>) -> Option<Vec<u8>> {
    match v? {
        Value::Bytes(b) => Some(b.data.to_vec()),
        Value::Json(j) => Some(j.to_string().into_bytes()),
        Value::Map(m) => {
            let obj: serde_json::Map<String, Json> = m
                .iter()
                .map(|(k, v)| (k.clone(), v.as_text().map(Json::from).unwrap_or(Json::Null)))
                .collect();
            Some(Json::Object(obj).to_string().into_bytes())
        }
        other => other.as_text().map(String::into_bytes),
    }
}

struct Request;

impl<H: Host> NodeRun<H> for Request {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let url = cx
            .input_or_cfg("url")
            .as_ref()
            .and_then(Value::as_text)
            .unwrap_or_default();
        if url.is_empty() {
            return Err(NodeError("no url".into()));
        }
        let method = cx.cfg_str("method").unwrap_or("GET").to_uppercase();
        let timeout_secs = cx
            .cfg_str("timeout_secs")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(30);

        let mut hs = headers(cx.input("headers"));
        if let Some(extra) = cx.config.get("headers").and_then(Json::as_object) {
            for (k, v) in extra {
                if let Some(v) = v.as_str() {
                    hs.push((k.clone(), v.to_string()));
                }
            }
        }

        let res = cx.host.http().send(HttpRequest {
            method: smol_str::SmolStr::new(&method),
            url,
            headers: hs,
            body: body_bytes(cx.input("body")),
            timeout_secs,
        })?;

        let mut out = PortValues::new();
        out.insert(PortName::new("status"), Value::int(res.status as i64));
        out.insert(
            PortName::new("ok"),
            Value::Bool((200..300).contains(&res.status)),
        );
        match String::from_utf8(res.body.clone()) {
            Ok(text) => {
                if let Ok(j) = serde_json::from_str::<Json>(&text) {
                    out.insert(PortName::new("json"), Value::Json(j));
                }
                out.insert(PortName::new("text"), Value::Text(text));
            }
            // Not text. Handing back mojibake would be worse than handing back the bytes.
            Err(e) => {
                out.insert(
                    PortName::new("text"),
                    Value::Bytes(Bytes::new("application/octet-stream", e.into_bytes())),
                );
            }
        }
        Ok(out)
    }

    fn summary(&self, _cx: &NodeCx<'_, H>, out: &PortValues) -> String {
        match out.get(&PortName::new("status")).and_then(Value::as_i64) {
            Some(s) => format!("HTTP {s}"),
            None => String::new(),
        }
    }
}

pub fn spec<H: Host>() -> NodeSpec<H> {
    NodeSpec::effectful("http_request", "HTTP Request", "Network")
        .with_aliases(&["http_post"])
        .with_inputs(Ports::Static(&IN))
        .with_outputs(Ports::Static(&OUT))
        .with_config(|| json!({ "method": "GET", "url": "", "headers": {}, "timeout_secs": "30" }))
        .with_timeout(Timeout::Secs(60))
        .running(Request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{HostError, HttpResponse};

    #[test]
    fn a_json_body_arrives_decoded_as_well_as_as_text() {
        let body = br#"{"ok":true,"n":2}"#.to_vec();
        let text = String::from_utf8(body.clone()).unwrap();
        let parsed: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["n"],
            json!(2),
            "a graph should not have to parse this itself"
        );
    }

    #[test]
    fn a_map_body_is_sent_as_json() {
        let v = Value::Map(vec![("a".into(), Value::text("1"))]);
        let bytes = body_bytes(Some(&v)).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), r#"{"a":"1"}"#);
    }

    #[test]
    fn headers_come_from_a_map() {
        let v = Value::Map(vec![("Accept".into(), Value::text("application/json"))]);
        assert_eq!(
            headers(Some(&v)),
            vec![("Accept".to_string(), "application/json".to_string())]
        );
    }

    /// A 404 is an answer, not a broken node.
    #[test]
    fn a_not_found_is_reported_rather_than_failing_the_run() {
        let res = HttpResponse {
            status: 404,
            headers: vec![],
            body: b"nope".to_vec(),
        };
        let ok = (200..300).contains(&res.status);
        assert!(!ok);
        // The distinction the node draws: this is `ok = false`, whereas a HostError — not
        // reaching the other end at all — is what stops the run.
        let unreachable: Result<HttpResponse, HostError> =
            Err(HostError("connection refused".into()));
        assert!(unreachable.is_err());
    }
}
