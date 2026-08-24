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
```

## Status

Early. The topology, port, value and host layers are in place with tests; the registry,
scheduler and standard node set are landing next.

## Licence

MIT or Apache-2.0, at your option.
