// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Services the STANDARD NODES need — not the scheduler.
//!
//! Nothing in `exec.rs` calls any of these. They are here because the node library this crate
//! ships needs an approval channel, a network, a model and a table store, and a product that
//! registers those nodes has to supply them. A product that registers none of them owes nothing:
//! see [`Services::none`].
//!
//! They lived on `Host` once, which meant every consumer implemented four capabilities it might
//! never use. The first real one answered all four with a struct called `NotRouted`, whose every
//! method said "this reaches the product's own node instead". A stub that large, written on day
//! one, is the design saying something.

use crate::host::HostError;
use crate::host::NodeTarget;
use crate::id::PortName;
use crate::value::{PortValues, Value};
use smol_str::SmolStr;
use std::sync::Arc;
use uuid::Uuid;

/// Asking a person, and finding out later what they said.
///
/// The hard part of pause-for-approval is not the pause — it is that the answer arrives in a
/// different process, possibly on a different pod, possibly never. So `ask` returns immediately
/// with a handle and **the run ends**; a settled request is delivered later as a fresh entry at
/// the same node. A run cannot block on a human.
pub trait Approvals: Send + Sync {
    /// Record a pending question and deliver it to whoever must answer. Returns the request id.
    fn ask(&self, req: ApprovalRequest) -> Result<Uuid, HostError>;
}

#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    pub target: NodeTarget,
    pub run: Uuid,
    /// Who to ask. Opaque to the core — a phone, an address, a role.
    pub audience: String,
    pub prompt: String,
    pub expires_in_secs: u64,
}

/// The port name an entry payload carries a verdict on.
///
/// A resumption is an ordinary entry that happens to carry an answer, so the answer travels the
/// same way every other value does. One well-known name rather than a side channel: a side
/// channel is a thing the run log cannot see.
pub const VERDICT_PORT: &str = "__verdict";

/// How an approval came back.
///
/// Three states, not two, and the third is the reason this type exists: `Denied` is an *answer*,
/// `Unanswered` is the absence of one. A person who said no and a person who never saw the
/// question require different things to happen next, and folding them together is how a
/// workflow quietly treats silence as refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Approved,
    Denied,
    Unanswered,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Approved => "approved",
            Verdict::Denied => "denied",
            Verdict::Unanswered => "unanswered",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        match s {
            "approved" => Some(Verdict::Approved),
            "denied" => Some(Verdict::Denied),
            "unanswered" => Some(Verdict::Unanswered),
            _ => None,
        }
    }

    /// The verdict an entry payload is delivering, if it is delivering one.
    pub fn from_payload(payload: &PortValues) -> Option<Verdict> {
        payload
            .get(VERDICT_PORT)
            .and_then(Value::as_text)
            .and_then(|s| Verdict::parse(&s))
    }

    /// Build the payload that delivers this verdict back to a waiting node.
    pub fn into_payload(self) -> PortValues {
        let mut p = PortValues::new();
        p.insert(PortName::new(VERDICT_PORT), Value::text(self.as_str()));
        p
    }
}

/// An outbound request. A conector, behind a trait, so the core has no notion of a network.
pub trait Http: Send + Sync {
    fn send(&self, req: HttpRequest) -> Result<HttpResponse, HostError>;
}

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: SmolStr,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout_secs: u64,
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A language model, as the graph uses one: ask a question, get an answer of a declared shape.
///
/// Narrow on purpose. This is not a chat client and must not grow into one — the products that
/// need conversations, tool calling and streaming build those on their own side and expose the
/// three shapes a *node* actually needs.
pub trait Llm: Send + Sync {
    /// Free-form text.
    fn ask_text(&self, req: LlmRequest) -> Result<String, HostError>;
    /// A decision. `None` means the model would not commit — which is a third branch, not a no.
    fn ask_bool(&self, req: LlmRequest) -> Result<Option<bool>, HostError>;
    /// One of `choices`, or `None` for the same reason.
    fn classify(&self, req: LlmRequest, choices: &[String]) -> Result<Option<String>, HostError>;
}

#[derive(Clone, Debug)]
pub struct LlmRequest {
    pub prompt: String,
    /// Optional attachment — an image, a document. Products without vision ignore it.
    pub attachment: Option<crate::value::Bytes>,
    pub timeout_secs: u64,
}

/// Named tables a graph writes to and reads back, outliving the run that made them.
pub trait TableStore: Send + Sync {
    fn list(&self) -> Result<Vec<String>, HostError>;
    fn read(&self, name: &str) -> Result<Vec<Vec<(String, Value)>>, HostError>;
    fn row_count(&self, name: &str) -> Result<u64, HostError>;
    fn append(&self, name: &str, row: &[(String, Value)]) -> Result<(), HostError>;
    fn set_cell(&self, name: &str, row: u64, column: &str, v: &Value) -> Result<(), HostError>;
    fn delete_row(&self, name: &str, row: u64) -> Result<(), HostError>;
    fn clear(&self, name: &str) -> Result<(), HostError>;
}

/// What the standard node set needs from the world.
///
/// Handed to [`register_all`](super::register_all) rather than reached through `Host`, so a
/// product that wants the table nodes but not the model ones says exactly that.
#[derive(Clone)]
pub struct Services {
    pub approvals: Arc<dyn Approvals>,
    pub http: Arc<dyn Http>,
    pub llm: Arc<dyn Llm>,
    pub tables: Arc<dyn TableStore>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Services")
    }
}

impl Default for Services {
    fn default() -> Self {
        Services::none()
    }
}

impl Services {
    /// None of them. Every node needing one refuses with a message naming what is missing, which
    /// is what a graph author needs to read — rather than a panic, or a silence.
    pub fn none() -> Self {
        Services {
            approvals: Arc::new(Absent),
            http: Arc::new(Absent),
            llm: Arc::new(Absent),
            tables: Arc::new(Absent),
        }
    }
}

/// Refuses, and says what would have been needed.
#[derive(Debug)]
pub struct Absent;

impl Approvals for Absent {
    fn ask(&self, _: ApprovalRequest) -> Result<Uuid, HostError> {
        Err(HostError::permanent("no approval channel is configured"))
    }
}

impl Http for Absent {
    fn send(&self, _: HttpRequest) -> Result<HttpResponse, HostError> {
        Err(HostError::permanent("no network is configured"))
    }
}

impl Llm for Absent {
    fn ask_text(&self, _: LlmRequest) -> Result<String, HostError> {
        Err(HostError::permanent("no model is configured"))
    }
    fn ask_bool(&self, _: LlmRequest) -> Result<Option<bool>, HostError> {
        Err(HostError::permanent("no model is configured"))
    }
    fn classify(&self, _: LlmRequest, _: &[String]) -> Result<Option<String>, HostError> {
        Err(HostError::permanent("no model is configured"))
    }
}

impl TableStore for Absent {
    fn list(&self) -> Result<Vec<String>, HostError> {
        Ok(Vec::new())
    }
    fn read(&self, _: &str) -> Result<Vec<Vec<(String, Value)>>, HostError> {
        Err(HostError::permanent("no table store is configured"))
    }
    fn row_count(&self, _: &str) -> Result<u64, HostError> {
        Ok(0)
    }
    fn append(&self, _: &str, _: &[(String, Value)]) -> Result<(), HostError> {
        Err(HostError::permanent("no table store is configured"))
    }
    fn set_cell(&self, _: &str, _: u64, _: &str, _: &Value) -> Result<(), HostError> {
        Err(HostError::permanent("no table store is configured"))
    }
    fn delete_row(&self, _: &str, _: u64) -> Result<(), HostError> {
        Err(HostError::permanent("no table store is configured"))
    }
    fn clear(&self, _: &str) -> Result<(), HostError> {
        Err(HostError::permanent("no table store is configured"))
    }
}
