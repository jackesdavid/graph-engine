//! **ggraph** — a graph execution engine with no domain.
//!
//! A graph is nodes with typed ports, wired by edges the engine owns. What the nodes *are* is
//! not this crate's business: a product registers its own against [`Host`], and the same
//! scheduler runs a camera pipeline and a document workflow without knowing which it has.
//!
//! Two properties are load-bearing, and both are enforced rather than promised:
//!
//! - **No domain vocabulary in here.** Not `camera`, not `pdf`, not `chunk`. CI greps for it.
//! - **No I/O in here.** Four dependencies, none of them async, none of them a client of
//!   anything. Everything that touches the world arrives through [`Host`].
//!
//! ```
//! use ggraph_core::{Graph, NodeId};
//!
//! let mut g: Graph = Graph::new("hello");
//! let a = g.add_node(NodeId::new_static("format"), 0, 0);
//! let b = g.add_node(NodeId::new_static("print"), 200, 0);
//! assert_eq!(g.nodes.len(), 2);
//! assert_eq!((a, b), (1, 2));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod codec;
pub mod exec;
pub mod graph;
pub mod host;
pub mod id;
pub mod nodes;
pub mod port;
pub mod registry;
pub mod spec;
pub mod topo;
pub mod value;

pub use codec::{decode, decode_ports, encode, encode_ports};
pub use exec::{run, Budget, Checkpoint, Entry, Outputs, RunError, RunOptions};
pub use graph::{Edge, Graph, GraphMeta, GraphNode, PortLookup, WireError};
pub use host::{
    ApprovalRequest, Approvals, Host, HostError, Http, HttpRequest, HttpResponse, Llm, LlmRequest,
    NodeTarget, Observer, Retry, RunKey, Slot, StateKey, StateStore, TableStore, UiEvent, ValueIo,
    Verdict,
};
pub use id::{NodeId, PortName};
pub use port::{compatible, Port, PortType, EXEC_IN, EXEC_OUT};
pub use registry::{NodeRegistry, RegistryError};
/// Re-exported because it appears in this crate's public API (`Host::instance_key`,
/// `NodeTarget::instance`). A consumer must not have to guess which version to depend on, nor
/// end up with two that do not unify.
pub use smol_str::SmolStr;
pub use spec::{
    Behavior, ConfigFn, ExecOut, NodeCx, NodeError, NodeRoute, NodeRun, NodeSpec, NodeStep, Ports,
    PortsFn, Purity, Step, StepCx, Timeout,
};
pub use topo::{back_edges, entry_nodes, ordering_pairs, topo_order};
pub use value::{Bytes, ExternValue, Num, PortValues, Value};
