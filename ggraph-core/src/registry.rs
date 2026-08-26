// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a product's nodes meet the engine.
//!
//! The registry is the whole extension mechanism. Each product builds one at boot — the
//! standard nodes register themselves, then the product registers its own — and from that point
//! there is no difference in status between them. To the scheduler, to the catalog and to the
//! editor, a branch and a camera snapshot are two entries in the same map.
//!
//! ```ignore
//! fn build() -> NodeRegistry<MyHost> {
//!     let mut reg = NodeRegistry::new();
//!     ggraph_core::nodes::register_all(&mut reg, &Services::none());   // the standard set
//!     my_nodes::register_all(&mut reg, &Services::none());             // whatever this product is about
//!     reg
//! }
//! ```
//!
//! The registry also holds the decoders for [`Value::Extern`](crate::Value::Extern): the codec
//! writes a product value as a tagged object, and this is what turns the tag back into the type.

use crate::graph::PortLookup;
use crate::host::{Host, ValueIo};
use crate::id::NodeId;
use crate::port::Port;
use crate::spec::NodeSpec;
use crate::value::Value;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::Arc;

/// Turns a persisted tagged object back into a product value.
pub type Decoder = fn(&Json, &dyn ValueIo) -> Option<Value>;

/// Why a registration was refused. Registration happens at boot, so these are startup failures,
/// which is the right time for them — a duplicate slug discovered at run time is a node that
/// silently shadows another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateId(NodeId),
    DuplicateAlias {
        alias: String,
        taken_by: NodeId,
    },
    /// An alias that collides with a real kind's id. Allowing it would make resolution depend
    /// on registration order.
    AliasShadowsKind(NodeId),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::DuplicateId(id) => write!(f, "node kind {id} is registered twice"),
            RegistryError::DuplicateAlias { alias, taken_by } => {
                write!(f, "alias {alias:?} is already taken by {taken_by}")
            }
            RegistryError::AliasShadowsKind(id) => {
                write!(f, "alias {id} collides with a registered node kind")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

pub struct NodeRegistry<H: Host> {
    specs: HashMap<NodeId, Arc<NodeSpec<H>>>,
    /// Registration order, so the catalog is stable and diffable.
    order: Vec<NodeId>,
    aliases: HashMap<String, NodeId>,
    decoders: HashMap<&'static str, Decoder>,
}

impl<H: Host> Default for NodeRegistry<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: Host> NodeRegistry<H> {
    pub fn new() -> Self {
        NodeRegistry {
            specs: HashMap::new(),
            order: Vec::new(),
            aliases: HashMap::new(),
            decoders: HashMap::new(),
        }
    }

    /// Register a node kind.
    pub fn add(&mut self, spec: NodeSpec<H>) -> Result<(), RegistryError> {
        let id = spec.id.clone();
        if self.specs.contains_key(&id) {
            return Err(RegistryError::DuplicateId(id));
        }
        for a in spec.aliases {
            if let Some(taken) = self.aliases.get(*a) {
                return Err(RegistryError::DuplicateAlias {
                    alias: (*a).to_string(),
                    taken_by: taken.clone(),
                });
            }
        }
        for a in spec.aliases {
            self.aliases.insert((*a).to_string(), id.clone());
        }
        self.order.push(id.clone());
        self.specs.insert(id, Arc::new(spec));
        Ok(())
    }

    /// Register, panicking on conflict. For a product's own boot code, where a conflict is a
    /// programming error and there is nothing sensible to do but fail loudly and early.
    pub fn register(&mut self, spec: NodeSpec<H>) {
        let id = spec.id.clone();
        self.add(spec)
            .unwrap_or_else(|e| panic!("registering {id}: {e}"));
    }

    /// How a product's own value type comes back out of storage.
    pub fn decoder(&mut self, tag: &'static str, f: Decoder) {
        self.decoders.insert(tag, f);
    }

    /// The decoder table, for the codec.
    pub fn decoders(&self) -> &HashMap<&'static str, Decoder> {
        &self.decoders
    }

    pub fn decode(&self, tag: &str, body: &Json, io: &dyn ValueIo) -> Option<Value> {
        self.decoders.get(tag).and_then(|f| f(body, io))
    }

    /// Resolve a name from a stored document — the id, or any alias.
    pub fn resolve(&self, name: &str) -> Option<&Arc<NodeSpec<H>>> {
        let id = NodeId::new(name);
        self.specs
            .get(&id)
            .or_else(|| self.aliases.get(name).and_then(|id| self.specs.get(id)))
    }

    pub fn get(&self, id: &NodeId) -> Option<&Arc<NodeSpec<H>>> {
        self.specs.get(id)
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Every registered kind, in registration order.
    pub fn all(&self) -> impl Iterator<Item = &Arc<NodeSpec<H>>> {
        self.order.iter().filter_map(|id| self.specs.get(id))
    }

    /// The kinds a person may add from the palette — everything except the hidden ones.
    pub fn palette(&self) -> impl Iterator<Item = &Arc<NodeSpec<H>>> {
        self.all().filter(|s| !s.hidden)
    }

    /// The palette, as the editor reads it.
    ///
    /// This is the contract with the front end, and it is worth freezing in a test: a renamed
    /// slug, a moved port or a changed default reaches a person as a broken canvas, and nothing
    /// else in a Rust codebase notices.
    pub fn catalog_json(&self) -> Json {
        let kinds: Vec<Json> = self
            .palette()
            .map(|s| {
                let cfg = (s.default_config)();
                let mut inputs: Vec<Json> = Vec::new();
                let mut outputs: Vec<Json> = Vec::new();
                // Exec pins come first and are omitted entirely for a pure node, so the editor
                // knows to hide the exec circles rather than drawing dead ones.
                if s.purity.has_exec() {
                    inputs.push(port_json(&crate::port::EXEC_IN));
                    outputs.extend(s.exec_out.resolve(&cfg).iter().map(port_json));
                }
                inputs.extend(s.inputs.resolve(&cfg).iter().map(port_json));
                outputs.extend(s.outputs.resolve(&cfg).iter().map(port_json));
                json!({
                    "slug": s.id.as_str(),
                    "label": s.label,
                    "category": s.category,
                    "inputs": inputs,
                    "outputs": outputs,
                    "default_config": cfg,
                })
            })
            .collect();
        json!({ "kinds": kinds })
    }
}

fn port_json(p: &Port) -> Json {
    json!({ "name": p.name.as_str(), "type": p.ty.as_str(), "required": p.required })
}

impl<H: Host> std::fmt::Debug for NodeRegistry<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NodeRegistry {{ {} kind(s), {} alias(es) }}",
            self.specs.len(),
            self.aliases.len()
        )
    }
}

/// This is what [`Graph::add_edge`](crate::Graph::add_edge) validates against — so wiring and
/// execution ask the same question and cannot drift apart.
impl<H: Host> PortLookup for NodeRegistry<H> {
    fn inputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        self.get(kind).map(|s| s.inputs.resolve(config))
    }
    fn outputs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        self.get(kind).map(|s| s.outputs.resolve(config))
    }
    fn exec_outs(&self, kind: &NodeId, config: &Json) -> Option<Vec<Port>> {
        self.get(kind).map(|s| s.exec_out.resolve(config))
    }
    fn has_exec_in(&self, kind: &NodeId) -> bool {
        self.get(kind).is_some_and(|s| s.purity.has_exec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::port::PortType;
    use crate::spec::{ExecOut, Ports};

    fn spec(id: &'static str) -> NodeSpec<TestHost> {
        NodeSpec::effectful(id, "Label", "Group")
    }

    #[test]
    fn a_duplicate_kind_is_refused_at_boot() {
        let mut r = NodeRegistry::new();
        r.add(spec("thing")).unwrap();
        assert_eq!(
            r.add(spec("thing")),
            Err(RegistryError::DuplicateId(NodeId::new_static("thing"))),
            "a second registration silently shadowing the first is the worst outcome"
        );
    }

    #[test]
    fn an_alias_resolves_to_its_kind() {
        let mut r = NodeRegistry::new();
        r.add(spec("notify_recipient").with_aliases(&["notify_beacon"]))
            .unwrap();
        assert_eq!(
            r.resolve("notify_beacon").map(|s| s.id.clone()),
            Some(NodeId::new_static("notify_recipient")),
            "a renamed node must keep loading every graph that still names the old one"
        );
    }

    #[test]
    fn two_kinds_cannot_claim_the_same_alias() {
        let mut r = NodeRegistry::new();
        r.add(spec("a").with_aliases(&["old"])).unwrap();
        assert!(matches!(
            r.add(spec("b").with_aliases(&["old"])),
            Err(RegistryError::DuplicateAlias { .. })
        ));
    }

    #[test]
    fn a_hidden_kind_resolves_but_is_not_offered() {
        let mut r = NodeRegistry::new();
        r.add(spec("reroute").hidden()).unwrap();
        r.add(spec("visible")).unwrap();
        assert!(
            r.resolve("reroute").is_some(),
            "a stored graph containing it must still load — this is the trap that hiding a kind \
             by leaving it out of the list walks straight into"
        );
        let palette: Vec<&str> = r.palette().map(|s| s.id.as_str()).collect();
        assert_eq!(palette, vec!["visible"]);
    }

    // Port tables are `static`, not inline slices: `Port` holds a `SmolStr`, which has a
    // destructor, so `&[..]` is not promoted to `'static`. Every node file uses this shape.
    static ADD_IN: [Port; 1] = [Port::opt("a", PortType::NUM)];
    static ADD_OUT: [Port; 1] = [Port::opt("result", PortType::NUM)];

    #[test]
    fn the_catalog_hides_exec_pins_on_a_pure_node() {
        let mut r: NodeRegistry<TestHost> = NodeRegistry::new();
        r.add(
            NodeSpec::pure("add", "Add", "Math")
                .with_inputs(Ports::Static(&ADD_IN))
                .with_outputs(Ports::Static(&ADD_OUT)),
        )
        .unwrap();
        let cat = r.catalog_json();
        let k = &cat["kinds"][0];
        assert_eq!(k["inputs"].as_array().unwrap().len(), 1);
        assert_eq!(
            k["outputs"].as_array().unwrap().len(),
            1,
            "a pure node must not advertise exec circles the editor would draw and nothing \
             would ever light up"
        );
    }

    #[test]
    fn the_catalog_is_stable_across_calls() {
        let mut r = NodeRegistry::new();
        for id in ["c", "a", "b"] {
            r.add(spec(Box::leak(id.to_string().into_boxed_str())))
                .unwrap();
        }
        let first = r.catalog_json();
        for _ in 0..10 {
            assert_eq!(
                r.catalog_json(),
                first,
                "an unstable catalog makes the snapshot test that guards it useless"
            );
        }
        let slugs: Vec<&str> = first["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["slug"].as_str().unwrap())
            .collect();
        assert_eq!(
            slugs,
            vec!["c", "a", "b"],
            "registration order, not hash order"
        );
    }

    #[test]
    fn dynamic_arms_come_from_the_configuration() {
        fn arms(cfg: &Json) -> Vec<Port> {
            cfg.get("arms")
                .and_then(Json::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Json::as_str)
                        .map(|s| Port::new(crate::id::PortName::new(s), PortType::EXEC, false))
                        .collect()
                })
                .unwrap_or_default()
        }
        let mut r = NodeRegistry::new();
        r.add(
            spec("switch")
                .with_exec_out(ExecOut::dynamic(arms))
                .with_config(|| json!({ "arms": ["alpha", "beta"] })),
        )
        .unwrap();
        let cfg = json!({ "arms": ["one", "two", "three"] });
        let got: Vec<String> = r
            .exec_outs(&NodeId::new_static("switch"), &cfg)
            .unwrap()
            .iter()
            .map(|p| p.name.as_str().to_string())
            .collect();
        assert_eq!(
            got,
            vec!["one", "two", "three"],
            "wiring must see the arms the author configured, not the defaults — a validator \
             reading the static table gives a different answer than the scheduler"
        );
    }

    #[test]
    fn a_declared_id_is_free_to_clone_at_any_length() {
        // The scheduler clones node ids and port names in its inner loop, so this matters.
        // `new_static` keeps the pointer rather than copying, which makes a declared id free
        // regardless of length — the 23-byte inline budget only binds ids built at run time.
        // That is the reason node files declare their id as a constant instead of a string.
        let mut r = NodeRegistry::new();
        r.add(spec("a_deliberately_long_declared_node_identifier"))
            .unwrap();
        r.add(spec("if")).unwrap();
        assert!(
            r.all().all(|s| s.id.is_inline()),
            "a declared id must never allocate"
        );
        // Built through `format!` so it is genuinely a run-time string, not a literal the
        // compiler could fold back into a constant.
        let at_runtime = format!("a_deliberately_long_{}_node_identifier", "declared");
        assert!(
            !NodeId::new(at_runtime).is_inline(),
            "the same text built at run time does allocate — which is what makes the guarantee \
             above a property of `new_static`, not of the string"
        );
    }
}
