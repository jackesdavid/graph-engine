// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! The catalog, frozen.
//!
//! `catalog_json()` is what an editor builds its palette from: every slug, label, category,
//! typed port and default config. Nothing else in a Rust codebase notices when it changes —
//! `cargo check` is perfectly happy with a renamed slug, and the first thing that notices is a
//! saved graph that will not open.
//!
//! So it is committed and compared byte for byte. A diff here is sometimes right (a new node)
//! and sometimes a graph somebody saved last month failing to load. The point is that it is
//! never silent.
//!
//! Regenerate deliberately:
//!
//!     UPDATE_CATALOG=1 cargo test -p ggraph-core --test catalog

use ggraph_core::host::testkit::TestHost;
use ggraph_core::NodeRegistry;

const SNAPSHOT: &str = include_str!("catalog_snapshot.json");

fn registry() -> NodeRegistry<TestHost> {
    let mut r = NodeRegistry::new();
    ggraph_core::nodes::register_all(&mut r, &ggraph_core::Services::none());
    r
}

fn rendered() -> String {
    let mut s = serde_json::to_string_pretty(&registry().catalog_json()).expect("serializes");
    s.push('\n');
    s
}

#[test]
fn the_catalog_matches_the_committed_snapshot() {
    let now = rendered();
    if std::env::var("UPDATE_CATALOG").is_ok() {
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/catalog_snapshot.json"),
            &now,
        )
        .expect("writable");
        return;
    }
    if now != SNAPSHOT {
        let a: Vec<&str> = SNAPSHOT.lines().collect();
        let b: Vec<&str> = now.lines().collect();
        let i = (0..a.len().max(b.len()))
            .find(|i| a.get(*i) != b.get(*i))
            .unwrap_or(0);
        panic!(
            "the catalog changed at line {}:\n  committed: {:?}\n  now:       {:?}\n\n\
             If intended: UPDATE_CATALOG=1 cargo test -p ggraph-core --test catalog",
            i + 1,
            a.get(i),
            b.get(i)
        );
    }
}

/// Every registered kind must resolve from its own slug, and from every alias it claims.
///
/// The alias half is the one that matters: an alias exists because a node was renamed, and a
/// rename that does not carry its alias is every stored graph naming the old slug failing to
/// load — at load time, in front of somebody.
#[test]
fn every_kind_resolves_from_its_slug_and_its_aliases() {
    let r = registry();
    for spec in r.all() {
        assert!(
            r.resolve(spec.id.as_str()).is_some(),
            "{} does not resolve from its own slug",
            spec.id
        );
        for a in spec.aliases {
            assert_eq!(
                r.resolve(a).map(|s| s.id.clone()),
                Some(spec.id.clone()),
                "alias {a:?} of {} does not resolve",
                spec.id
            );
        }
    }
}

/// A pure node has no exec pins in the catalog, so an editor knows to hide the circles.
#[test]
fn purity_and_exec_pins_agree() {
    let r = registry();
    for spec in r.all() {
        let cat = spec_json(&r, spec.id.as_str());
        let has_exec_in = cat["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["type"] == "exec");
        assert_eq!(
            has_exec_in,
            spec.purity.has_exec(),
            "{}: catalog and purity disagree about exec pins",
            spec.id
        );
    }
}

fn spec_json(r: &NodeRegistry<TestHost>, slug: &str) -> serde_json::Value {
    r.catalog_json()["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["slug"] == slug)
        .cloned()
        .unwrap_or_else(|| panic!("{slug} is not in the catalog"))
}

/// Two kinds sharing a category is fine; two sharing a slug is not, and the registry refuses it
/// at boot. This asserts the shipped set is actually free of it rather than trusting that the
/// refusal was never triggered.
#[test]
fn the_shipped_set_has_no_duplicate_slugs() {
    let r = registry();
    let mut slugs: Vec<&str> = r.all().map(|s| s.id.as_str()).collect();
    let n = slugs.len();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), n);
}

/// No port in the standard set says `any`, except the one node whose job is to look at whatever is
/// on a wire.
///
/// An `any` port is one the editor cannot refuse, and every refusal it cannot make is a wire
/// somebody draws and a run that goes wrong somewhere else. This is the guard, because the way that
/// port comes back is not a decision anybody takes — it is the shortest thing to type when a node
/// accepts two types and there is no family for it yet.
#[test]
fn nothing_but_the_observer_accepts_anything() {
    let reg = registry();
    let mut offenders: Vec<String> = Vec::new();
    for spec in reg.palette() {
        if spec.id.as_str() == "print" {
            continue;
        }
        let cfg = (spec.default_config)();
        for (side, ports) in [
            ("input", spec.inputs.resolve(&cfg)),
            ("output", spec.outputs.resolve(&cfg)),
        ] {
            for p in ports {
                if p.ty.as_str() == "any" {
                    offenders.push(format!("{}.{} ({side})", spec.id.as_str(), p.name.as_str()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these ports accept anything: {offenders:?}\n\
         Give the port its type, or add it to a family in `port.rs` — never `any`."
    );
}
