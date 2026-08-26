// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A host that does nothing, for testing nodes and topology.
//!
//! Its value is that it is *complete*: a node test needs no database, no blob store, no
//! model and no clock that moves. State lives in a map, time is a number you set, and the
//! observer records what happened so a test can assert on it.

use super::*;
use crate::nodes::services::{
    ApprovalRequest, Approvals, Http, HttpRequest, HttpResponse, Llm, LlmRequest, TableStore,
};
use crate::value::Value;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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
    /// Which arms fired, in order, as `(node, port)`.
    pub arms: Mutex<Vec<(u32, String)>>,
    /// How many times the run was declared over. A buffering observer flushes here, so a test
    /// can prove the hook fires — including on the path where the run failed.
    pub ends: Mutex<usize>,
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
    fn arm(&self, node: u32, port: &str) {
        self.arms.lock().unwrap().push((node, port.to_string()));
    }
    fn run_finished(&self) {
        *self.ends.lock().unwrap() += 1;
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

impl TestHost {
    /// Services for a node test.
    ///
    /// They are not on [`Host`] any more — the scheduler never called them — so a node test
    /// hands them to [`register_all`](crate::nodes::register_all) the way a product would.
    /// These refuse, which is what a test of a node's *logic* wants: the refusal is visible and
    /// nothing reaches a network. A test that needs a working one supplies it.
    pub fn services(&self) -> crate::nodes::services::Services {
        crate::nodes::services::Services::none()
    }
}
