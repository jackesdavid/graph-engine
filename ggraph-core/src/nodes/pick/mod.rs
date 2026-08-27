// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Pulling one column out of a table.
//!
//! This is how a table becomes something a chart can take: `table` in, one named column out, as
//! `numbers` or as `texts`.
//!
//! Two nodes rather than one, for the reason the rounding pair exists: a node that returned numbers
//! or text depending on what it found would have an output port that cannot say what it produces,
//! and everything downstream would have to accept either.

mod numbers;
mod texts;

use crate::host::Host;
use crate::registry::NodeRegistry;
use crate::value::Value;
use serde_json::Value as Json;

pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(numbers::spec());
    reg.register(texts::spec());
}

/// The column to read, from config.
pub(crate) fn column(cfg: &Json) -> Option<String> {
    cfg.get("column")
        .and_then(Json::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// One cell of one row, whatever shape the row arrived in.
pub(crate) fn at<'a>(row: &'a Value, name: &str) -> Option<FieldRef<'a>> {
    match row {
        Value::Json(Json::Object(map)) => map.get(name).map(FieldRef::Json),
        Value::Map(pairs) => pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| FieldRef::Value(v)),
        _ => None,
    }
}

pub(crate) enum FieldRef<'a> {
    Json(&'a Json),
    Value(&'a Value),
}

impl FieldRef<'_> {
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            FieldRef::Json(j) => j.as_f64(),
            FieldRef::Value(v) => v.as_f64().filter(|_| v.as_text().is_none()),
        }
    }

    pub(crate) fn as_text(&self) -> String {
        match self {
            FieldRef::Json(Json::String(s)) => s.clone(),
            FieldRef::Json(Json::Null) => String::new(),
            FieldRef::Json(other) => other.to_string(),
            FieldRef::Value(v) => v.as_text().unwrap_or_else(|| v.summary()),
        }
    }
}
