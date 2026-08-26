// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The two open identifiers: which node kind, and which port.
//!
//! These are newtypes over `SmolStr`, not enums, and that is the whole reason this engine can
//! serve two products. A closed `enum NodeKind` forces every kind either product will ever have
//! into one crate; a string resolved through a [`NodeRegistry`](crate::NodeRegistry) lets each
//! product register its own and keeps the core ignorant of both.
//!
//! **They serialize transparently**, as bare strings. That is what made the original extraction
//! a refactor rather than a migration: the closed enum it replaced was
//! `#[serde(rename_all = "snake_case")]` and its hand-written slug method returned
//! byte-identical text for all ninety-one of its variants, so stored graph documents load
//! unchanged.
//!
//! `SmolStr` rather than `String` or `Arc<str>`: the scheduler clones port names in its inner
//! loop, and a `SmolStr` clone is a 24-byte memcpy with no allocation and no atomic. Names up to
//! 23 bytes live inline. That is not much headroom — `break_person_detection` is 22 — so
//! [`NodeId::is_inline`] exists and the registry asserts over it in a test rather than trusting
//! it silently.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt;

macro_rules! str_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// A compile-time constant. Node kinds and builtin port types are declared with this.
            pub const fn new_static(s: &'static str) -> Self {
                $name(SmolStr::new_static(s))
            }

            pub fn new(s: impl AsRef<str>) -> Self {
                $name(SmolStr::new(s.as_ref()))
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Whether this value avoids the heap. Cheap clones depend on it.
            pub fn is_inline(&self) -> bool {
                !self.0.is_heap_allocated()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0.as_str())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0.as_str())
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self { Self::new(s) }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self { Self::new(s) }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str { self.0.as_str() }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool { self.0.as_str() == other }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool { self.0.as_str() == *other }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool { self.0.as_str() == other.as_str() }
        }

        // The reversed directions too. Equality that only works one way around is a trap
        // people fall into once each and then work around by reordering the operands.
        impl PartialEq<$name> for str {
            fn eq(&self, other: &$name) -> bool { self == other.0.as_str() }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool { *self == other.0.as_str() }
        }

        impl PartialEq<$name> for String {
            fn eq(&self, other: &$name) -> bool { self.as_str() == other.0.as_str() }
        }

        /// Lets a `HashMap` keyed by this type be looked up with a plain `&str`.
        ///
        /// `PortValues` is keyed by [`PortName`], and every node reads its inputs by name.
        /// Without this each read allocates a `PortName` to throw away immediately — and worse,
        /// the obvious `map.get(name)` simply does not compile, which pushes callers into
        /// building the key by hand at every site.
        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str { self.0.as_str() }
        }
    };
}

str_id! {
    /// Which kind of node this is — `"if"`, `"for_each"`, `"camera_snapshot"`.
    ///
    /// Resolved to a [`NodeSpec`](crate::NodeSpec) through the registry. A graph document may
    /// name a kind the registry does not know; that is an error at load time, with the name in
    /// the message, not a panic and not a silent skip.
    NodeId
}

str_id! {
    /// The name of a port on a node — `"condition"`, `"exec_in"`, `"true"`.
    ///
    /// Ports are addressed by name, never by index: an edge that survives a node gaining a port
    /// is the difference between an additive change and a migration.
    PortName
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_serializes_as_a_bare_string() {
        let id = NodeId::new_static("camera_snapshot");
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            "\"camera_snapshot\"",
            "an id must be indistinguishable on the wire from the enum variant it replaced"
        );
        let back: NodeId = serde_json::from_str("\"camera_snapshot\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    // The owned strings are the point: this asserts the impls exist, not that they are the
    // efficient way to compare.
    #[allow(clippy::cmp_owned)]
    fn equality_with_a_string_works_in_both_directions() {
        let id = NodeId::new_static("if");
        assert!(id == "if");
        assert!("if" == id);
        assert!(String::from("if") == id);
        assert!(id == String::from("if"));
    }

    #[test]
    fn a_map_keyed_by_a_name_is_readable_with_a_plain_str() {
        // Hash and Eq have to agree with the borrowed form for this to be sound; the assertion
        // is that a lookup actually finds the entry, which is what proves they do.
        let mut m = std::collections::HashMap::new();
        m.insert(PortName::new_static("condition"), 1);
        assert_eq!(
            m.get("condition"),
            Some(&1),
            "without Borrow<str> every input read allocates a key to throw away"
        );
    }

    #[test]
    fn an_id_that_would_allocate_is_visible() {
        assert!(NodeId::new_static("break_person_detection").is_inline());
        assert!(
            !NodeId::new("a_node_kind_name_far_past_the_inline_budget").is_inline(),
            "is_inline must actually distinguish, or the registry's assertion proves nothing"
        );
    }
}
