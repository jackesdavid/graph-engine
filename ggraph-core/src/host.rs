//! The seam. Everything the engine needs from the world outside it, as traits.
//!
//! This is why `ggraph-core` has four dependencies and no tokio: the scheduler never opens a
//! socket, never touches a database and never reads a clock directly. It asks the [`Host`].
//!
//! The seam is small, and it is smaller than it first looks. The engine this was extracted from
//! threaded a fourteen-field context through every node; the *scheduler* used six of those
//! fields, and of the six, the tenant id was only ever an argument to a store call and one more
//! was only ever used inside a single node. What is actually needed is four capabilities and a
//! run id.
//!
//! Note what is NOT here: no tenant. A [`StateStore`] is constructed already scoped to its
//! tenant, so the scheduler cannot address another one — not by mistake, not by a shifted
//! argument, not at all. Multi-tenancy that depends on every call site remembering to pass an
//! id is multi-tenancy that fails once.

use crate::id::PortName;
use crate::value::{PortValues, Value};
use serde_json::Value as Json;
use smol_str::SmolStr;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Whether the same call might work if it were tried again.
///
/// This exists so retry is a decision somebody made rather than a guess. Without it a retry
/// policy has to match on error text, which is how "connection refused" gets retried forever
/// alongside "that table does not exist".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Retry {
    /// The world was busy, unreachable or slow. Trying again later is reasonable.
    #[default]
    Maybe,
    /// It will fail the same way every time — a missing table, a malformed request, a
    /// credential that is wrong rather than expired. Retrying only delays the report.
    Never,
}

/// Something the world refused to do. The engine surfaces these; it does not interpret them.
///
/// Defaults to [`Retry::Maybe`], because a *host* failure is usually about the world rather
/// than about the request. A host that knows better says so with [`HostError::permanent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub retry: Retry,
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        HostError {
            retry: Retry::Maybe,
            message: message.into(),
        }
    }

    /// It will fail the same way next time.
    pub fn permanent(message: impl Into<String>) -> Self {
        HostError {
            retry: Retry::Never,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

impl From<String> for HostError {
    fn from(s: String) -> Self {
        HostError::new(s)
    }
}

impl From<&str> for HostError {
    fn from(s: &str) -> Self {
        HostError::new(s)
    }
}

/// Which node of which graph, in which instance. The address a durable wake-up is delivered to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeTarget {
    pub graph: Uuid,
    pub node: u32,
    /// Which instance of the graph this belongs to. `""` is the single-instance case, and is
    /// what a graph that has never heard of instances stores.
    pub instance: SmolStr,
}

/// Which run. Distinct from [`NodeTarget`]: a target may be re-entered by many runs over time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub graph: Uuid,
    pub run: Uuid,
    pub instance: SmolStr,
}

/// Which slot of a node's durable state is being addressed.
///
/// Three, because they have three different lifetimes and merging them was a bug waiting to
/// happen: `State` is the node's own machine (armed, cooling down), `Values` is what it last
/// produced, `Variables` is the run's variable map at a suspension point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    State,
    Values,
    Variables,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateKey {
    pub target: NodeTarget,
    pub slot: Slot,
}

/// Durable per-node state.
///
/// The three conditional operations are not conveniences — they are the whole point. Two pods
/// run the same graph; `try_arm` is how exactly one of them opens a window, and
/// `try_disarm_expired` is how exactly one of them closes it. A `get` followed by a `set` is a
/// race that shows up as a duplicated notification once a month and is never reproducible.
pub trait StateStore: Send + Sync {
    fn get(&self, key: &StateKey) -> Option<Json>;
    fn set(&self, key: &StateKey, value: &Json);
    fn clear(&self, key: &StateKey);

    /// Move to armed, only if not already armed. `true` means this caller won.
    ///
    /// Deadlines are epoch seconds rather than a formatted timestamp: a string deadline means
    /// two components have to agree on a format and a zone, and the day they disagree the window
    /// closes at the wrong time with nothing in the logs to say why.
    fn try_arm(&self, key: &StateKey, expires_at: i64) -> bool;

    /// Push an armed window's deadline out. A no-op if not armed.
    fn extend(&self, key: &StateKey, expires_at: i64);

    /// Move an armed-and-expired window to idle. `true` means this caller won.
    ///
    /// `now` comes from the engine's clock. A store whose backend has an authoritative clock —
    /// a database doing this as one conditional UPDATE — should prefer its own, since that is
    /// the only one every pod agrees on.
    fn try_disarm_expired(&self, key: &StateKey, now: i64) -> bool;
}

/// What the engine reports as it runs: to a live editor, to a run log, to a trace.
///
/// Every method has a default no-op, so a product that wants none of it implements nothing and
/// a test host stays three lines long.
pub trait Observer: Send + Sync {
    /// A node is about to execute.
    fn node_started(&self, _node: u32) {}
    /// A node finished, with a one-line summary and how long it took.
    fn node_finished(&self, _node: u32, _summary: &str, _elapsed_ms: u128) {}
    /// A node emitted a cross-graph event.
    fn emitted(&self, _event: &str, _payload: &PortValues) {}
    /// A node produced something for a person to look at right now.
    fn ui(&self, _node: u32, _event: UiEvent) {}
}

/// Something a node wants shown, live, while the graph runs.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Value { label: String, value: Value },
    Image { label: String, bytes: Vec<u8> },
}

/// Turns a configuration literal into a value on an **unwired** input port.
///
/// Most graph editors let a port be either wired or typed into an inspector, and the second is
/// by far the more common. The engine cannot interpret those itself: what `"front door"` means
/// on a port of the product's own type is the product's business, and answering may take a
/// lookup — a device id resolved against a device store, a table id against a run-start
/// snapshot.
///
/// The engine asks **before** anything checks whether a required input is present, and that
/// ordering is the point rather than an implementation detail. A scheduler that validated first
/// would decide the branch was dead, skip everything under it, and report the run `ok` — a
/// failure with nothing red anywhere.
pub trait Literals: Send + Sync {
    /// `None` leaves the port unfilled, which downstream reads as "not provided".
    fn read(&self, kind: &crate::NodeId, port: &crate::port::Port, config: &Json) -> Option<Value>;
}

/// Nothing is read from configuration: every value arrives on a wire, or the node reads its own
/// config through [`NodeCx::input_or_cfg`](crate::spec::NodeCx::input_or_cfg). Fine for a small
/// node set; it stops scaling at roughly the point where every node has to remember to do it.
#[derive(Debug)]
pub struct NoLiterals;

impl Literals for NoLiterals {
    fn read(&self, _: &crate::NodeId, _: &crate::port::Port, _: &Json) -> Option<Value> {
        None
    }
}

/// Where values too large to inline are kept.
///
/// The codec calls this when an [`ExternValue`](crate::ExternValue) or a [`Bytes`](crate::Bytes)
/// needs to survive a restart. [`disabled`](Disabled) is a legitimate implementation: it reports
/// `enabled() == false` and the codec drops such values rather than failing the run.
pub trait ValueIo: Send + Sync {
    fn enabled(&self) -> bool;
    fn put(&self, bytes: &[u8], mime: &str) -> Result<String, HostError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, HostError>;
}

/// A blob store that isn't there. Values that need one are dropped, and the drop is recorded.
#[derive(Debug)]
pub struct Disabled;

impl ValueIo for Disabled {
    fn enabled(&self) -> bool {
        false
    }
    fn put(&self, _: &[u8], _: &str) -> Result<String, HostError> {
        Err(HostError::permanent("no blob store configured"))
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError::permanent("no blob store configured"))
    }
}

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

/// Everything the engine needs from its product.
///
/// One trait, one type parameter, chosen once at the binary's root. Nodes written against the
/// concrete host reach whatever else that host offers — cameras, a corpus, a face index —
/// without any of it appearing here.
pub trait Host: Send + Sync + Clone + 'static {
    /// The product's per-graph policy. See [`Graph`](crate::Graph).
    type Meta: crate::graph::GraphMeta;

    fn state(&self) -> &dyn StateStore;
    fn io(&self) -> &dyn ValueIo;
    fn observer(&self) -> &dyn Observer;
    fn approvals(&self) -> &dyn Approvals;
    fn http(&self) -> &dyn Http;
    fn llm(&self) -> &dyn Llm;
    fn tables(&self) -> &dyn TableStore;

    /// The run's variable map. Mutable, shared, and scoped to this run.
    fn vars(&self) -> &Mutex<HashMap<String, Value>>;

    fn run_id(&self) -> Uuid;

    /// Seconds since the epoch. The engine never calls the clock directly, so a test host can
    /// make a time window pass instantly instead of sleeping through it.
    fn now_epoch_secs(&self) -> i64;

    /// Which instance of this graph a signal belongs to.
    ///
    /// This is the whole of instance scoping, from the core's point of view: an opaque string.
    /// "One run per camera" is a rule about cameras and lives in the product; the engine only
    /// needs to know that two signals with different keys do not share node state.
    fn instance_key(&self, _meta: &Self::Meta, _payload: &PortValues) -> SmolStr {
        SmolStr::default()
    }

    /// How a configuration literal becomes a value on an unwired input port.
    ///
    /// Defaults to [`NoLiterals`], under which nodes read their own configuration themselves.
    fn literals(&self) -> &dyn Literals {
        &NoLiterals
    }

    /// Re-enter `target` at `at_epoch_secs`. The durable timer, and the retry.
    fn schedule(&self, at_epoch_secs: i64, target: NodeTarget) -> Result<(), HostError>;
}

/// A convenience for nodes: read an input by name.
pub fn input<'a>(inputs: &'a PortValues, name: &str) -> Option<&'a Value> {
    inputs.get(name)
}

// The test surface is deliberately wider than any single test uses: `advance` and `inner`
// are how a scheduler test moves a time window and reads back what was scheduled.
#[allow(dead_code)]
pub mod testkit {
    //! A host that does nothing, for testing nodes and topology.
    //!
    //! Its value is that it is *complete*: a node test needs no database, no blob store, no
    //! model and no clock that moves. State lives in a map, time is a number you set, and the
    //! observer records what happened so a test can assert on it.

    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    pub struct MemState(Mutex<HashMap<String, Json>>);

    fn k(key: &StateKey) -> String {
        format!(
            "{}/{}/{}/{:?}",
            key.target.graph, key.target.node, key.target.instance, key.slot
        )
    }

    impl StateStore for MemState {
        fn get(&self, key: &StateKey) -> Option<Json> {
            self.0.lock().unwrap().get(&k(key)).cloned()
        }
        fn set(&self, key: &StateKey, value: &Json) {
            self.0.lock().unwrap().insert(k(key), value.clone());
        }
        fn clear(&self, key: &StateKey) {
            self.0.lock().unwrap().remove(&k(key));
        }
        fn try_arm(&self, key: &StateKey, expires_at: i64) -> bool {
            let mut m = self.0.lock().unwrap();
            let armed = m
                .get(&k(key))
                .and_then(|v| v.get("state"))
                .and_then(Json::as_str)
                == Some("armed");
            if armed {
                return false;
            }
            m.insert(
                k(key),
                serde_json::json!({ "state": "armed", "expires_at": expires_at }),
            );
            true
        }
        fn extend(&self, key: &StateKey, expires_at: i64) {
            let mut m = self.0.lock().unwrap();
            if let Some(v) = m.get_mut(&k(key)) {
                if v.get("state").and_then(Json::as_str) == Some("armed") {
                    v["expires_at"] = serde_json::json!(expires_at);
                }
            }
        }
        fn try_disarm_expired(&self, key: &StateKey, now: i64) -> bool {
            let mut m = self.0.lock().unwrap();
            let Some(v) = m.get(&k(key)) else {
                return false;
            };
            if v.get("state").and_then(Json::as_str) != Some("armed") {
                return false;
            }
            if v.get("expires_at").and_then(Json::as_i64).unwrap_or(0) > now {
                // Someone pushed the deadline out after this wake-up was scheduled. Their later
                // wake-up closes the window; this one does nothing.
                return false;
            }
            m.insert(k(key), serde_json::json!({ "state": "idle" }));
            true
        }
    }

    /// Records what the engine reported, so a test can assert on it.
    #[derive(Default, Debug)]
    pub struct Recorder {
        pub started: Mutex<Vec<u32>>,
        pub finished: Mutex<Vec<(u32, String)>>,
        pub events: Mutex<Vec<String>>,
        /// Kept separately from `finished` so that adding it did not change what every existing
        /// assertion compares against.
        pub elapsed: Mutex<Vec<u128>>,
    }

    impl Recorder {
        /// The most recent elapsed time reported. Zero if nothing finished.
        pub fn last_elapsed_ms(&self) -> u128 {
            self.elapsed.lock().unwrap().last().copied().unwrap_or(0)
        }
    }

    impl Observer for Recorder {
        fn node_started(&self, node: u32) {
            self.started.lock().unwrap().push(node);
        }
        fn node_finished(&self, node: u32, summary: &str, ms: u128) {
            self.finished
                .lock()
                .unwrap()
                .push((node, summary.to_string()));
            self.elapsed.lock().unwrap().push(ms);
        }
        fn emitted(&self, event: &str, _payload: &PortValues) {
            self.events.lock().unwrap().push(event.to_string());
        }
    }

    #[derive(Debug)]
    pub struct Refuses;

    impl Approvals for Refuses {
        fn ask(&self, _: ApprovalRequest) -> Result<Uuid, HostError> {
            Err(HostError::permanent("no approval channel in tests"))
        }
    }
    impl Http for Refuses {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, HostError> {
            Err(HostError::permanent("no network in tests"))
        }
    }
    impl Llm for Refuses {
        fn ask_text(&self, _: LlmRequest) -> Result<String, HostError> {
            Err(HostError::permanent("no model in tests"))
        }
        fn ask_bool(&self, _: LlmRequest) -> Result<Option<bool>, HostError> {
            Err(HostError::permanent("no model in tests"))
        }
        fn classify(&self, _: LlmRequest, _: &[String]) -> Result<Option<String>, HostError> {
            Err(HostError::permanent("no model in tests"))
        }
    }
    impl TableStore for Refuses {
        fn list(&self) -> Result<Vec<String>, HostError> {
            Ok(Vec::new())
        }
        fn read(&self, _: &str) -> Result<Vec<Vec<(String, Value)>>, HostError> {
            Err(HostError::permanent("no tables in tests"))
        }
        fn row_count(&self, _: &str) -> Result<u64, HostError> {
            Ok(0)
        }
        fn append(&self, _: &str, _: &[(String, Value)]) -> Result<(), HostError> {
            Err(HostError::permanent("no tables in tests"))
        }
        fn set_cell(&self, _: &str, _: u64, _: &str, _: &Value) -> Result<(), HostError> {
            Err(HostError::permanent("no tables in tests"))
        }
        fn delete_row(&self, _: &str, _: u64) -> Result<(), HostError> {
            Err(HostError::permanent("no tables in tests"))
        }
        fn clear(&self, _: &str) -> Result<(), HostError> {
            Err(HostError::permanent("no tables in tests"))
        }
    }

    #[derive(Clone, Debug)]
    pub struct TestHost {
        inner: Arc<Inner>,
    }

    #[derive(Debug)]
    pub struct Inner {
        state: MemState,
        io: Disabled,
        pub observer: Recorder,
        approvals: Refuses,
        http: Refuses,
        llm: Refuses,
        tables: Refuses,
        vars: Mutex<HashMap<String, Value>>,
        run: Uuid,
        /// Settable, so a test makes a ten-minute window pass without waiting ten minutes.
        pub now: Mutex<i64>,
        pub scheduled: Mutex<Vec<(i64, NodeTarget)>>,
    }

    impl std::fmt::Debug for MemState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "MemState")
        }
    }

    impl Default for TestHost {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TestHost {
        pub fn new() -> Self {
            TestHost {
                inner: Arc::new(Inner {
                    state: MemState::default(),
                    io: Disabled,
                    observer: Recorder::default(),
                    approvals: Refuses,
                    http: Refuses,
                    llm: Refuses,
                    tables: Refuses,
                    vars: Mutex::new(HashMap::new()),
                    run: Uuid::from_bytes([9; 16]),
                    now: Mutex::new(1_700_000_000),
                    scheduled: Mutex::new(Vec::new()),
                }),
            }
        }

        pub fn inner(&self) -> &Inner {
            &self.inner
        }

        /// Move the clock. The reason `now_epoch_secs` is on the trait at all.
        pub fn advance(&self, secs: i64) {
            *self.inner.now.lock().unwrap() += secs;
        }
    }

    impl Host for TestHost {
        type Meta = ();

        fn state(&self) -> &dyn StateStore {
            &self.inner.state
        }
        fn io(&self) -> &dyn ValueIo {
            &self.inner.io
        }
        fn observer(&self) -> &dyn Observer {
            &self.inner.observer
        }
        fn approvals(&self) -> &dyn Approvals {
            &self.inner.approvals
        }
        fn http(&self) -> &dyn Http {
            &self.inner.http
        }
        fn llm(&self) -> &dyn Llm {
            &self.inner.llm
        }
        fn tables(&self) -> &dyn TableStore {
            &self.inner.tables
        }
        fn vars(&self) -> &Mutex<HashMap<String, Value>> {
            &self.inner.vars
        }
        fn run_id(&self) -> Uuid {
            self.inner.run
        }
        fn now_epoch_secs(&self) -> i64 {
            *self.inner.now.lock().unwrap()
        }
        fn schedule(&self, at: i64, target: NodeTarget) -> Result<(), HostError> {
            self.inner.scheduled.lock().unwrap().push((at, target));
            Ok(())
        }
    }
}
