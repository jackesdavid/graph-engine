// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Which kinds can follow which, and what chains connect two of them.
//!
//! The first mile of building a graph is choosing the nodes and their order, and it is not a
//! generation problem — it is a SEARCH problem over a graph the registry already describes. Kind A
//! can be followed by kind B when something A outputs is something B accepts. That relation is a
//! fact, computable, and the same every time; asking a model to rediscover it produces a chain that
//! is plausible and, often enough, impossible.
//!
//! So: [`next`] answers "what may come after this", and [`route`] answers "what gets me from here
//! to there". Both are exhaustive and both are correct by construction. What is left for judgement
//! is the part that needs it — WHICH source, WHICH destination, and which of several valid chains
//! says what the person asked for.
//!
//! # Ports are not the subject here
//!
//! Deliberately. This answers in kinds, and where the wire lands is a separate question with its
//! own answer. Conflating them is what made the first mile impossible to get right: a chain that
//! was correct got thrown away because one port name in it was wrong.

use crate::host::Host;
use crate::id::NodeId;
use crate::port::{compatible, PortType};
use crate::registry::NodeRegistry;
use std::collections::{HashMap, HashSet, VecDeque};

/// How long a chain may get before the search gives up.
///
/// Six is past anything a person would draw by hand between two named endpoints. Without a ceiling
/// a set of kinds that can feed each other in a cycle — a loop, a round-trip through a table —
/// makes the search unbounded.
const LONGEST: usize = 6;

/// May a value leave `from` and arrive at `to`?
///
/// Against each kind's DEFAULT configuration, which is the state a node is in when somebody is
/// deciding whether to place it. A node whose ports depend on its settings can gain more once
/// configured; it never loses the ones it started with.
pub fn connects<H: Host>(reg: &NodeRegistry<H>, from: &NodeId, to: &NodeId) -> bool {
    strength(reg, from, to).is_some()
}

/// How MUCH two kinds connect, or `None` if they do not.
///
/// Connectivity alone turned out to be nearly meaningless. Half a catalogue has a `text` port, so
/// almost everything can follow almost everything: `chunk_search → send_email` is a legal chain via
/// the search's `error_message` reaching the mailer's `to`. Type-correct, and nonsense.
///
/// So a link is scored by how SPECIFIC the type it travels on is — how few kinds in the whole set
/// produce one. A `chunk_results` has one producer and means something; a `text` has thirty and
/// means almost nothing. The strongest shared type wins, because two kinds joined by both a
/// `table` and a `text` are joined by the table.
pub fn strength<H: Host>(reg: &NodeRegistry<H>, from: &NodeId, to: &NodeId) -> Option<u32> {
    let (a, b) = (reg.get(from)?, reg.get(to)?);
    let outs = a.outputs.resolve(&(a.default_config)());
    let ins = b.inputs.resolve(&(b.default_config)());
    let mut best: Option<u32> = None;
    for o in outs.iter().filter(|p| p.ty != PortType::EXEC) {
        let fits = ins
            .iter()
            .filter(|p| p.ty != PortType::EXEC)
            .any(|i| compatible(o, i));
        if fits {
            let s = specificity(reg, &o.ty);
            best = Some(best.map_or(s, |b| b.max(s)));
        }
    }
    best
}

/// How much a type narrows things down: the fewer kinds produce one, the more a wire carrying it
/// says. Inverted into a score so that bigger is better and the arithmetic reads the right way.
fn specificity<H: Host>(reg: &NodeRegistry<H>, ty: &PortType) -> u32 {
    let makers = reg
        .palette()
        .filter(|s| {
            s.outputs
                .resolve(&(s.default_config)())
                .iter()
                .any(|p| p.ty == *ty)
        })
        .count() as u32;
    u32::MAX - makers
}

/// Every kind that may follow this one, in palette order.
pub fn next<H: Host>(reg: &NodeRegistry<H>, from: &NodeId) -> Vec<NodeId> {
    reg.palette()
        .filter(|s| connects(reg, from, &s.id))
        .map(|s| s.id.clone())
        .collect()
}

/// Every kind that may precede this one.
pub fn previous<H: Host>(reg: &NodeRegistry<H>, to: &NodeId) -> Vec<NodeId> {
    reg.palette()
        .filter(|s| connects(reg, &s.id, to))
        .map(|s| s.id.clone())
        .collect()
}

/// The chains of kinds that get from `from` to `to`, shortest first.
///
/// Breadth-first, so the first answers are the fewest nodes — which is what somebody asking this
/// question wants, and what a longer chain has to earn. Both endpoints are included.
///
/// `limit` caps how many are returned. All of them is rarely useful and frequently enormous: with
/// fifty kinds, "text to text" has hundreds of answers and they are all the same idea.
pub fn route<H: Host>(
    reg: &NodeRegistry<H>,
    from: &NodeId,
    to: &NodeId,
    limit: usize,
) -> Vec<Vec<NodeId>> {
    if reg.get(from).is_none() || reg.get(to).is_none() {
        return Vec::new();
    }
    if from == to {
        return vec![vec![from.clone()]];
    }

    // The relation, computed once with its scores. Recomputing it inside the walk turns a search
    // over fifty kinds into fifty times the port resolution it needed.
    let kinds: Vec<NodeId> = reg.palette().map(|s| s.id.clone()).collect();
    let mut edges: HashMap<&NodeId, Vec<(&NodeId, u32)>> = HashMap::new();
    for a in &kinds {
        for b in &kinds {
            if a == b {
                continue;
            }
            if let Some(w) = strength(reg, a, b) {
                edges.entry(a).or_default().push((b, w));
            }
        }
    }

    // BEST-first, not breadth-first. Breadth returns the shortest routes, and the shortest are the
    // ones held together by the vaguest types: `chunk_search → send_email` in one hop, through an
    // error message reaching an address. Expanding by weakest-link instead means the first answers
    // out are the most meaningful, and the long specific chain is found before the budget is spent
    // on dozens of short empty ones.
    //
    // Ordered by (weakest link, then FEWER nodes) — `Reverse` on the length so the heap's max is
    // the shortest of equals.
    let mut heap: std::collections::BinaryHeap<(u32, std::cmp::Reverse<usize>, Vec<&NodeId>)> =
        std::collections::BinaryHeap::new();
    heap.push((u32::MAX, std::cmp::Reverse(1), vec![from]));

    let mut found: Vec<Vec<NodeId>> = Vec::new();
    while let Some((worst, _, path)) = heap.pop() {
        let last = *path.last().expect("a path has a head");

        // A completed route is RECORDED when it is popped, not when it is reached. Recording on
        // arrival puts them in the order the walk stumbled over them, which is not the order of
        // their weakest link — and the whole ranking then did nothing.
        if last == to {
            found.push(path.iter().map(|k| (*k).clone()).collect());
            if found.len() >= limit {
                break;
            }
            continue;
        }
        if path.len() > LONGEST {
            continue;
        }

        for (nxt, w) in edges.get(last).into_iter().flatten() {
            // A kind appears once. A chain that visits one twice is a longer way to say the same
            // thing, and it is how a search runs forever on a set that loops.
            if path.contains(nxt) {
                continue;
            }
            let mut on = path.clone();
            on.push(nxt);
            heap.push((worst.min(*w), std::cmp::Reverse(on.len()), on));
        }
    }
    found
}

/// The kinds that start a chain: they take no data, so nothing needs to come before them.
///
/// Where a graph BEGINS, which is the one end a search cannot work backwards from.
pub fn sources<H: Host>(reg: &NodeRegistry<H>) -> Vec<NodeId> {
    reg.palette()
        .filter(|s| {
            s.inputs
                .resolve(&(s.default_config)())
                .iter()
                .all(|p| p.ty == PortType::EXEC)
        })
        .map(|s| s.id.clone())
        .collect()
}

/// Is this list of kinds a chain — does each one connect to the next?
///
/// Returns the index of the first pair that does not, so the answer names WHERE it breaks rather
/// than only that it does.
pub fn check<H: Host>(reg: &NodeRegistry<H>, plan: &[NodeId]) -> Result<(), Broken> {
    for (i, pair) in plan.windows(2).enumerate() {
        if reg.get(&pair[0]).is_none() {
            return Err(Broken::NoSuchKind {
                at: i,
                kind: pair[0].clone(),
            });
        }
        if reg.get(&pair[1]).is_none() {
            return Err(Broken::NoSuchKind {
                at: i + 1,
                kind: pair[1].clone(),
            });
        }
        if !connects(reg, &pair[0], &pair[1]) {
            return Err(Broken::CannotFollow {
                at: i,
                from: pair[0].clone(),
                to: pair[1].clone(),
            });
        }
    }
    if let [only] = plan {
        if reg.get(only).is_none() {
            return Err(Broken::NoSuchKind {
                at: 0,
                kind: only.clone(),
            });
        }
    }
    Ok(())
}

/// Why a plan is not a chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    NoSuchKind { at: usize, kind: NodeId },
    CannotFollow { at: usize, from: NodeId, to: NodeId },
}

impl std::fmt::Display for Broken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Broken::NoSuchKind { at, kind } => {
                write!(f, "step {}: no kind `{}`", at + 1, kind.as_str())
            }
            Broken::CannotFollow { at, from, to } => write!(
                f,
                "step {} to {}: nothing `{}` gives is anything `{}` takes",
                at + 1,
                at + 2,
                from.as_str(),
                to.as_str()
            ),
        }
    }
}

/// The kinds reachable from here at all, however long the chain.
pub fn reachable<H: Host>(reg: &NodeRegistry<H>, from: &NodeId) -> HashSet<NodeId> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([from.clone()]);
    while let Some(k) = queue.pop_front() {
        for n in next(reg, &k) {
            if seen.insert(n.clone()) {
                queue.push_back(n);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::nodes::services::Services;

    fn reg() -> NodeRegistry<TestHost> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r, &Services::none());
        r
    }

    fn id(s: &str) -> NodeId {
        NodeId::new(s)
    }

    /// The relation the whole thing rests on: A may be followed by B when something A gives is
    /// something B takes. A fact about the registry, not an opinion about the graph.
    #[test]
    fn one_kind_follows_another_when_a_type_matches() {
        let r = reg();
        assert!(
            connects(&r, &id("format"), &id("print")),
            "text into a message"
        );
        assert!(
            !connects(&r, &id("print"), &id("format")),
            "print gives nothing, so nothing can follow it"
        );
    }

    /// The question a builder asks, answered by search rather than by guessing.
    #[test]
    fn a_route_is_found_between_two_kinds() {
        let r = reg();
        let paths = route(&r, &id("format"), &id("output"), 4);
        assert!(!paths.is_empty(), "text reaches an Output");
        assert_eq!(paths[0].first(), Some(&id("format")));
        assert_eq!(paths[0].last(), Some(&id("output")));
    }

    /// Shortest first, because that is what somebody asking wants and what a longer chain must
    /// earn against.
    #[test]
    fn the_shortest_chain_comes_first() {
        let r = reg();
        let paths = route(&r, &id("format"), &id("output"), 5);
        for pair in paths.windows(2) {
            assert!(pair[0].len() <= pair[1].len(), "{paths:?}");
        }
    }

    /// Two kinds with nothing between them say so, rather than returning a chain that cannot run.
    #[test]
    fn unreachable_is_an_empty_answer_not_a_wrong_one() {
        let r = reg();
        assert!(route(&r, &id("print"), &id("output"), 4).is_empty());
    }

    /// A plan is checked as a chain and the answer names WHERE it breaks — the first mile's whole
    /// purpose is to be correctable, and "wrong" is not correctable.
    #[test]
    fn a_broken_plan_names_the_step_that_breaks_it() {
        let r = reg();
        let ok = [id("format"), id("print")];
        assert_eq!(check(&r, &ok), Ok(()));

        let bad = [id("print"), id("format")];
        let Err(Broken::CannotFollow { at, .. }) = check(&r, &bad) else {
            panic!("that is not a chain")
        };
        assert_eq!(at, 0);

        let unknown = [id("format"), id("teleport")];
        assert!(matches!(
            check(&r, &unknown),
            Err(Broken::NoSuchKind { at: 1, .. })
        ));
    }

    /// Where a graph begins: the kinds nothing has to come before.
    #[test]
    fn a_source_is_a_kind_that_needs_nothing() {
        let r = reg();
        let s = sources(&r);
        assert!(
            s.contains(&id("table_schema")),
            "a schema declares itself: {s:?}"
        );
        assert!(!s.contains(&id("print")), "print needs something to print");
    }
}
