//! Implementing `Host`: everything a product must provide, and nothing more.
//!
//!     cargo run --example host
//!
//! This is the whole integration surface. If it ever stops being a page of obvious code, the
//! seam has grown something it should not have — which is why CI runs this rather than only
//! compiling it.
//!
//! It was written first as a separate crate outside this workspace, pulling the engine in as a
//! git dependency, to check the thing that a test inside the repo cannot: that a product can
//! implement `Host` using only the public API, in safe code, without either side's vocabulary
//! crossing over. It ran a three-item loop and printed the three items. Kept here as an example
//! so it stays true.

use ggraph_core::host::*;
use ggraph_core::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Everything the engine can reach. A real product would put its own world in here too —
/// a database pool, a corpus index, a camera set — and its own nodes would reach that directly.
struct World {
    vars: Mutex<HashMap<String, Value>>,
    state: MemState,
    log: Log,
    absent: Absent,
}

#[derive(Clone)]
struct MyHost(Arc<World>);

#[derive(Default)]
struct MemState(Mutex<HashMap<String, serde_json::Value>>);

fn k(key: &StateKey) -> String {
    format!("{}/{}/{:?}", key.target.graph, key.target.node, key.slot)
}

impl StateStore for MemState {
    fn get(&self, key: &StateKey) -> Option<serde_json::Value> {
        self.0.lock().unwrap().get(&k(key)).cloned()
    }
    fn set(&self, key: &StateKey, v: &serde_json::Value) {
        self.0.lock().unwrap().insert(k(key), v.clone());
    }
    fn clear(&self, key: &StateKey) {
        self.0.lock().unwrap().remove(&k(key));
    }
    fn try_arm(&self, _: &StateKey, _: i64) -> bool {
        true
    }
    fn extend(&self, _: &StateKey, _: i64) {}
    fn try_disarm_expired(&self, _: &StateKey, _: i64) -> bool {
        true
    }
}

#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Observer for Log {
    fn node_finished(&self, node: u32, summary: &str, _ms: u128) {
        if !summary.is_empty() {
            self.0.lock().unwrap().push(format!("{node}: {summary}"));
        }
    }
}

/// The capabilities this product has not wired up yet. Every one of them is a trait, so "not
/// yet" costs a struct rather than a stub node or a feature flag.
struct Absent;

impl Approvals for Absent {
    fn ask(&self, _: ApprovalRequest) -> Result<uuid::Uuid, HostError> {
        Err(HostError::permanent("no approval channel"))
    }
}
impl Http for Absent {
    fn send(&self, _: HttpRequest) -> Result<HttpResponse, HostError> {
        Err(HostError::permanent("no network"))
    }
}
impl Llm for Absent {
    fn ask_text(&self, _: LlmRequest) -> Result<String, HostError> {
        Err(HostError::permanent("no model"))
    }
    fn ask_bool(&self, _: LlmRequest) -> Result<Option<bool>, HostError> {
        Err(HostError::permanent("no model"))
    }
    fn classify(&self, _: LlmRequest, _: &[String]) -> Result<Option<String>, HostError> {
        Err(HostError::permanent("no model"))
    }
}
impl TableStore for Absent {
    fn list(&self) -> Result<Vec<String>, HostError> {
        Ok(vec![])
    }
    fn read(&self, _: &str) -> Result<Vec<Vec<(String, Value)>>, HostError> {
        Ok(vec![])
    }
    fn row_count(&self, _: &str) -> Result<u64, HostError> {
        Ok(0)
    }
    fn append(&self, _: &str, _: &[(String, Value)]) -> Result<(), HostError> {
        Ok(())
    }
    fn set_cell(&self, _: &str, _: u64, _: &str, _: &Value) -> Result<(), HostError> {
        Ok(())
    }
    fn delete_row(&self, _: &str, _: u64) -> Result<(), HostError> {
        Ok(())
    }
    fn clear(&self, _: &str) -> Result<(), HostError> {
        Ok(())
    }
}

impl Host for MyHost {
    type Meta = ();
    fn state(&self) -> &dyn StateStore {
        &self.0.state
    }
    fn io(&self) -> &dyn ValueIo {
        &Disabled
    }
    fn observer(&self) -> &dyn Observer {
        &self.0.log
    }
    fn approvals(&self) -> &dyn Approvals {
        &self.0.absent
    }
    fn http(&self) -> &dyn Http {
        &self.0.absent
    }
    fn llm(&self) -> &dyn Llm {
        &self.0.absent
    }
    fn tables(&self) -> &dyn TableStore {
        &self.0.absent
    }
    fn vars(&self) -> &Mutex<HashMap<String, Value>> {
        &self.0.vars
    }
    fn run_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
    fn now_epoch_secs(&self) -> i64 {
        1_700_000_000
    }
    fn schedule(&self, _: i64, _: NodeTarget) -> Result<(), HostError> {
        Ok(())
    }
}

fn main() {
    let mut reg = NodeRegistry::<MyHost>::new();
    ggraph_core::nodes::register_all(&mut reg);

    let mut g: Graph = Graph::new("smoke");
    let each = g.add_node(NodeId::new("for_each"), 0, 0);
    g.node_mut(each).unwrap().config = json!({ "items": "alpha,beta,gamma" });
    let say = g.add_node(NodeId::new("print"), 200, 0);
    g.node_mut(say).unwrap().config = json!({ "message": "" });
    g.add_edge(&reg, each, "loop_body", say, "exec_in").unwrap();
    g.add_edge(&reg, each, "item", say, "message").unwrap();

    let host = MyHost(Arc::new(World {
        vars: Mutex::new(HashMap::new()),
        state: MemState::default(),
        log: Log::default(),
        absent: Absent,
    }));

    ggraph_core::run(&g, &reg, &host, &Entry::default(), &RunOptions::default()).unwrap();

    println!("{} node kinds available", reg.palette().count());
    let log = host.0.log.0.lock().unwrap().clone();
    for line in &log {
        println!("  {line}");
    }
    assert_eq!(
        log.len(),
        4,
        "three passes plus the loop's own line, got {log:?}"
    );
    println!("\nok — a host outside the engine ran a graph.");
}
