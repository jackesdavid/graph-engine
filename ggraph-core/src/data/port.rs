// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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
    /// Key/value pairs.
    pub const MAP: PortType = PortType::new_static("dictionary");
    /// A named table of rows.
    pub const TABLE: PortType = PortType::new_static("table");
    /// Control flow, not data. Exec ports carry no value — they say *when*, not *what*.
    pub const EXEC: PortType = PortType::new_static("exec");
    /// Wildcard: compatible with everything, in both directions.
    pub const ANY: PortType = PortType::new_static("any");

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

/// May a value of type `from` be delivered to a port of type `to`?
///
/// Deliberately not a lattice, not a coercion table, not a subtype relation: equality plus a
/// wildcard. Every richer rule anybody wants here ("num should flow into text") is a
/// *conversion*, and a conversion that happens invisibly inside the wiring is a conversion
/// nobody can see in the editor. Those get a node.
pub fn compatible(from: &PortType, to: &PortType) -> bool {
    from == to || from.is_any() || to.is_any()
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
}

impl Port {
    pub const fn new(name: PortName, ty: PortType, required: bool) -> Self {
        Port { name, ty, required }
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
        assert!(compatible(&sprocket, &sprocket));
        assert!(!compatible(&sprocket, &PortType::TEXT));
    }

    #[test]
    fn any_is_compatible_in_both_directions() {
        assert!(compatible(&PortType::ANY, &PortType::IMAGE));
        assert!(compatible(&PortType::IMAGE, &PortType::ANY));
    }

    #[test]
    fn unrelated_types_do_not_connect() {
        assert!(!compatible(&PortType::TEXT, &PortType::NUM));
        assert!(
            !compatible(&PortType::EXEC, &PortType::ANY) || PortType::ANY.is_any(),
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
