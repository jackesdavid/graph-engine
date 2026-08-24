//! The two open identifiers: which node kind, and which port.
//!
//! These are newtypes over `SmolStr`, not enums, and that is the whole reason this engine can
//! serve two products. A closed `enum NodeKind` forces every kind either product will ever have
//! into one crate; a string resolved through a [`NodeRegistry`](crate::NodeRegistry) lets each
//! product register its own and keeps the core ignorant of both.
//!
//! **They serialize transparently**, as bare strings. That is what made the extraction from
//! Sentinel a refactor rather than a migration: its `NodeKind` was
//! `#[serde(rename_all = "snake_case")]` and its hand-written `slug()` returned byte-identical
//! text for all 91 kinds, so stored graph documents load unchanged.
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
    fn an_id_that_would_allocate_is_visible() {
        assert!(NodeId::new_static("break_person_detection").is_inline());
        assert!(
            !NodeId::new("a_node_kind_name_far_past_the_inline_budget").is_inline(),
            "is_inline must actually distinguish, or the registry's assertion proves nothing"
        );
    }
}
