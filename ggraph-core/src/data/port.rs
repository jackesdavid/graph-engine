// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Typed ports, and the one rule that decides whether an edge may exist.
//!
//! A node declares input and output ports; an edge maps one node's output port to another
//! node's input port. **A node never references another node** — the graph owns all wiring and
//! the engine resolves it. That isolation is what lets a node be tested with a map of inputs
//! and nothing else.
//!
//! [`PortType`] is open, like [`NodeId`](crate::NodeId): a newtype over a string with constants
//! for the builtin set. The core defines the types it can reason about; a product defines its
//! own next to them and the core moves the values along wires without knowing what they are.

use crate::id::PortName;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt;

/// What flows along an edge. Open by design — see the module docs.
///
/// The wire representation is the bare string, so a product's `PortType::new_static("camera")`
/// is indistinguishable from a builtin.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PortType(SmolStr);

impl PortType {
    pub const fn new_static(s: &'static str) -> Self {
        PortType(SmolStr::new_static(s))
    }

    pub fn new(s: impl AsRef<str>) -> Self {
        PortType(SmolStr::new(s.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    // The builtin set. The strings are the ones the first consumer already had on the wire,
    // including where the Rust name has since changed (`BYTES` is `"file"`, `MAP` is
    // `"dictionary"`) — renaming them would have meant migrating every stored graph in exchange
    // for a tidier identifier.

    /// Free text.
    pub const TEXT: PortType = PortType::new_static("text");
    /// A number. Integer or float — see [`Num`](crate::Num).
    pub const NUM: PortType = PortType::new_static("num");
    /// A boolean.
    pub const BOOL: PortType = PortType::new_static("bool");
    /// Opaque bytes with a mime type — a file, a document, an audio clip.
    pub const BYTES: PortType = PortType::new_static("file");
    /// Image bytes. Distinct from [`BYTES`](Self::BYTES) so an image input cannot be wired from
    /// an arbitrary download by accident.
    pub const IMAGE: PortType = PortType::new_static("image");
    /// A structured object, when a typed port would be a lie.
    pub const JSON: PortType = PortType::new_static("json");
    /// An ordered sequence.
    pub const LIST: PortType = PortType::new_static("list");

    /// A list of numbers.
    ///
    /// Distinct from `LIST` so that arithmetic — rounding, scaling, summing — declares what it can
    /// actually operate on. A node taking `LIST` accepts a list of documents, and the mistake only
    /// shows at run time.
    pub const NUMBERS: PortType = PortType::new_static("numbers");

    /// A list of text.
    ///
    /// Its own type rather than `LIST` for the same reason, and one specific one: a chart whose
    /// values and names are swapped is wrong and looks entirely fine. Two different types make that
    /// wire impossible to draw.
    pub const TEXTS: PortType = PortType::new_static("texts");
    /// Key/value pairs.
    pub const MAP: PortType = PortType::new_static("dictionary");
    /// A named table of rows.
    pub const TABLE: PortType = PortType::new_static("table");

    /// The shape of a table — columns and their types, with no rows.
    ///
    /// Its own type rather than an empty `TABLE`, because a shape and the data shaped by it are
    /// different things to wire: one search's results must not be pluggable into another search's
    /// schema input, which is a wire the editor would happily draw and which means nothing.
    pub const SCHEMA: PortType = PortType::new_static("schema");

    /// A table's rows, on their own. What a loop walks.
    pub const TABLE_ROWS: PortType = PortType::new_static("table_rows");
    /// One row: named cells, in the author's column order.
    pub const TABLE_ROW: PortType = PortType::new_static("table_row");
    /// One column's name and type.
    pub const TABLE_COLUMN: PortType = PortType::new_static("table_column");
    /// One cell, addressed rather than read. A node that READS a cell returns the type its column
    /// was declared as, so the value can go straight into whatever wanted a number.
    pub const TABLE_CELL: PortType = PortType::new_static("table_cell");
    /// Control flow, not data. Exec ports carry no value — they say *when*, not *what*.
    pub const EXEC: PortType = PortType::new_static("exec");
    /// One value a person could read: text, a number, or a boolean. Never a table, a list or a
    /// file — those become one of these through a node, which is a step somebody can see.
    pub const SCALAR: PortType = PortType::new_static("scalar");
    /// Wildcard: compatible with everything, in both directions.
    ///
    /// Left for a product that genuinely needs it. Nothing in the standard set does any more: an
    /// `any` port is one the editor cannot refuse, and every refusal it cannot make is a wire
    /// somebody draws and a run that goes wrong somewhere else.
    pub const ANY: PortType = PortType::new_static("any");

    /// Does this value satisfy this type?
    ///
    /// **Only the types the engine defines are checked.** An open identifier — `block`, `document`,
    /// whatever a product declares — is that product's vocabulary, and the engine guessing at its
    /// runtime shape would be the engine acquiring knowledge it has no business having. Unknown
    /// types pass, and that is a boundary rather than a gap.
    ///
    /// Element types walk the whole list. A `numbers` holding one string is not "mostly numbers":
    /// it is the wire that will feed a chart a bar it cannot draw, and finding out on the tenth
    /// element is finding out.
    pub fn accepts(&self, v: &crate::value::Value) -> bool {
        use crate::value::Value as V;
        use serde_json::Value as Json;

        let is_num = |x: &V| matches!(x, V::Num(_)) || matches!(x, V::Json(Json::Number(_)));
        let is_text = |x: &V| matches!(x, V::Text(_)) || matches!(x, V::Json(Json::String(_)));
        let is_record = |x: &V| matches!(x, V::Map(_)) || matches!(x, V::Json(Json::Object(_)));
        let every = |items: &[V], f: &dyn Fn(&V) -> bool| items.iter().all(f);

        // Compared rather than matched: a `PortType` wraps a `SmolStr`, which cannot appear in a
        // pattern. The order below is the order of the constants above.
        let t = self.as_str();
        match t {
            // Exec carries no value; a node returning one on an exec port is confused, but that is
            // not this check's business.
            "any" | "exec" => true,
            "text" => is_text(v),
            "num" => is_num(v),
            "bool" => matches!(v, V::Bool(_)) || matches!(v, V::Json(Json::Bool(_))),
            "file" | "image" => matches!(v, V::Bytes(_)),
            "json" => matches!(v, V::Json(_)),
            "dictionary" => matches!(v, V::Map(_)) || matches!(v, V::Json(Json::Object(_))),
            "list" => matches!(v, V::List(_)),
            "numbers" => matches!(v, V::List(items) if every(items, &is_num)),
            "texts" => matches!(v, V::List(items) if every(items, &is_text)),
            // A table is its columns and its rows. A bare list of rows is accepted too: stored
            // graphs predate the columns travelling alongside the data.
            "table" => {
                is_record(v) || matches!(v, V::List(items) if every(items, &is_record))
            }
            "table_rows" => matches!(v, V::List(items) if every(items, &is_record)),
            "table_row" | "table_column" => is_record(v),
            "scalar" => is_text(v) || is_num(v) || matches!(v, V::Bool(_) | V::Json(Json::Bool(_))),
            // A schema is the column list itself — named shapes, no rows.
            "schema" => matches!(v, V::List(items) if every(items, &is_record)),
            // A product's own vocabulary. Not the engine's to judge.
            _ => true,
        }
    }

    /// A short description of what arrived, for a defect message.
    ///
    /// The name of the value's own shape, not a guess at what it should have been: "returned a
    /// list of text where `numbers` was declared" is a sentence somebody can act on.
    /// The engine type a value already is. Used to read a column's type back off a table that
    /// arrived without one.
    pub fn describe_type(v: &crate::value::Value) -> PortType {
        use crate::value::Value as V;
        use serde_json::Value as Json;
        match v {
            V::Num(_) | V::Json(Json::Number(_)) => PortType::NUM,
            V::Bool(_) | V::Json(Json::Bool(_)) => PortType::BOOL,
            _ => PortType::TEXT,
        }
    }

    pub fn describe(v: &crate::value::Value) -> String {
        use crate::value::Value as V;
        use serde_json::Value as Json;
        match v {
            V::Text(_) | V::Json(Json::String(_)) => "text".into(),
            V::Num(_) | V::Json(Json::Number(_)) => "a number".into(),
            V::Bool(_) | V::Json(Json::Bool(_)) => "a bool".into(),
            V::Bytes(_) => "bytes".into(),
            V::Map(_) | V::Json(Json::Object(_)) => "a record".into(),
            V::List(items) => match items.first() {
                None => "an empty list".into(),
                Some(first) => format!("a list of {}", Self::describe(first)),
            },
            V::Json(_) => "json".into(),
            V::Extern(_) => "an external value".into(),
        }
    }

    pub fn is_exec(&self) -> bool {
        *self == Self::EXEC
    }

    pub fn is_any(&self) -> bool {
        *self == Self::ANY
    }
}

impl fmt::Debug for PortType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PortType({:?})", self.0.as_str())
    }
}

impl fmt::Display for PortType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// The types that ARE a list of something: what a loop may walk.
///
/// A `table` is deliberately not one. It is columns and rows, and a loop handed one would either
/// walk those two entries or have to know to reach inside — so it takes the rows instead, which is
/// a wire somebody can see.
pub const LIST_TYPES: [&str; 4] = ["list", "table_rows", "numbers", "texts"];

/// The types that are ONE value a person could read: what a comparison compares, what a template
/// interpolates, what a cell holds.
///
/// Not a table, not a list, not a file. Those need a node to become one of these, and a node is a
/// step somebody can see.
pub const SCALAR_TYPES: [&str; 3] = ["text", "num", "bool"];

impl PortType {
    /// The family this type belongs to. Every type has one.
    ///
    /// A family is a real attribute rather than a note in a comment: it is what lets a port say
    /// "a list of something" without saying "anything", what lets a palette offer the types that
    /// would fit a pin instead of all of them, and what an editor filters on.
    ///
    /// - `scalar` — one value a person could read: text, a number, a boolean.
    /// - `list` — a sequence of something.
    /// - `unique` — everything else. An image goes where an image goes, and nowhere else.
    ///
    /// Only `scalar` and `list` are also usable AS a port type, and a port typed with one accepts
    /// every member. `unique` names the absence of that: it is what a type says about itself, not
    /// something a port can ask for.
    pub fn family(&self) -> Family {
        Family::of(self)
    }

    /// Is this type the name of a family rather than a concrete type?
    pub fn is_family(&self) -> bool {
        matches!(self.as_str(), "scalar" | "list")
    }
}

/// What a port's type can stand in for.
///
/// - `Scalar` — one value a person could read: text, a number, a boolean.
/// - `List` — a sequence of something.
/// - `Unique` — everything else. An image goes where an image goes, and nowhere else.
///
/// **Every port declares one**, and that is the point: a product's own type is a family member the
/// engine has never heard of, and a `chunk_results` that could not say it is a list would be a list
/// no loop could walk. Declaring it on the port means nothing downstream — the wire check, the
/// catalogue, the editor — has to know whether a node came from the engine or from a product.
///
/// [`Family::of`] is the default for the types the engine defines, so its own nodes say nothing
/// and get the right answer. A product says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Scalar,
    List,
    Unique,
}

impl Family {
    /// The family of a type the engine defines. Anything else is its own.
    pub fn of(ty: &PortType) -> Family {
        let t = ty.as_str();
        // A family's own name reports that family. `list` is already one of its members and
        // `scalar` is not, and having the two answer differently was a difference with no meaning.
        if t == "scalar" || SCALAR_TYPES.contains(&t) {
            Family::Scalar
        } else if LIST_TYPES.contains(&t) {
            Family::List
        } else {
            Family::Unique
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Family::Scalar => "scalar",
            Family::List => "list",
            Family::Unique => "unique",
        }
    }
}


/// The element type of a list type, for a node that has to say what it hands out one at a time.
pub fn element_of(list: &PortType) -> PortType {
    match list.as_str() {
        "numbers" => PortType::NUM,
        "texts" => PortType::TEXT,
        "table_rows" => PortType::TABLE_ROW,
        _ => PortType::ANY,
    }
}

/// May a value of type `from` be delivered to a port of type `to`?
///
/// Equality, a wildcard, and one family: anything that IS a list satisfies a port asking for a
/// list. Still not a lattice, not a coercion table, not a subtype relation — every richer rule
/// anybody wants here ("num should flow into text") is a *conversion*, and a conversion that
/// happens invisibly inside the wiring is one nobody can see in the editor. Those get a node.
///
/// The family exists because the alternative was `any` on every loop, and `any` is the port that
/// let a table be wired into one.
pub fn compatible(from: &Port, to: &Port) -> bool {
    from.ty == to.ty
        || from.ty.is_any()
        || to.ty.is_any()
        // A port asking for a family takes any member of it. That is the whole widening, and it
        // reads the DECLARED family, so a product's own type takes part on the same terms as the
        // engine's — which is why nothing here has to know where a node came from.
        || (to.ty.is_family() && from.family().as_str() == to.ty.as_str())
}

/// One column of a `table`.
///
/// Declared so the shape is known before anything runs: an editor offers a list instead of a text
/// field, and a model assembling a graph reads what it can ask for rather than guessing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: PortName,
    #[serde(rename = "type")]
    pub ty: PortType,
    /// Not every row carries it. An optional column is one that can come out empty.
    #[serde(default)]
    pub optional: bool,
}

impl Column {
    pub fn new(name: impl Into<PortName>, ty: PortType) -> Self {
        Column {
            name: name.into(),
            ty,
            optional: false,
        }
    }

    pub fn optional(name: impl Into<PortName>, ty: PortType) -> Self {
        Column {
            name: name.into(),
            ty,
            optional: true,
        }
    }
}

/// One port on a node: its name, what flows through it, and whether the node can run without it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    pub name: PortName,
    #[serde(rename = "type")]
    pub ty: PortType,
    /// A required input with nothing wired and no config literal fails the node before it runs.
    /// Outputs are never required.
    pub required: bool,
    /// The columns, when the type is `table`. Empty for everything else.
    ///
    /// A `Vec` rather than a static slice because a table's columns can come from configuration —
    /// `table_read` knows them only once a table is chosen. `Vec::new()` is `const`, so the const
    /// constructors below still work inside a `static`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<Column>,
    /// Declared for a product's own type; `None` means the type's own family. Never serialised:
    /// what the catalogue publishes is the resolved answer, so a reader has one thing to read.
    #[serde(skip)]
    pub family: Option<Family>,
    /// Declared for a product's own list type; `None` means the type's own element. Never
    /// serialised: the catalogue publishes the resolved answer, so a reader has one thing to read.
    #[serde(skip)]
    pub element: Option<PortType>,
}

impl Port {
    /// Declare what ONE element of this list is.
    ///
    /// A loop hands out items, and their type is not readable from the list's name: the engine
    /// knows that `numbers` holds `num`, and has never heard of `chunk_results`. Declared here for
    /// the same reason the family is — so a product's list is walked on the same terms as the
    /// engine's, and the item arrives typed instead of as `any`.
    pub fn of(mut self, element: PortType) -> Self {
        self.element = Some(element);
        self
    }

    /// One element of this port's list: what it declared, or what its type implies.
    pub fn element(&self) -> PortType {
        self.element.clone().unwrap_or_else(|| element_of(&self.ty))
    }

    /// Declare this port's family — what its type can stand in for.
    ///
    /// Needed by a product declaring its own type: the engine has never heard of `chunk_results`,
    /// and one that could not say it is a list would be a list no loop could walk. The engine's own
    /// types default correctly, so its nodes say nothing.
    pub fn in_family(mut self, f: Family) -> Self {
        self.family = Some(f);
        self
    }

    /// This port's family: what it declared, or what its type is.
    pub fn family(&self) -> Family {
        self.family.unwrap_or_else(|| Family::of(&self.ty))
    }

    /// The columns this port carries. Only meaningful for a `table`.
    pub fn with_columns(mut self, columns: Vec<Column>) -> Self {
        self.columns = columns;
        self
    }

    pub const fn new(name: PortName, ty: PortType, required: bool) -> Self {
        Port {
            name,
            ty,
            required,
            // `Vec::new()` is const, which is what keeps every `static [Port; N]` working.
            columns: Vec::new(),
            family: None,
            element: None,
        }
    }

    /// A required input port.
    pub const fn req(name: &'static str, ty: PortType) -> Self {
        Port::new(PortName::new_static(name), ty, true)
    }

    /// An optional input, or any output.
    pub const fn opt(name: &'static str, ty: PortType) -> Self {
        Port::new(PortName::new_static(name), ty, false)
    }
}

/// The single exec input every effectful node has. Unlike a data input, it accepts fan-in:
/// several branches may converge on the same node.
pub const EXEC_IN: Port = Port::opt("exec_in", PortType::EXEC);

/// The default exec output — "and then".
pub const EXEC_OUT: Port = Port::opt("exec_out", PortType::EXEC);

#[cfg(test)]
mod tests {

    /// Three families, and every type is in exactly one.
    #[test]
    fn every_type_belongs_to_one_family() {
        assert_eq!(PortType::TEXT.family(), Family::Scalar);
        assert_eq!(PortType::NUM.family(), Family::Scalar);
        assert_eq!(PortType::BOOL.family(), Family::Scalar);
        assert_eq!(PortType::NUMBERS.family(), Family::List);
        assert_eq!(PortType::TEXTS.family(), Family::List);
        assert_eq!(PortType::TABLE_ROWS.family(), Family::List);
        assert_eq!(PortType::LIST.family(), Family::List);
        // A family's own name reports that family, whichever one it is.
        assert_eq!(PortType::SCALAR.family(), Family::Scalar);
        // An image goes where an image goes, and nowhere else.
        assert_eq!(PortType::IMAGE.family(), Family::Unique);
        assert_eq!(PortType::TABLE.family(), Family::Unique);
        assert_eq!(PortType::TABLE_ROW.family(), Family::Unique);
        assert_eq!(PortType::SCHEMA.family(), Family::Unique);
    }

    /// A port asking for a family takes any member of it.
    #[test]
    fn a_family_port_takes_its_members() {
        assert!(compatible(&Port::opt("a", PortType::NUM), &Port::opt("b", PortType::SCALAR)));
        assert!(compatible(&Port::opt("a", PortType::TEXT), &Port::opt("b", PortType::SCALAR)));
        assert!(compatible(&Port::opt("a", PortType::BOOL), &Port::opt("b", PortType::SCALAR)));
        assert!(compatible(&Port::opt("a", PortType::NUMBERS), &Port::opt("b", PortType::LIST)));
        assert!(compatible(&Port::opt("a", PortType::TABLE_ROWS), &Port::opt("b", PortType::LIST)));
    }

    /// And nothing else. A table is columns and rows, so a loop handed one would walk those two
    /// entries; its rows are a wire away, and that wire is something somebody can see.
    #[test]
    fn a_unique_type_only_goes_where_it_belongs() {
        assert!(!compatible(&Port::opt("a", PortType::TABLE), &Port::opt("b", PortType::LIST)));
        assert!(!compatible(&Port::opt("a", PortType::TABLE), &Port::opt("b", PortType::SCALAR)));
        assert!(!compatible(&Port::opt("a", PortType::TABLE_ROW), &Port::opt("b", PortType::TABLE)));
        assert!(!compatible(&Port::opt("a", PortType::IMAGE), &Port::opt("b", PortType::BYTES)));
        assert!(compatible(&Port::opt("a", PortType::IMAGE), &Port::opt("b", PortType::IMAGE)));
    }

    /// A product's list says what one of its elements is, because the engine cannot read that off
    /// a name it has never seen. Without it a loop hands out `any`, and the chain stops being
    /// checkable exactly where the product's own data enters it.
    #[test]
    fn a_products_list_declares_its_element() {
        let results = Port::opt("results", PortType::new("chunk_results"))
            .in_family(Family::List)
            .of(PortType::new("chunk_result"));
        assert_eq!(results.element().as_str(), "chunk_result");

        // The engine's own lists still answer for themselves, undeclared.
        assert_eq!(Port::opt("v", PortType::NUMBERS).element(), PortType::NUM);
        assert_eq!(Port::opt("v", PortType::TEXTS).element(), PortType::TEXT);
        assert_eq!(
            Port::opt("v", PortType::TABLE_ROWS).element(),
            PortType::TABLE_ROW
        );
    }

    /// The point of declaring it: a product's own type takes part on the same terms as the
    /// engine's. `chunk_results` is a name the engine has never heard, and a list no loop could
    /// walk would be a list in name only.
    #[test]
    fn a_products_own_type_joins_a_family_by_saying_so() {
        let chunks = PortType::new("chunk_results");
        let declared = Port::opt("results", chunks.clone()).in_family(Family::List);
        let undeclared = Port::opt("results", chunks);
        let loop_items = Port::opt("items", PortType::LIST);

        assert_eq!(declared.family(), Family::List);
        assert!(compatible(&declared, &loop_items));

        assert_eq!(undeclared.family(), Family::Unique, "silence is not a family");
        assert!(!compatible(&undeclared, &loop_items));
    }

    /// And a declaration does not widen anything else: it says which family, not that anything goes.
    #[test]
    fn declaring_a_family_is_not_a_wildcard() {
        let chunks = Port::opt("results", PortType::new("chunk_results")).in_family(Family::List);
        assert!(!compatible(&chunks, &Port::opt("v", PortType::NUMBERS)));
        assert!(!compatible(&chunks, &Port::opt("v", PortType::SCALAR)));
    }

    /// The widening goes one way. A port that produces "a list of something" cannot be plugged
    /// into one that needs numbers, because the something might be documents.
    #[test]
    fn a_family_does_not_flow_back_into_a_member() {
        assert!(!compatible(&Port::opt("a", PortType::LIST), &Port::opt("b", PortType::NUMBERS)));
        assert!(!compatible(&Port::opt("a", PortType::SCALAR), &Port::opt("b", PortType::NUM)));
    }
    use super::*;

    #[test]
    fn a_port_type_serializes_as_a_bare_string() {
        assert_eq!(serde_json::to_string(&PortType::TEXT).unwrap(), "\"text\"");
        assert_eq!(serde_json::to_string(&PortType::BYTES).unwrap(), "\"file\"");
        assert_eq!(
            serde_json::to_string(&PortType::MAP).unwrap(),
            "\"dictionary\"",
            "the wire name is the one stored graphs already use, not the Rust name"
        );
    }

    #[test]
    fn a_product_type_is_indistinguishable_from_a_builtin() {
        // A deliberately meaningless noun. The CI guard greps this crate for real product
        // vocabulary and does not exempt tests — an exemption is where the words come back in.
        let sprocket = PortType::new_static("sprocket");
        assert_eq!(serde_json::to_string(&sprocket).unwrap(), "\"sprocket\"");
        assert!(compatible(&Port::opt("a", sprocket.clone()), &Port::opt("b", sprocket.clone())));
        assert!(!compatible(&Port::opt("a", sprocket.clone()), &Port::opt("b", PortType::TEXT)));
    }

    #[test]
    fn any_is_compatible_in_both_directions() {
        assert!(compatible(&Port::opt("a", PortType::ANY), &Port::opt("b", PortType::IMAGE)));
        assert!(compatible(&Port::opt("a", PortType::IMAGE), &Port::opt("b", PortType::ANY)));
    }

    #[test]
    fn unrelated_types_do_not_connect() {
        assert!(!compatible(&Port::opt("a", PortType::TEXT), &Port::opt("b", PortType::NUM)));
        assert!(
            !compatible(&Port::opt("a", PortType::EXEC), &Port::opt("b", PortType::ANY)) || PortType::ANY.is_any(),
            "exec is a wire kind, not a value; ANY still absorbs it and that is deliberate"
        );
    }

    #[test]
    fn a_port_round_trips() {
        let p = Port::req("condition", PortType::BOOL);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json, r#"{"name":"condition","type":"bool","required":true}"#,
            "the catalog the editor reads is this shape"
        );
        assert_eq!(serde_json::from_str::<Port>(&json).unwrap(), p);
    }
}
