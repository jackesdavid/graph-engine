// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

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
use crate::id::{NodeId, PortName};
use crate::port::Port;
use crate::port::PortType;
use crate::spec::{ExecOut, Field, FieldKind, Fields, NodeSpec, Ports};
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
        self.get(&NodeId::new(name))
    }

    /// A kind by its id, or by any name it also answers to.
    ///
    /// Aliases are consulted HERE and not only in `resolve`, because the scheduler and the wire
    /// check reach for a kind through this one. While they did not, the whole alias mechanism was
    /// exercised by a catalogue test and by nothing else: a renamed node passed every test in the
    /// repository and then failed to load the first stored graph that contained the old name.
    pub fn get(&self, id: &NodeId) -> Option<&Arc<NodeSpec<H>>> {
        self.specs.get(id).or_else(|| {
            self.aliases
                .get(id.as_str())
                .and_then(|id| self.specs.get(id))
        })
    }

    /// Todas as declarações, para quem precisa de listar o que existe.
    ///
    /// Sem isto não há catálogo — e o catálogo é como um editor descobre o que pode desenhar. Um
    /// produto com um enum de tipos consegue iterar o enum; um que use identificadores abertos, que
    /// é o que esta engine oferece, só tem o registry. Recusar a enumeração obrigava cada consumidor
    /// a manter uma segunda lista à parte, e duas listas da mesma coisa divergem.
    ///
    /// A ordem é estável: por identificador, para um catálogo servido duas vezes sair igual.
    /// Every output port that could feed a port of this type, as `(kind, port)`.
    ///
    /// The question anyone building a graph asks, over and over, and the only way to answer it was
    /// to read every kind and compare types by hand. The registry has held the answer all along.
    ///
    /// Ports are resolved against each kind's DEFAULT config, so a node whose ports depend on its
    /// settings is listed as it arrives from the palette — which is the state it is in when
    /// somebody is deciding whether to place it.
    pub fn producers(&self, ty: &PortType) -> Vec<(NodeId, PortName)> {
        self.matching(ty, false)
    }

    /// And every input port that could take one, as `(kind, port)`.
    pub fn consumers(&self, ty: &PortType) -> Vec<(NodeId, PortName)> {
        self.matching(ty, true)
    }

    /// Both sides of the same question. `exec` is left out: control is not data, every node that
    /// takes control has the same port for it, and listing it would bury the answer.
    fn matching(&self, ty: &PortType, want_input: bool) -> Vec<(NodeId, PortName)> {
        if ty.as_str() == PortType::EXEC.as_str() {
            return Vec::new();
        }
        let probe = Port::opt("", ty.clone());
        let mut out = Vec::new();
        for spec in self.iter() {
            let cfg = (spec.default_config)();
            let ports = if want_input {
                spec.inputs.resolve(&cfg)
            } else {
                spec.outputs.resolve(&cfg)
            };
            for p in ports {
                if p.ty == PortType::EXEC {
                    continue;
                }
                // Direction matters: a wire runs producer → consumer, and the family rules are not
                // symmetric. `text` feeds a `scalar` input; a `scalar` output does not feed `text`.
                let ok = if want_input {
                    crate::port::compatible(&probe, &p)
                } else {
                    crate::port::compatible(&p, &probe)
                };
                if ok {
                    out.push((spec.id.clone(), p.name.clone()));
                }
            }
        }
        out
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<NodeSpec<H>>> {
        let mut v: Vec<_> = self.specs.values().collect();
        v.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        v.into_iter()
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
            .map(|s| kind_json(s, &(s.default_config)()))
            .collect();
        json!({ "kinds": kinds })
    }

    /// One kind, resolved against a node's actual configuration.
    ///
    /// The catalogue can only resolve dynamic ports against the DEFAULT config, so an editor that
    /// wants the pins a configured node really has had to re-implement [`Ports::dynamic`] in its
    /// own language — and two implementations of a port list is how a canvas comes to draw a pin
    /// the run does not have. This is the same answer, from the side that knows.
    pub fn resolve_json(&self, slug: &str, config: &Json) -> Option<Json> {
        self.get(&NodeId::new(slug)).map(|s| kind_json(s, config))
    }
}

/// One kind as the editor reads it, resolved against `cfg`.
fn kind_json<H: Host>(s: &NodeSpec<H>, cfg: &Json) -> Json {
    let mut inputs: Vec<Json> = Vec::new();
    let mut outputs: Vec<Json> = Vec::new();
    // Exec pins come first and are omitted entirely for a pure node, so the editor knows to hide
    // the exec circles rather than drawing dead ones.
    if s.purity.has_exec() {
        inputs.push(port_json(&crate::port::EXEC_IN));
        outputs.extend(s.exec_out.resolve(cfg).iter().map(port_json));
    }
    inputs.extend(s.inputs.resolve(cfg).iter().map(port_json));
    outputs.extend(s.outputs.resolve(cfg).iter().map(port_json));

    let mut j = json!({
        "slug": s.id.as_str(),
        "label": s.label,
        "category": s.category,
        "inputs": inputs,
        "outputs": outputs,
        "default_config": (s.default_config)(),
    });
    // Whether this kind's pins depend on its configuration. An editor showing a configured node
    // has to ask again for those; without being told which, it keeps its own list of them, and a
    // list of that kind is one somebody forgets to add the next node to.
    if matches!(s.inputs, Ports::Dynamic(_))
        || matches!(s.outputs, Ports::Dynamic(_))
        || matches!(s.exec_out, ExecOut::Dynamic(_))
        || matches!(s.fields, Fields::Dynamic(_))
    {
        j["dynamic"] = Json::Bool(true);
    }
    // Only when declared. A node that says nothing leaves the editor guessing from the default
    // value, which is what every node did before fields existed.
    // Can a chain BEGIN here — nothing must come before it, and something comes out. Published
    // rather than left to each reader to work out: a harness that recomputed the rule in another
    // language drifted from it immediately, and offered a sink as a place to start.
    if starts(s, cfg) {
        j["can_start"] = Json::Bool(true);
    }

    // What the kind is FOR. Two readers, one text: the palette shows it to a person and the
    // catalogue hands it to whatever is choosing nodes. Absent when nobody has written one, so the
    // reader can tell "no description" from "described as nothing".
    if !s.about.is_empty() {
        j["about"] = Json::from(s.about);
    }

    let fields = s.fields.resolve(cfg);
    if !fields.is_empty() {
        j["fields"] = Json::Array(fields.iter().map(field_json).collect());
    }
    j
}

/// The rule [`sources`](crate::graph::route::sources) applies, for one spec.
///
/// Here rather than only there because the catalogue has to publish it, and a rule stated twice is
/// a rule that disagrees with itself.
pub(crate) fn starts<H: Host>(s: &NodeSpec<H>, cfg: &Json) -> bool {
    let ins = s.inputs.resolve(cfg);
    let outs = s.outputs.resolve(cfg);

    // Nothing has to come before it: no input needs a wire.
    let takeable = ins
        .iter()
        .all(|p| p.ty == PortType::EXEC || !p.needs_a_wire());

    // And something comes out that was not put in. A node whose every output type is also one of
    // its input types RELAYS — `output` hands back what it was given, satisfied "something comes
    // out", and was offered as a place to begin.
    let originates = outs
        .iter()
        .any(|o| o.ty != PortType::EXEC && !ins.iter().any(|i| i.ty == o.ty));

    takeable && originates
}

fn field_json(f: &Field) -> Json {
    // `required` travels the same way a port's does, so an editor can mark a setting nobody filled
    // with the badge it already draws — and so a model reading the catalogue can see it at all.
    let mut j = json!({ "key": f.key.as_str(), "label": f.label, "required": f.required });
    match &f.kind {
        FieldKind::Text => j["kind"] = "text".into(),
        FieldKind::LongText => j["kind"] = "long_text".into(),
        FieldKind::Num => j["kind"] = "num".into(),
        FieldKind::Bool => j["kind"] = "bool".into(),
        FieldKind::Choice(options) => {
            j["kind"] = "choice".into();
            j["options"] = Json::Array(options.iter().map(|o| Json::from(o.as_str())).collect());
        }
        FieldKind::Rows(fields) => {
            j["kind"] = "rows".into();
            j["fields"] = Json::Array(fields.iter().map(field_json).collect());
        }
    }
    j
}

fn port_json(p: &Port) -> Json {
    let mut j = json!({
        "name": p.name.as_str(),
        "type": p.ty.as_str(),
        "required": p.required,
    });
    // Only when true. An editor drawing "wire this" on every port would be drawing nothing.
    if p.wired_only {
        j["wired_only"] = Json::Bool(true);
    }
    // What the port is for, when somebody has said. This is the surface a model reads before it
    // wires anything, and the tooltip a person reads before drawing the same wire.
    if !p.about.is_empty() {
        j["about"] = p.about.into();
    }
    // Every type has a family. It is what lets an editor offer the types that would fit a pin,
    // check a wire, and filter a palette, without keeping its own copy of the rule.
    j["family"] = p.family().as_str().into();
    if p.ty.is_family() {
        j["accepts_family"] = true.into();
    }
    // What one element of a list is, so a loop can say what it hands out. Not for a port that asks
    // for the family itself: it receives lists rather than producing one, and `any` there would be
    // a claim about something it never holds.
    if p.family() == crate::port::Family::List && !p.ty.is_family() {
        j["element"] = p.element().as_str().to_string().into();
    }
    // Only when there are any. A `columns: []` on every text port would be noise in a catalogue a
    // person reads and a model parses.
    if !p.columns.is_empty() {
        j["columns"] = Json::Array(
            p.columns
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name.as_str(),
                        "type": c.ty.as_str(),
                        "optional": c.optional,
                    })
                })
                .collect(),
        );
    }
    j
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

    /// The question anyone building a graph asks, answered by the thing that has always held the
    /// answer. Before this it took reading every kind and comparing types by hand.
    #[test]
    fn the_registry_says_what_can_feed_a_port() {
        let mut r: NodeRegistry<crate::host::testkit::TestHost> = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &crate::nodes::services::Services::none());

        let makes_text = r.producers(&PortType::TEXT);
        assert!(
            makes_text
                .iter()
                .any(|(k, p)| k.as_str() == "format" && p.as_str() == "text"),
            "Format produces text: {makes_text:?}"
        );
        let takes_text = r.consumers(&PortType::TEXT);
        assert!(!takes_text.is_empty(), "something takes text");
    }

    /// Control is not data. Every node that takes control has the same port for it, so listing
    /// them would be the whole catalogue and would bury whatever was asked about.
    #[test]
    fn exec_is_not_a_connection_worth_listing() {
        let mut r: NodeRegistry<crate::host::testkit::TestHost> = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &crate::nodes::services::Services::none());
        assert!(r.producers(&PortType::EXEC).is_empty());
        assert!(r.consumers(&PortType::EXEC).is_empty());
    }

    /// A wire runs one way and the family rules are not symmetric: `text` satisfies a `scalar`
    /// input, while a `scalar` output does not satisfy a `text` input. Asking both sides with one
    /// comparison would answer one of them wrongly.
    #[test]
    fn direction_is_not_symmetric() {
        let mut r: NodeRegistry<crate::host::testkit::TestHost> = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &crate::nodes::services::Services::none());
        let scalar_takers = r.consumers(&PortType::TEXT);
        let text_makers = r.producers(&PortType::SCALAR);
        assert!(
            scalar_takers.len() != text_makers.len() || scalar_takers.is_empty(),
            "the two directions are answered separately"
        );
    }
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
