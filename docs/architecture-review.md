# What is incoherent here, and what it will cost to leave

An audit at 4,100 lines and two consumers, written while both were still small enough to change.
Ordered by **cost of delay** rather than by severity: a thing that gets harder every week a
consumer is added is worth more attention than a thing that is merely wrong.

Everything below is measured against the code, not impressions. The counts are from `grep`.

---

## 1. `Host` is two traits wearing one coat

**The scheduler uses four of the nine capabilities.** Counted against `exec.rs`:

| | calls from the scheduler |
|---|---|
| `observer()` | 9 |
| `state()` | 3 |
| `io()` | 2 |
| `literals()` | 1 |
| `approvals()` `http()` `llm()` `tables()` `vars()` | **0** |

Those last five are used by the **bundled node library** and by nothing else. `vars()` is reached
from exactly one file, `nodes/variables.rs`. `tables()` from six, all of them `nodes/table/*`.

So every product that embeds this engine must implement five capabilities it may never use.
That is not hypothetical: the first real consumer implements all four service ones as a struct
called `NotRouted` whose every method returns "this reaches the product's own node directly".
A stub that large, written on day one, is the design telling you something.

**The coherent split** is by who asks, not by what it does:

- `Host` — what the *scheduler* needs to run any graph: state, blobs, observation, literals,
  instance key, schedule, run id, clock.
- The standard nodes' needs belong to the standard nodes. Either a `StdHost: Host` supertrait
  that only a product registering them implements, or — better — the capability is handed to
  the node at registration (`nodes::register_all(&mut reg, llm, http, tables)`), which also
  makes "I want the table nodes but not the AI ones" expressible.

**Cost of delay:** every consumer's `Host` impl breaks when this is fixed. Two consumers today.

---

## 2. `vars()` is the only capability that is not a trait

```rust
fn vars(&self) -> &Mutex<HashMap<String, Value>>;
```

Everything else returns `&dyn Something`. This returns a concrete container, which means the
engine dictates how a product stores run variables — the one piece of state where it does.
It is also keyed by `String` while every other name in the crate is a `PortName`, so a product
holding variables keyed the same way as its ports has to convert at the boundary.

And it is only used by the bundled variable nodes, so it is really an instance of §1.

**Cost of delay:** low on its own, but it is the crack that makes §1 look acceptable.

---

## 3. `Step` can say two contradictory things at once

```rust
pub struct Step {
    pub reenter: bool,   // run me again next epoch
    pub halt: bool,      // end the run here
    ...
}
```

Nothing stops both being `true`, and there is no sensible reading of it. Today no node does it;
the twentieth node someone writes will, and the scheduler will pick one silently.

This wants to be an enum — `Continue` / `Reenter` / `Halt` — which also documents the three
outcomes in the type instead of in a comment.

**Cost of delay:** every `Step` construction changes. There are eight today.

---

## 4. A graph cannot be checked without running it

There is no `validate(&graph, &registry)`. A stored graph naming a kind this build does not
register fails as `RunError::UnknownKind` — **at run time**, on somebody's automation, at 3am if
that is when the trigger fires.

The information to answer it exists: the registry knows every kind, `Graph::add_edge` already
validates a wire. What is missing is the sweep over an existing document, which is exactly what
an editor wants on load, a deploy wants on boot, and a migration wants before it commits.

It should return every problem, not the first: "this graph has 3 unknown kinds and 1 wire whose
port no longer exists" is actionable, and "unknown kind at node 7" is a bisect.

**Cost of delay:** none — it is purely additive. It is on this list because it is the cheapest
real improvement available and it converts a class of run-time failure into a load-time one.

---

## 5. `Purity` bundles two properties that are not the same question

`Effectful` / `Pure` / `PureSource` answers *"does it have exec pins?"* and *"is it re-read on
every access?"* with one value, as if the second only applies when the first is "no".

The first consumer needed a third property under a colliding name: its `is_pure_source` means
*"run this even as an orphan entry inside a sub-run"* — a seeding rule from a real incident —
while ours means *"re-evaluate on every read"* — a caching rule. Several of its kinds are the
first without being the second, and deriving purity from the wrong one silently stripped a
node's exec pin out of the published catalog. The catalog snapshot caught it; review had not.

Two fields would not have collided:

```rust
pub struct Purity { pub has_exec: bool, pub reevaluates: bool }
```

**Cost of delay:** moderate. Every `NodeSpec` builder call that sets purity, and every consumer
that reads it.

---

## 6. `RunError` classifies retryability on one variant out of three

`Node { .., retry }` carries the judgement. `UnknownKind` and `Budget` do not, so a durable host
deciding whether to back off has to match on the variant and know that two of them are permanent.
It is the host's own inference about the engine's internals — which is the thing `retry` was
added to stop.

**Cost of delay:** low, and it is a five-line fix.

---

## What is NOT wrong, and is worth saying

- **One scheduler with a checkpoint policy.** The two-schedulers design the extraction started
  from would have been two copies of the epoch loop, the dead-branch rule and the pull semantics,
  diverging quietly. The policy enum is right.
- **Open string identifiers.** `NodeId` and `PortType` as `#[serde(transparent)]` newtypes are
  what made the first adoption a refactor rather than a migration.
- **`Extern` with a codec that may refuse.** Letting a value say "do not persist me", and
  letting the codec drop it rather than fail the row, is the correct shape and was arrived at by
  hitting the alternative.
- **The three declared-but-not-done features are done.** Timeouts are enforced, elapsed is
  measured, and `memoize` was deleted rather than implemented once it turned out the distinction
  it drew was already made correctly by how a node is reached.

---

## Recommended order

1. **§4 graph validation** — additive, cheap, converts run-time failures into load-time ones.
2. **§3 `Step` enum** and **§6 `RunError`** — small, mechanical, and both get worse with node count.
3. **§1 + §2 the `Host` split** — the expensive one, and the one that gets more expensive with
   every consumer. Two today. This is the moment.
4. **§5 `Purity`** — after §1, since both touch every `NodeSpec`.
