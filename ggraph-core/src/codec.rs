//! Turning values into something a database can hold, and back.
//!
//! Deliberately **not** `#[derive(Serialize)]` on [`Value`]. A derive would round-trip the shape
//! and quietly lie about two things that matter:
//!
//! - **Bytes do not belong in a column.** A decoded image is megabytes; a hundred of them in a
//!   `values_json` column is a table nobody can query and a row nobody can read. Bytes go to the
//!   blob store and what is persisted is a reference.
//! - **A product's own types are not the core's to encode.** `Extern` asks the value itself, and
//!   the value is allowed to answer "do not persist me". A derive has no way to express that and
//!   would invent a shape that decodes back into something plausible and wrong.
//!
//! ## The tag is the port type
//!
//! Everything is written as `{"t": <type>, "v": <body>}`, where `t` is the value's port type
//! string. That is the same string the wire is typed with and the same string the editor colours
//! by, so a persisted value cannot disagree with the port it came off.
//!
//! ## Dropping is a legitimate outcome
//!
//! [`encode`] returns `None` for a value that should not survive, and [`decode`] returns `None`
//! for one that cannot be reconstructed — a blob whose key is gone, a product type this build
//! does not register. Both are *absences*, and a graph reads an absence the same way it reads
//! any other: as "not knowing", which every node in the standard set is built to route rather
//! than to guess about.

use crate::host::ValueIo;
use crate::id::PortName;
use crate::registry::Decoder;
use crate::value::{Bytes, Num, PortValues, Value};
use serde_json::{json, Map, Value as Json};
use std::collections::HashMap;

/// Write a value in a form that survives a restart, or `None` if it should not.
pub fn encode(v: &Value, io: &dyn ValueIo) -> Option<Json> {
    Some(match v {
        Value::Text(s) => json!({ "t": "text", "v": s }),
        Value::Num(Num::Int(i)) => json!({ "t": "num", "v": i }),
        Value::Num(Num::Float(f)) => json!({ "t": "num", "v": f }),
        Value::Bool(b) => json!({ "t": "bool", "v": b }),
        Value::Json(j) => json!({ "t": "json", "v": j }),

        Value::Bytes(b) => {
            // A reference, never the bytes. With no blob store the value is dropped rather than
            // inlined — an inlined frame is how a state table becomes unreadable.
            if !io.enabled() {
                return None;
            }
            let key = io.put(&b.data, &b.mime).ok()?;
            json!({ "t": "bytes", "k": key, "mime": b.mime, "name": b.name })
        }

        Value::List(items) => {
            // All or nothing. A list that silently lost its third element is worse than an
            // absent list: a graph counting it gets a smaller number and no way to know.
            let encoded: Option<Vec<Json>> = items.iter().map(|i| encode(i, io)).collect();
            json!({ "t": "list", "v": encoded? })
        }

        Value::Map(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                out.push(json!({ "k": k, "v": encode(v, io)? }));
            }
            json!({ "t": "map", "v": out })
        }

        Value::Extern(e) => {
            let body = e.to_json(io)?;
            json!({ "t": e.type_name(), "v": body })
        }
    })
}

/// Read a value back, or `None` if it cannot be reconstructed.
pub fn decode(
    j: &Json,
    io: &dyn ValueIo,
    decoders: &HashMap<&'static str, Decoder>,
) -> Option<Value> {
    let tag = j.get("t")?.as_str()?;
    let body = j.get("v");
    match tag {
        "text" => Some(Value::Text(body?.as_str()?.to_string())),
        "num" => {
            let n = body?.as_number()?;
            n.as_i64()
                .map(Value::int)
                .or_else(|| n.as_f64().map(Value::float))
        }
        "bool" => Some(Value::Bool(body?.as_bool()?)),
        "json" => Some(Value::Json(body?.clone())),

        "bytes" => {
            let key = j.get("k")?.as_str()?;
            // The blob may be gone — a TTL, a bucket lifecycle rule, a different environment.
            // Absent is the honest answer; a zero-length buffer is not.
            let data = io.get(key).ok()?;
            let mime = j
                .get("mime")
                .and_then(Json::as_str)
                .unwrap_or("application/octet-stream");
            let mut b = Bytes::new(mime, data);
            if let Some(n) = j.get("name").and_then(Json::as_str) {
                b = b.named(n);
            }
            Some(Value::Bytes(b))
        }

        "list" => {
            let items: Option<Vec<Value>> = body?
                .as_array()?
                .iter()
                .map(|i| decode(i, io, decoders))
                .collect();
            Some(Value::List(items?))
        }

        "map" => {
            let mut out = Vec::new();
            for pair in body?.as_array()? {
                let k = pair.get("k")?.as_str()?.to_string();
                out.push((k, decode(pair.get("v")?, io, decoders)?));
            }
            Some(Value::Map(out))
        }

        // A product type. Unknown here means this build does not register it — a different
        // service, an older deploy — and inventing something would be worse than the absence.
        other => decoders.get(other).and_then(|f| f(body?, io)),
    }
}

/// A whole port map, for checkpointing a node's outputs.
///
/// Values that cannot be persisted are **left out, not failed**. A node whose image output is
/// dropped still has its other five outputs worth keeping, and refusing the lot because of one
/// would mean a graph with an image anywhere in it cannot be checkpointed at all.
pub fn encode_ports(values: &PortValues, io: &dyn ValueIo) -> Json {
    let mut m = Map::new();
    for (name, v) in values {
        if let Some(j) = encode(v, io) {
            m.insert(name.as_str().to_string(), j);
        }
    }
    Json::Object(m)
}

pub fn decode_ports(
    j: &Json,
    io: &dyn ValueIo,
    decoders: &HashMap<&'static str, Decoder>,
) -> PortValues {
    let mut out = PortValues::new();
    let Some(obj) = j.as_object() else {
        return out;
    };
    for (name, v) in obj {
        if let Some(v) = decode(v, io, decoders) {
            out.insert(PortName::new(name), v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Disabled;
    use crate::value::ExternValue;
    use std::any::Any;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemIo(Mutex<HashMap<String, Vec<u8>>>);

    impl ValueIo for MemIo {
        fn enabled(&self) -> bool {
            true
        }
        fn put(&self, bytes: &[u8], _mime: &str) -> Result<String, crate::host::HostError> {
            let key = format!("k{}", self.0.lock().unwrap().len());
            self.0.lock().unwrap().insert(key.clone(), bytes.to_vec());
            Ok(key)
        }
        fn get(&self, key: &str) -> Result<Vec<u8>, crate::host::HostError> {
            self.0
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| crate::host::HostError("gone".into()))
        }
    }

    #[derive(Debug)]
    struct Sprocket(u32);

    impl ExternValue for Sprocket {
        fn type_name(&self) -> &'static str {
            "sprocket"
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn to_json(&self, _: &dyn ValueIo) -> Option<Json> {
            Some(json!({ "n": self.0 }))
        }
    }

    #[derive(Debug)]
    struct Ephemeral;

    impl ExternValue for Ephemeral {
        fn type_name(&self) -> &'static str {
            "ephemeral"
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        // No to_json: this value does not survive, on purpose.
    }

    fn decoders() -> HashMap<&'static str, Decoder> {
        let mut d: HashMap<&'static str, Decoder> = HashMap::new();
        d.insert("sprocket", |body, _| {
            Some(Value::ext(Sprocket(body.get("n")?.as_u64()? as u32)))
        });
        d
    }

    fn round_trip(v: Value) -> Option<Value> {
        let io = MemIo::default();
        let j = encode(&v, &io)?;
        decode(&j, &io, &decoders())
    }

    #[test]
    fn scalars_survive() {
        assert_eq!(round_trip(Value::text("hi")), Some(Value::text("hi")));
        assert_eq!(round_trip(Value::int(-4)), Some(Value::int(-4)));
        assert_eq!(round_trip(Value::float(1.5)), Some(Value::float(1.5)));
        assert_eq!(round_trip(Value::Bool(true)), Some(Value::Bool(true)));
    }

    #[test]
    fn a_whole_float_stays_a_float() {
        // Worth pinning down, because the obvious guess is that it does not: 2.0 and 2 look the
        // same in JSON text. serde_json keeps the distinction internally and `as_i64` refuses a
        // value that arrived as a float, so a column that held a rate of 2.0 does not come back
        // as a count of 2 — which would then divide differently.
        assert_eq!(round_trip(Value::float(2.0)), Some(Value::float(2.0)));
        assert_eq!(round_trip(Value::float(2.5)), Some(Value::float(2.5)));
        assert_eq!(round_trip(Value::int(2)), Some(Value::int(2)));
    }

    #[test]
    fn bytes_are_stored_by_reference_not_inline() {
        let io = MemIo::default();
        let v = Value::Bytes(Bytes::new("image/jpeg", vec![1u8; 4096]));
        let j = encode(&v, &io).unwrap();
        assert!(
            j.to_string().len() < 200,
            "the encoded form is a reference, not the payload: {} bytes",
            j.to_string().len()
        );
        assert_eq!(j["t"], "bytes");
        let back = decode(&j, &io, &decoders()).unwrap();
        assert_eq!(back.as_bytes().unwrap().len(), 4096);
    }

    #[test]
    fn without_a_blob_store_bytes_are_dropped_rather_than_inlined() {
        assert!(
            encode(
                &Value::Bytes(Bytes::new("image/jpeg", vec![1, 2, 3])),
                &Disabled
            )
            .is_none(),
            "an inlined frame is how a state table becomes something nobody can read"
        );
    }

    #[test]
    fn a_blob_that_is_gone_decodes_to_absent_not_to_empty() {
        let io = MemIo::default();
        let j = encode(&Value::Bytes(Bytes::new("image/png", vec![9; 10])), &io).unwrap();
        io.0.lock().unwrap().clear(); // a TTL, a lifecycle rule, another environment
        assert!(
            decode(&j, &io, &decoders()).is_none(),
            "a zero-length buffer would read downstream as a real, empty image"
        );
    }

    #[test]
    fn a_product_type_round_trips_through_its_own_encoder() {
        let back = round_trip(Value::ext(Sprocket(7))).unwrap();
        assert_eq!(back.downcast::<Sprocket>().map(|s| s.0), Some(7));
    }

    #[test]
    fn a_product_type_may_refuse_to_be_persisted() {
        assert!(encode(&Value::ext(Ephemeral), &MemIo::default()).is_none());
    }

    #[test]
    fn an_unregistered_tag_decodes_to_absent() {
        let io = MemIo::default();
        let j = json!({ "t": "something_this_build_never_heard_of", "v": 1 });
        assert!(
            decode(&j, &io, &decoders()).is_none(),
            "a different service or an older deploy — inventing a value would be worse"
        );
    }

    #[test]
    fn a_list_is_all_or_nothing() {
        let io = MemIo::default();
        let v = Value::List(vec![Value::int(1), Value::ext(Ephemeral), Value::int(3)]);
        assert!(
            encode(&v, &io).is_none(),
            "a list that silently lost its middle element gives a graph a smaller count and no \
             way to know"
        );
    }

    #[test]
    fn a_port_map_keeps_what_it_can_rather_than_failing_wholesale() {
        let io = MemIo::default();
        let mut vals = PortValues::new();
        vals.insert(PortName::new("width"), Value::int(1920));
        vals.insert(PortName::new("preview"), Value::ext(Ephemeral));
        let j = encode_ports(&vals, &io);
        let back = decode_ports(&j, &io, &decoders());
        assert_eq!(back.len(), 1, "the five outputs worth keeping are kept");
        assert_eq!(
            back.get(&PortName::new("width")).and_then(Value::as_i64),
            Some(1920)
        );
        assert!(!back.contains_key(&PortName::new("preview")));
    }

    #[test]
    fn the_tag_is_the_port_type() {
        let io = MemIo::default();
        for v in [Value::text("a"), Value::int(1), Value::Bool(true)] {
            let ty = v.port_type();
            let j = encode(&v, &io).unwrap();
            assert_eq!(
                j["t"].as_str(),
                Some(ty.as_str()),
                "a persisted value must not be able to disagree with the port it came off"
            );
        }
        let j = encode(&Value::ext(Sprocket(1)), &io).unwrap();
        assert_eq!(j["t"], "sprocket");
    }
}
