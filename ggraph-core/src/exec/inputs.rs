// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a node's inputs come from.
//!
//! In order: a wire, if one is connected; otherwise what the user typed into the node's own form,
//! read through the host's [`Literals`](crate::host::Literals) capability. A port with neither is
//! absent, and a node that requires it refuses rather than inventing a zero.
//!
//! [`pull`] is the part that surprises people. A pure source behind an input is evaluated ON
//! DEMAND, right here, rather than waiting its turn — which is what lets a variable read feed a
//! node that runs before it in topological order. It is also why such a node activates on every
//! epoch, including on branches nobody took, and why reporting it is deferred until something
//! actually reads it.
use super::*;
use crate::graph::{Graph, GraphMeta};
use crate::host::Host;
use crate::id::PortName;
use crate::registry::NodeRegistry;
use crate::spec::{Behavior, NodeCx, Purity};
use crate::value::{PortValues, Value};

/// Collect a node's inputs, evaluating pure sources behind them on demand.
pub(crate) fn gather<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    nid: u32,
    st: &mut State,
) -> Result<PortValues, RunError> {
    let mut inputs = PortValues::new();
    let wires: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.to == nid && e.to_port != crate::port::EXEC_IN.name)
        .cloned()
        .collect();

    for w in wires {
        // A wire from a node that never ran carries nothing. That is not an error: it is how a
        // dead branch's absence reaches the far side of a join.
        if !st.ran.contains(&w.from) {
            pull(graph, reg, host, w.from, st)?;
        }
        if let Some(v) = st
            .outputs
            .get(&w.from)
            .and_then(|o| o.get(&w.from_port))
            .cloned()
        {
            inputs.insert(w.to_port.clone(), v);
        }
    }

    // A port with no wire takes its value from the node's own configuration, if the host can
    // interpret one. This runs before anything asks whether a required input is present, which
    // is the whole point: in most editors an inspector field is the ordinary way to fill a port,
    // and treating those as missing would call live branches dead and still report the run ok.
    let node = graph.node(nid).expect("gathering for a node that exists");
    if let Some(spec) = reg.get(&node.kind) {
        for port in spec.inputs.resolve(&node.config) {
            if port.ty == crate::port::PortType::EXEC || inputs.contains_key(&port.name) {
                continue;
            }
            if let Some(v) = host.literals().read(&node.kind, &port, &node.config) {
                inputs.insert(port.name, v);
            }
        }
    }

    Ok(inputs)
}

/// Evaluate a pure node because something is reading it.
pub(crate) fn pull<M: GraphMeta, H: Host<Meta = M>>(
    graph: &Graph<M>,
    reg: &NodeRegistry<H>,
    host: &H,
    nid: u32,
    st: &mut State,
) -> Result<(), RunError> {
    let node = graph.node(nid).expect("wire source exists");
    let spec = reg.get(&node.kind).ok_or_else(|| RunError::UnknownKind {
        node: nid,
        kind: node.kind.as_str().to_string(),
    })?;

    // An effectful node that has not been reached stays unreached. Pulling it would run the
    // untaken side of a branch through the back door.
    if spec.purity.has_exec() {
        return Ok(());
    }
    if st.ran.contains(&nid) && spec.purity != Purity::PURE_SOURCE {
        return Ok(());
    }

    let vars = st.vars.clone();
    let inputs = gather(graph, reg, host, nid, st)?;
    let Behavior::Run(runner) = &spec.behavior else {
        st.ran.insert(nid);
        return Ok(());
    };
    let cx = NodeCx {
        config: &node.config,
        inputs: &inputs,
        node: nid,
        host,
        vars,
    };
    st.steps += 1;
    host.observer().node_started(nid);
    let out = runner.run(&cx).map_err(|e| RunError::Node {
        node: nid,
        kind: node.kind.as_str().to_string(),
        message: e.message,
        retry: e.retry,
    })?;
    let summary = runner.summary(&cx, &out);
    host.observer().node_finished(nid, &summary, 0);
    st.outputs.insert(nid, out);
    st.ran.insert(nid);
    Ok(())
}

/// Read a node's output port after a run.
pub fn output<'a>(outputs: &'a Outputs, node: u32, port: &str) -> Option<&'a Value> {
    outputs.get(&node)?.get(&PortName::new(port))
}
