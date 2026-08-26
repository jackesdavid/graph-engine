// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The values that travel along edges.
//!
//! [`Value`] is a closed enum of the shapes the core can reason about, plus one escape hatch —
//! [`Value::Extern`] — through which a product carries its own types. A camera, a face
//! embedding, a parsed PDF: the core moves them along wires and never looks inside.
//!
//! Three deliberate choices, each of which cost something to get wrong before:
//!
//! **`Extern` carries a type name, not just an `Any`.** The tempting design is
//! `Arc<dyn Any>` plus downcasting. It loses the one thing persistence and debugging both need:
//! what this value *is*. A tagged codec cannot write `{"t":"camera",…}` from a `TypeId`, and a
//! run log cannot say "a camera" either. So [`ExternValue`] declares its name and the name must
//! equal the [`PortType`](crate::PortType) string the value travels on.
//!
//! **`Extern` is not a generic parameter.** `Value<X>` looks cleaner until `List(Vec<Value<X>>)`
//! makes the parameter recursive, at which point it infects the port map, the node spec, the
//! registry, the codec and every scheduler call site — and two products can no longer share one
//! compiled scheduler. The dynamic dispatch here is paid once per node output, which is
//! nothing next to what a node does.
//!
//! **`to_json` may return `None`.** Not every value should survive a restart — a decoded 4 MB
//! frame has no business in a database column. `None` means "dropped, on purpose", and the
//! codec records the drop rather than inventing a placeholder that later reads as real data.

use crate::port::PortType;
use serde_json::Value as Json;
use smol_str::SmolStr;
use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// A number on a wire.
///
/// Integer and float are one `PortType` because a workflow author does not think in machine
/// types — `days_of_cover` being fractional is not a different kind of port from `count` being
/// whole. They are separate *variants* because rounding silently is how a total stops adding up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }

    /// The integer value, or `None` if this is a float that is not a whole number.
    ///
    /// Not a silent truncation: a node that needs an index must know that `2.5` is not one.
    pub fn as_i64(self) -> Option<i64> {
        match self {
            Num::Int(i) => Some(i),
            Num::Float(f) if f.fract() == 0.0 && f.is_finite() => Some(f as i64),
            Num::Float(_) => None,
        }
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Num::Int(i) => write!(f, "{i}"),
            Num::Float(x) => write!(f, "{x}"),
        }
    }
}

/// Opaque bytes plus what they are. `Arc` because a frame or a document is passed to several
/// downstream nodes and copying it per edge is how a graph runs out of memory.
#[derive(Clone)]
pub struct Bytes {
    pub mime: SmolStr,
    pub name: Option<SmolStr>,
    pub data: Arc<[u8]>,
}

impl Bytes {
    pub fn new(mime: impl AsRef<str>, data: impl Into<Arc<[u8]>>) -> Self {
        Bytes {
            mime: SmolStr::new(mime.as_ref()),
            name: None,
            data: data.into(),
        }
    }

    pub fn named(mut self, name: impl AsRef<str>) -> Self {
        self.name = Some(SmolStr::new(name.as_ref()));
        self
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the bytes themselves: this ends up in run logs.
        write!(
            f,
            "Bytes({}, {} bytes{})",
            self.mime,
            self.data.len(),
            match &self.name {
                Some(n) => format!(", {n:?}"),
                None => String::new(),
            }
        )
    }
}

/// A product's own value type, carried by the core without being understood by it.
pub trait ExternValue: fmt::Debug + Send + Sync + 'static {
    /// What this is. **Must equal the `PortType` string it travels on** — that identity is what
    /// lets the codec round-trip it and the editor colour the wire.
    fn type_name(&self) -> &'static str;

    /// For a product that needs its concrete type back out.
    fn as_any(&self) -> &dyn Any;

    /// How this value survives a restart, or `None` if it should not.
    ///
    /// The `io` handle is there for values too large to inline: put the bytes in the blob store
    /// and return a reference to them. Returning `None` is a legitimate answer, and the default.
    fn to_json(&self, _io: &dyn crate::host::ValueIo) -> Option<Json> {
        None
    }

    /// One line for a run log. Never the payload.
    fn summary(&self) -> String {
        self.type_name().to_string()
    }

    /// Text coercion, for nodes like `format` that stringify whatever they are handed.
    fn as_text(&self) -> Option<String> {
        None
    }
}

/// A value on a wire.
#[derive(Clone, Debug)]
pub enum Value {
    Text(String),
    Num(Num),
    Bool(bool),
    Bytes(Bytes),
    Json(Json),
    List(Vec<Value>),
    /// Key/value pairs. A `Vec` rather than a map because author-declared column order is
    /// meaningful — a table built by a graph is read by a human.
    Map(Vec<(String, Value)>),
    Extern(Arc<dyn ExternValue>),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Self {
        Value::Text(s.into())
    }

    pub fn int(i: i64) -> Self {
        Value::Num(Num::Int(i))
    }

    pub fn float(f: f64) -> Self {
        Value::Num(Num::Float(f))
    }

    pub fn ext(v: impl ExternValue) -> Self {
        Value::Extern(Arc::new(v))
    }

    /// The port type this value travels on.
    ///
    /// `Bytes` reports `image` or `file` from its mime — that is how an image port stays
    /// distinguishable from an arbitrary download without a separate variant.
    pub fn port_type(&self) -> PortType {
        match self {
            Value::Text(_) => PortType::TEXT,
            Value::Num(_) => PortType::NUM,
            Value::Bool(_) => PortType::BOOL,
            Value::Bytes(b) if b.mime.starts_with("image/") => PortType::IMAGE,
            Value::Bytes(_) => PortType::BYTES,
            Value::Json(_) => PortType::JSON,
            Value::List(_) => PortType::LIST,
            Value::Map(_) => PortType::MAP,
            Value::Extern(e) => PortType::new(e.type_name()),
        }
    }

    /// Text, coercing the scalars. Returns `None` for shapes with no honest text form —
    /// a node that wants "whatever this is, as a string" should use [`summary`](Self::summary).
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Text(s) => Some(s.clone()),
            Value::Num(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Extern(e) => e.as_text(),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<Num> {
        match self {
            Value::Num(n) => Some(*n),
            // A number that arrived as text (a config literal, an HTTP field) is still a number.
            // Refusing it here just moves the parse into every node.
            Value::Text(s) => s
                .trim()
                .parse::<i64>()
                .ok()
                .map(Num::Int)
                .or_else(|| s.trim().parse::<f64>().ok().map(Num::Float)),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_num().and_then(Num::as_i64)
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_num().map(Num::as_f64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&Bytes> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(v) => Some(v),
            _ => None,
        }
    }

    /// The product's concrete type, if this is that type.
    pub fn downcast<T: ExternValue>(&self) -> Option<&T> {
        match self {
            Value::Extern(e) => e.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }

    /// One line for a run log. Never a payload, never unbounded.
    pub fn summary(&self) -> String {
        const CAP: usize = 120;
        match self {
            Value::Text(s) if s.chars().count() > CAP => {
                let head: String = s.chars().take(CAP).collect();
                format!("{head}… ({} chars)", s.chars().count())
            }
            Value::Text(s) => s.clone(),
            Value::Num(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Bytes(b) => format!("{} ({} bytes)", b.mime, b.data.len()),
            Value::Json(_) => "json".to_string(),
            Value::List(v) => format!("{} item(s)", v.len()),
            Value::Map(m) => format!("{} field(s)", m.len()),
            Value::Extern(e) => e.summary(),
        }
    }
}

impl PartialEq for Value {
    /// Structural equality for the shapes that have it. **Two `Extern`s are never equal** — the
    /// core has no way to compare them, and answering `false` is honest where answering `true`
    /// by pointer identity would be a lie that a `compare` node would act on.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Json(a), Value::Json(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a.mime == b.mime && a.data == b.data,
            _ => false,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::int(i)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::float(f)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

/// A node's inputs, or a node's outputs: values by port name.
pub type PortValues = std::collections::HashMap<crate::id::PortName, Value>;
