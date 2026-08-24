//! Graph ordering: topological sort, and finding the edges that close a loop.
//!
//! Pure functions over the document — no host, no values, no execution. A scheduler decides
//! *when* to run a node; this decides what "before" means.
//!
//! The distinction that matters is the back edge. A graph with loops has no topological order,
//! but a graph whose *only* cycles run through exec edges does: exclude those, order the rest,
//! and the excluded edges become re-entry points. That is what makes a loop a loop rather than
//! an error.

use crate::graph::{Graph, GraphMeta, PortLookup};
use std::collections::{HashMap, HashSet};

/// The edges whose removal makes the graph acyclic — the ones that close a loop.
///
/// Found by a colour-marking depth-first search: an edge into a node that is currently on the
/// search stack (grey) is a back edge. Deterministic in node order, so the same document always
/// yields the same set.
pub fn back_edges<M: GraphMeta>(g: &Graph<M>) -> HashSet<(u32, u32)> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        White,
        Grey,
        Black,
    }
    let mut mark: HashMap<u32, Mark> = g.nodes.iter().map(|n| (n.id, Mark::White)).collect();
    let mut back = HashSet::new();

    // Iterative, not recursive: a generated graph can be deep enough to blow the stack, and a
    // stack overflow in a scheduler is an unattributable crash.
    let mut stack: Vec<(u32, usize)> = Vec::new();
    let mut ids: Vec<u32> = g.nodes.iter().map(|n| n.id).collect();
    ids.sort_unstable();

    for root in ids {
        if mark.get(&root) != Some(&Mark::White) {
            continue;
        }
        stack.push((root, 0));
        mark.insert(root, Mark::Grey);
        while let Some((node, idx)) = stack.pop() {
            let kids = g.children(node);
            if idx < kids.len() {
                stack.push((node, idx + 1));
                let next = kids[idx];
                match mark.get(&next).copied().unwrap_or(Mark::White) {
                    Mark::Grey => {
                        back.insert((node, next));
                    }
                    Mark::White => {
                        mark.insert(next, Mark::Grey);
                        stack.push((next, 0));
                    }
                    Mark::Black => {}
                }
            } else {
                mark.insert(node, Mark::Black);
            }
        }
    }
    back
}

/// Node ids in dependency order, ignoring the edges that close loops.
///
/// `None` if a cycle remains after back edges are excluded, which should be impossible and is
/// therefore worth reporting rather than papering over.
pub fn topo_order<M: GraphMeta>(g: &Graph<M>) -> Option<Vec<u32>> {
    let back = back_edges(g);
    let mut indeg: HashMap<u32, usize> = g.nodes.iter().map(|n| (n.id, 0)).collect();
    let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();

    for (from, to) in ordering_pairs(g, &back) {
        adj.entry(from).or_default().push(to);
        *indeg.entry(to).or_insert(0) += 1;
    }

    // Lowest id first among ready nodes: a stable order makes a run log diffable.
    let mut ready: Vec<u32> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    ready.sort_unstable();

    let mut out = Vec::with_capacity(g.nodes.len());
    while let Some(n) = ready.first().copied() {
        ready.remove(0);
        out.push(n);
        for m in adj.get(&n).cloned().unwrap_or_default() {
            let d = indeg.get_mut(&m).expect("edge target exists");
            *d -= 1;
            if *d == 0 {
                ready.push(m);
                ready.sort_unstable();
            }
        }
    }
    (out.len() == g.nodes.len()).then_some(out)
}

/// The distinct `(from, to)` pairs that constrain ordering: every edge except the back edges,
/// deduplicated.
///
/// Deduplicated because two nodes are frequently joined by several edges at once — an exec pin
/// and two data pins — and counting that as three dependencies makes the indegree wrong.
pub fn ordering_pairs<M: GraphMeta>(g: &Graph<M>, back: &HashSet<(u32, u32)>) -> Vec<(u32, u32)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in &g.edges {
        let pair = (e.from, e.to);
        if back.contains(&pair) {
            continue;
        }
        if seen.insert(pair) {
            out.push(pair);
        }
    }
    out
}

/// Nodes with no incoming exec edge — where a run starts.
///
/// Pure nodes are never entries: they are pulled by whoever reads them, not pushed.
pub fn entry_nodes<M: GraphMeta>(g: &Graph<M>, reg: &dyn PortLookup) -> Vec<u32> {
    let mut out: Vec<u32> = g
        .nodes
        .iter()
        .filter(|n| reg.has_exec_in(&n.kind))
        .filter(|n| {
            !g.edges
                .iter()
                .any(|e| e.to == n.id && e.to_port == crate::port::EXEC_IN.name)
        })
        .map(|n| n.id)
        .collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testkit::Fake;
    use crate::id::NodeId;

    fn chain(n: u32) -> Graph {
        let mut g: Graph = Graph::new("t");
        for _ in 0..n {
            g.add_node(NodeId::new_static("k"), 0, 0);
        }
        for i in 1..n {
            g.add_edge(&Fake, i, "exec_out", i + 1, "exec_in").unwrap();
        }
        g
    }

    #[test]
    fn a_chain_orders_in_sequence() {
        let g = chain(4);
        assert_eq!(topo_order(&g), Some(vec![1, 2, 3, 4]));
        assert!(back_edges(&g).is_empty());
    }

    #[test]
    fn a_loop_yields_one_back_edge_and_still_orders() {
        let mut g = chain(3);
        g.add_edge(&Fake, 3, "exec_out", 1, "exec_in").unwrap();
        let back = back_edges(&g);
        assert_eq!(
            back,
            [(3, 1)].into_iter().collect::<HashSet<_>>(),
            "the edge that closes the loop is the one that must be excluded"
        );
        assert_eq!(
            topo_order(&g),
            Some(vec![1, 2, 3]),
            "excluding the back edge leaves an orderable graph — that is what makes it a loop \
             rather than an error"
        );
    }

    #[test]
    fn parallel_edges_between_two_nodes_count_once() {
        let mut g: Graph = Graph::new("t");
        for _ in 0..2 {
            g.add_node(NodeId::new_static("k"), 0, 0);
        }
        g.add_edge(&Fake, 1, "exec_out", 2, "exec_in").unwrap();
        g.add_edge(&Fake, 1, "value", 2, "a").unwrap();
        g.add_edge(&Fake, 1, "text", 2, "text").unwrap();
        assert_eq!(
            ordering_pairs(&g, &back_edges(&g)),
            vec![(1, 2)],
            "three edges are one dependency; counting them thrice breaks the indegree"
        );
        assert_eq!(topo_order(&g), Some(vec![1, 2]));
    }

    #[test]
    fn ordering_is_stable_across_runs() {
        let mut g: Graph = Graph::new("t");
        for _ in 0..5 {
            g.add_node(NodeId::new_static("k"), 0, 0);
        }
        g.add_edge(&Fake, 5, "exec_out", 3, "exec_in").unwrap();
        let first = topo_order(&g);
        for _ in 0..20 {
            assert_eq!(
                topo_order(&g),
                first,
                "an unstable order makes a run log undiffable"
            );
        }
    }

    #[test]
    fn entries_are_the_nodes_nothing_hands_control_to() {
        let mut g = chain(3);
        g.add_node(NodeId::new_static("k"), 0, 0);
        assert_eq!(
            entry_nodes(&g, &Fake),
            vec![1, 4],
            "an orphan is an entry: it has no incoming exec edge either"
        );
    }
}
