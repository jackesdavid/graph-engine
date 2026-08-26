// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Implementing `Host`: everything a product must provide, and nothing more.
//!
//!     cargo run --example host
//!
//! This is the whole integration surface. It used to be twice this long, because `Host` also
//! demanded an approval channel, a network, a model and a table store — none of which the
//! scheduler ever calls. Those belong to the standard node library and are handed over at
//! registration now, so a product registering none of them writes none of them.
//!
//! CI runs this rather than only compiling it. If implementing `Host` stops being a page of
//! obvious safe code, this is where it shows.

use ggraph_core::host::*;
use ggraph_core::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Everything the engine can reach. A real product would put its own world here too — a database
/// pool, a document index, a set of devices — and its own nodes would reach that directly.
struct World {
    state: MemState,
    log: Log,
}

#[derive(Clone)]
struct MyHost(Arc<World>);

#[derive(Default)]
struct MemState(Mutex<HashMap<String, serde_json::Value>>);

fn key(k: &StateKey) -> String {
    format!("{}/{}/{:?}", k.target.graph, k.target.node, k.slot)
}

impl StateStore for MemState {
    fn get(&self, k: &StateKey) -> Option<serde_json::Value> {
        self.0.lock().unwrap().get(&key(k)).cloned()
    }
    fn set(&self, k: &StateKey, v: &serde_json::Value) {
        self.0.lock().unwrap().insert(key(k), v.clone());
    }
    fn clear(&self, k: &StateKey) {
        self.0.lock().unwrap().remove(&key(k));
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
    fn run_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
    fn now_epoch_secs(&self) -> i64 {
        1_700_000_000
    }
    fn schedule(&self, _at: i64, _target: NodeTarget) -> Result<(), HostError> {
        // A real product would put a row in a durable queue. Returning Ok here means `wait` and
        // `approval` end the run and are never woken, which is honest for an example.
        Ok(())
    }
}

fn main() {
    let mut reg = NodeRegistry::<MyHost>::new();
    // The standard nodes' needs are a parameter, not something reached through `Host`. `none()`
    // supplies no approval channel, network, model or table store — and every node that wants
    // one then refuses with a message naming what is missing, rather than panicking.
    ggraph_core::nodes::register_all(&mut reg, &Services::none());

    let mut g: Graph = Graph::new("smoke");
    let each = g.add_node(NodeId::new("for_each"), 0, 0);
    g.node_mut(each).unwrap().config = json!({ "items": "alpha,beta,gamma" });
    let say = g.add_node(NodeId::new("print"), 200, 0);
    g.node_mut(say).unwrap().config = json!({ "message": "" });
    g.add_edge(&reg, each, "loop_body", say, "exec_in").unwrap();
    g.add_edge(&reg, each, "item", say, "message").unwrap();

    // Check the document before running it: an unregistered kind or a wire to a port that no
    // longer exists is a thing to learn here, not at 3am when a trigger fires.
    let problems = ggraph_core::validate(&g, &reg);
    assert!(problems.is_empty(), "{problems:?}");

    let host = MyHost(Arc::new(World {
        state: MemState::default(),
        log: Log::default(),
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
