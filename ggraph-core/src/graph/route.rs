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

/// What a kind gives, once what is ARRIVING has been taken into account.
///
/// A node whose ports depend on its settings does not know them yet: `For Each` publishes an `item`
/// of `text` until something tells it what the list holds. Asked at its default, the router saw a
/// loop that emits text and could not tell that looping over passages emits a passage — so
/// `chunk_search → for_each → break_chunk_result`, the obvious chain, was reported as impossible.
///
/// The answer is the same [`bake`](super::bake) a saved document gets, applied to a search: put the
/// arriving port in front of the kind, let it work out its own configuration, then ask.
pub fn gives<H: Host>(
    reg: &NodeRegistry<H>,
    kind: &NodeId,
    arriving: Option<(&crate::id::PortName, &crate::port::Port)>,
) -> Vec<crate::port::Port> {
    let Some(spec) = reg.get(kind) else {
        return Vec::new();
    };
    let mut cfg = (spec.default_config)();
    if let (Some(bake), Some((on, port))) = (spec.bake.as_ref(), arriving) {
        let wired = super::bake::Wired::from(vec![(on.clone(), port.clone())]);
        if let Some(next) = bake(&cfg, &wired) {
            cfg = next;
        }
    }
    spec.outputs.resolve(&cfg)
}

/// What a kind takes, at its default. Inputs do not depend on what is arriving — the port that
/// accepts a value is the one that has to exist before anything can arrive on it.
pub fn takes<H: Host>(reg: &NodeRegistry<H>, kind: &NodeId) -> Vec<crate::port::Port> {
    reg.get(kind)
        .map(|s| s.inputs.resolve(&(s.default_config)()))
        .unwrap_or_default()
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

/// Every type, scored once. Walking the palette per edge considered turned a search into a scan of
/// the whole registry a few thousand times over, and the search stopped finishing.
fn scores<H: Host>(reg: &NodeRegistry<H>) -> HashMap<PortType, u32> {
    let mut makers: HashMap<PortType, u32> = HashMap::new();
    for s in reg.palette() {
        let mut seen: Vec<PortType> = Vec::new();
        for p in s.outputs.resolve(&(s.default_config)()) {
            if !seen.contains(&p.ty) {
                seen.push(p.ty.clone());
                *makers.entry(p.ty).or_insert(0) += 1;
            }
        }
    }
    makers
        .into_iter()
        .map(|(ty, n)| (ty, u32::MAX - n))
        .collect()
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
    let kinds: Vec<NodeId> = reg.palette().map(|s| s.id.clone()).collect();

    /// A path, and what reached its last node — which is what that node needs in order to say
    /// what it gives. Edges cannot be computed once and reused: `For Each` following a table is a
    /// different node from `For Each` following a search.
    struct Step {
        path: Vec<NodeId>,
        arriving: Option<(crate::id::PortName, crate::port::Port)>,
        /// One score per link, in order. Kept whole because ranking needs more than the minimum.
        scores: Vec<u32>,
    }

    // BEST-first on the weakest link, not breadth-first. Breadth returns the shortest routes, and
    // the shortest are the ones held together by the vaguest types: `chunk_search → send_email` in
    // one hop, through an error message reaching an address. Expanding by weakest link means the
    // first answers out are the most meaningful.
    // The key is every link score, WEAKEST FIRST, padded so a shorter path's missing links read as
    // unbreakable. Ranking on the minimum alone left two paths with the same bottleneck
    // indistinguishable — `table_read → get_table_rows → for_each` and `table_read → pick_numbers
    // → for_each` share their weakest link and differ entirely in the one after it. Comparing the
    // whole sorted list says the second thing too, and the padding keeps a short strong path ahead
    // of a long one that merely matches it.
    let key = |scores: &[u32]| -> Vec<u32> {
        let mut v = scores.to_vec();
        v.sort_unstable();
        v.resize(LONGEST, u32::MAX);
        v
    };
    let mut heap: std::collections::BinaryHeap<(Vec<u32>, std::cmp::Reverse<usize>, usize)> =
        std::collections::BinaryHeap::new();
    let mut steps: Vec<Step> = vec![Step {
        path: vec![from.clone()],
        arriving: None,
        scores: Vec::new(),
    }];
    heap.push((key(&[]), std::cmp::Reverse(1), 0));

    let score = scores(reg);

    // How many times a STATE — a kind, plus the type that reached it — may be expanded. A pure
    // visited-set would find one route and no alternatives; unbounded revisiting is what made the
    // search stop finishing once paths began carrying what arrived, because the same kind reached
    // by a dozen routes became a dozen states with the same answer.
    const REVISITS: usize = 2;
    let mut seen: HashMap<(NodeId, PortType), usize> = HashMap::new();

    let mut found: Vec<Vec<NodeId>> = Vec::new();
    while let Some((_, _, at)) = heap.pop() {
        let last = steps[at].path.last().expect("a path has a head").clone();
        if last != *to {
            let state = (
                last.clone(),
                steps[at]
                    .arriving
                    .as_ref()
                    .map(|(_, p)| p.ty.clone())
                    .unwrap_or(PortType::EXEC),
            );
            let visits = seen.entry(state).or_insert(0);
            if *visits >= REVISITS {
                continue;
            }
            *visits += 1;
        }

        // Recorded when POPPED, not when reached. Recording on arrival puts routes in the order
        // the walk stumbled over them, which is not the order of their weakest link — and the
        // ranking then does nothing.
        if last == *to {
            found.push(steps[at].path.clone());
            if found.len() >= limit {
                break;
            }
            continue;
        }
        if steps[at].path.len() > LONGEST {
            continue;
        }

        let arriving = steps[at].arriving.as_ref().map(|(n, p)| (n, p));
        let outs = gives(reg, &last, arriving);
        for b in &kinds {
            // A kind appears once. A chain that visits one twice is a longer way to say the same
            // thing, and it is how a search runs forever on a set that loops.
            if steps[at].path.contains(b) {
                continue;
            }
            let ins = takes(reg, b);
            let mut best: Option<(u32, crate::id::PortName, crate::port::Port)> = None;
            for o in outs.iter().filter(|p| p.ty != PortType::EXEC) {
                for i in ins.iter().filter(|p| p.ty != PortType::EXEC) {
                    if !compatible(o, i) {
                        continue;
                    }
                    let sc = score.get(&o.ty).copied().unwrap_or(0);
                    if best.as_ref().is_none_or(|(b, _, _)| sc > *b) {
                        best = Some((sc, i.name.clone(), o.clone()));
                    }
                }
            }
            let Some((sc, on, port)) = best else { continue };
            let mut path = steps[at].path.clone();
            path.push(b.clone());
            let mut scores = steps[at].scores.clone();
            scores.push(sc);
            let k = key(&scores);
            let len = path.len();
            steps.push(Step {
                path,
                arriving: Some((on, port)),
                scores,
            });
            heap.push((k, std::cmp::Reverse(len), steps.len() - 1));
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

    /// The case that made the search carry what is arriving. `For Each` publishes an `item` of
    /// `text` until something tells it what the list holds, so asked at its default the router saw
    /// a loop that emits text — and the most obvious chain in the product was reported impossible.
    #[test]
    fn a_loop_gives_what_it_was_given() {
        let r = reg();
        let plain = gives(&r, &id("for_each"), None);
        let item = |ps: &[crate::port::Port]| {
            ps.iter()
                .find(|p| p.name.as_str() == "item")
                .unwrap()
                .ty
                .clone()
        };
        assert_eq!(item(&plain), PortType::TEXT, "nothing has told it yet");

        let rows = crate::port::Port::opt("rows", PortType::TABLE_ROWS);
        let told = gives(
            &r,
            &id("for_each"),
            Some((&crate::id::PortName::new("items"), &rows)),
        );
        assert_eq!(
            item(&told),
            PortType::TABLE_ROW,
            "a row, because rows arrived"
        );
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
