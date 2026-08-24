//! The seam. Everything the engine needs from the world outside it, as traits.
//!
//! This is why `ggraph-core` has four dependencies and no tokio: the scheduler never opens a
//! socket, never touches a database and never reads a clock directly. It asks the [`Host`].
//!
//! The seam is small, and it is smaller than it first looks. Sentinel's engine threaded a
//! 14-field context through every node; the *scheduler* used six of those fields, and of the
//! six, `tenant_id` was only ever an argument to a store call and `active_gates` only ever used
//! inside one node. What is actually needed is four capabilities and a run id.
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

/// Something the world refused to do. The engine surfaces these; it does not interpret them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}

impl From<String> for HostError {
    fn from(s: String) -> Self {
        HostError(s)
    }
}

impl From<&str> for HostError {
    fn from(s: &str) -> Self {
        HostError(s.to_string())
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
    fn try_arm(&self, key: &StateKey, expires_at: &str) -> bool;

    /// Push an armed window's deadline out. A no-op if not armed.
    fn extend(&self, key: &StateKey, expires_at: &str);

    /// Move an armed-and-expired window to idle. `true` means this caller won.
    fn try_disarm_expired(&self, key: &StateKey) -> bool;
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
        Err(HostError("no blob store configured".into()))
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError("no blob store configured".into()))
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

    /// Re-enter `target` at `at_epoch_secs`. The durable timer, and the retry.
    fn schedule(&self, at_epoch_secs: i64, target: NodeTarget) -> Result<(), HostError>;
}

/// A convenience for nodes: read an input by name.
pub fn input<'a>(inputs: &'a PortValues, name: &str) -> Option<&'a Value> {
    inputs.get(&PortName::new(name))
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
        fn try_arm(&self, key: &StateKey, expires_at: &str) -> bool {
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
        fn extend(&self, key: &StateKey, expires_at: &str) {
            let mut m = self.0.lock().unwrap();
            if let Some(v) = m.get_mut(&k(key)) {
                if v.get("state").and_then(Json::as_str) == Some("armed") {
                    v["expires_at"] = serde_json::json!(expires_at);
                }
            }
        }
        fn try_disarm_expired(&self, key: &StateKey) -> bool {
            let mut m = self.0.lock().unwrap();
            let armed = m
                .get(&k(key))
                .and_then(|v| v.get("state"))
                .and_then(Json::as_str)
                == Some("armed");
            if !armed {
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
    }

    impl Observer for Recorder {
        fn node_started(&self, node: u32) {
            self.started.lock().unwrap().push(node);
        }
        fn node_finished(&self, node: u32, summary: &str, _ms: u128) {
            self.finished
                .lock()
                .unwrap()
                .push((node, summary.to_string()));
        }
        fn emitted(&self, event: &str, _payload: &PortValues) {
            self.events.lock().unwrap().push(event.to_string());
        }
    }

    #[derive(Debug)]
    pub struct Refuses;

    impl Approvals for Refuses {
        fn ask(&self, _: ApprovalRequest) -> Result<Uuid, HostError> {
            Err(HostError("no approval channel in tests".into()))
        }
    }
    impl Http for Refuses {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, HostError> {
            Err(HostError("no network in tests".into()))
        }
    }
    impl Llm for Refuses {
        fn ask_text(&self, _: LlmRequest) -> Result<String, HostError> {
            Err(HostError("no model in tests".into()))
        }
        fn ask_bool(&self, _: LlmRequest) -> Result<Option<bool>, HostError> {
            Err(HostError("no model in tests".into()))
        }
        fn classify(&self, _: LlmRequest, _: &[String]) -> Result<Option<String>, HostError> {
            Err(HostError("no model in tests".into()))
        }
    }
    impl TableStore for Refuses {
        fn list(&self) -> Result<Vec<String>, HostError> {
            Ok(Vec::new())
        }
        fn read(&self, _: &str) -> Result<Vec<Vec<(String, Value)>>, HostError> {
            Err(HostError("no tables in tests".into()))
        }
        fn row_count(&self, _: &str) -> Result<u64, HostError> {
            Ok(0)
        }
        fn append(&self, _: &str, _: &[(String, Value)]) -> Result<(), HostError> {
            Err(HostError("no tables in tests".into()))
        }
        fn set_cell(&self, _: &str, _: u64, _: &str, _: &Value) -> Result<(), HostError> {
            Err(HostError("no tables in tests".into()))
        }
        fn delete_row(&self, _: &str, _: u64) -> Result<(), HostError> {
            Err(HostError("no tables in tests".into()))
        }
        fn clear(&self, _: &str) -> Result<(), HostError> {
            Err(HostError("no tables in tests".into()))
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
