# ggraph

A graph execution engine with no domain.

A graph is nodes with typed ports, wired by edges the engine owns. What the nodes *are* is not
this engine's business: a product registers its own, and the same scheduler runs a camera
pipeline and a document workflow without knowing which it has.

```rust
use ggraph_core::{Graph, NodeId};

let mut g: Graph = Graph::new("hello");
let fmt = g.add_node(NodeId::new_static("format"), 0, 0);
let out = g.add_node(NodeId::new_static("print"), 200, 0);
g.add_edge(registry, fmt, "exec_out", out, "exec_in")?;
```

## Why it exists

Two products need the same engine and share nothing else: **OlharAI** (camera monitoring —
continuous dataflow, millisecond nodes, ephemeral state) and **Redoma** (local AI platform —
task orchestration, durable runs, human approval steps). The engine was extracted from the
first while the second was being designed, on the theory that an engine extracted with one
consumer comes out shaped like that consumer.

## The rules it keeps

**The core runs without a single domain node.** Branch, loop, compare, log, time gate,
variables, tables, approval — the standard set ships *in* the engine, not beside it. An engine
that cannot branch or wait is a topology library.

**One node per file**, carrying its own spec, implementation and tests. Adding a node is adding
a file and one `register` call. In the codebase this was extracted from, it meant touching
fourteen match arms across two files, and forgetting one of them was silent.

**No I/O.** Four dependencies, none async, none a client of anything. Everything that touches
the world — state, blobs, HTTP, a model, a person to approve something, the clock — arrives
through the `Host` traits, implemented by the product. That is what lets nodes with durable
state live in the core without dragging a database in with them.

**No domain vocabulary.** Not `camera`, not `pdf`, not `chunk`. CI greps for it, because a
shared boundary erodes one convenient import at a time and nobody notices until it is a
refactor in two codebases at once.

**Open identifiers.** `NodeId` and `PortType` are strings resolved through a registry, not
closed enums, and they serialize transparently. A product adds a node kind, a port type and a
value type without this crate changing.

## Layout

```
ggraph-core/src/
  id.rs        NodeId, PortName — open, string-serialized
  port.rs      PortType, Port, and the one compatibility rule
  value.rs     Value, Num, Bytes, and Extern — how a product carries its own types
  graph.rs     Graph<M>, GraphNode, Edge, and every wiring rule
  topo.rs      topological order, back edges, entry nodes
  host.rs      the seam: StateStore, Observer, ValueIo, Approvals, Http, Llm, TableStore
  spec.rs      NodeSpec — one declaration per node, and the three behaviours
  registry.rs  where a product's nodes meet the engine
  exec.rs      the scheduler: epochs, dead branches, pull-not-push
  nodes/       the standard set, one file each
```

## The standard set

Fifteen nodes so far, and they exist to prove the design as much as to be useful:

| | |
|---|---|
| Control | `if` · `for_each` · `wait` · `cooldown` · `debounce` |
| Approval | `approval` |
| Logic / Text | `compare` · `format` |
| Variables | `get_variable` · `set_variable` |
| Network | `http_request` |
| AI | `ask_llm` · `llm_decide` · `llm_extract` |
| Debug | `print` |

`wait`, `approval` and `debounce` are the ones that matter for the architecture: none of them
sleeps, all of them keep durable state, and none of them imports a database or a runtime. If the
`Host` traits could not carry those three, the seam would be wrong.

A theme runs through several of them. `if` has a third arm, `llm_decide` has a third arm,
`approval` has a third arm, `compare` produces no answer rather than a wrong one, and
`get_variable` reads as absent rather than as zero. **Not knowing is an ordinary outcome**, and a
graph that cannot see the difference between "no" and "could not tell" quietly does the wrong
thing to whichever one it collapsed.

## Status

Early, and honest about it. Topology, ports, values, the host seam, the registry, the scheduler
and fifteen nodes are in place with 92 tests. Not there yet: durable run checkpointing (resume a
crashed run from the node it died on), the table nodes, and the persistence adapter.

## Licence

MIT or Apache-2.0, at your option.
