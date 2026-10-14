//! Layer 3 for `rastai.model/1`, block definitions and block instances (M2-01, contracts 5.2 and 5.3). Findings:
//! - a block instance (op `block`) with no `block_ref`, or a `block_ref` on any other op: `RRS-E2006`;
//! - a `block_ref` that names no block in `blocks`: `RRS-E2001` at the reference;
//! - a `repeat` of 0, or one on an op other than `block`: `RRS-E2006`;
//! - inside a block body: an op that isn't in the vocabulary (`RRS-E2001`), an id used twice (`RRS-E2005`), or a
//!   block instance, which would let a block contain itself (`RRS-E2006`);
//! - an edge that names a node of another graph, or no node (`RRS-E2001`); a `to_input` the op doesn't have
//!   (`RRS-E2006`); a loop with no `rnn` on it (`RRS-E2003`, at the body);
//! - a body with other than one output node (`RRS-E2006`, at the body).
//!
//! The block interface (contracts 5.2): an input of a body node that no edge of the body feeds takes the block
//! instance's input, so a layer's first norm and its residual add both read the layer's input. The body's output is
//! its one node that no edge leaves; a block instance (op `block`) stands for that node's outputs.
//!
//! A block body's ops get no attribute or shape checks here: the decoder blocks are built with M2-03, and the shape
//! walk stops at a block instance until then, the same as at any op without shapes.

use std::collections::{BTreeMap, BTreeSet};

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::model::{GraphNode, ModelGraph, ModelSpec};
use crate::documents::model_ops::{MANY_INPUTS_OP, MODEL_OPS, fixed_input_count};
use crate::json::pointer::join_pointer;
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_UNRESOLVED: &str = "RRS-E2001";
const CODE_CYCLE: &str = "RRS-E2003";
const CODE_DUPLICATE_ID: &str = "RRS-E2005";
const CODE_VALUE_FORM: &str = "RRS-E2006";

const BLOCK_OP: &str = "block";
const NODES_POINTER: &str = "/graph/nodes";
const BLOCKS_POINTER: &str = "/blocks";
const BLOCK_REF_KEY: &str = "block_ref";
const REPEAT_KEY: &str = "repeat";
const OP_KEY: &str = "op";
const NODES_KEY: &str = "nodes";
const EDGES_KEY: &str = "edges";
const RECURRENT_OP: &str = "rnn";

const EXPECTED_ONE_OUTPUT: &str = "exactly one node that no edge leaves: the block's output";

/// What a missing `block_ref` is shown as, and what a block instance needs.
const FOUND_MISSING: &str = "(missing)";
const EXPECTED_BLOCK_REF: &str = "a block id from blocks: a block instance (op block) names the block it stamps";
const EXPECTED_NO_BLOCK_REF: &str = "left out: only a block instance (op block) names a block";
const EXPECTED_REPEAT: &str = "a whole number of 1 or more";
const EXPECTED_NO_REPEAT: &str = "left out: only a block instance (op block) repeats";
const EXPECTED_OP_IN_BLOCK: &str = "an op other than block: a block's body holds no block instances";

/// Every finding about the block definitions and the block instances of the graph, in document order.
pub fn check_blocks(model: &ModelSpec, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, node) in model.graph.nodes.iter().enumerate() {
        check_instance(model, index, node, source, &mut diagnostics);
    }
    for (name, graph) in &model.blocks {
        check_block_body(name, graph, source, &mut diagnostics);
    }
    diagnostics
}

/// One graph node's block fields against its op.
fn check_instance(
    model: &ModelSpec,
    index: usize,
    node: &GraphNode,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let is_instance = node.op == BLOCK_OP;
    let node_pointer = format!("{NODES_POINTER}/{index}");
    let ref_pointer = join_pointer(&node_pointer, BLOCK_REF_KEY);
    match (is_instance, node.block_ref.as_deref()) {
        (true, None) => {
            let values = [("found", FOUND_MISSING), ("expected", EXPECTED_BLOCK_REF)];
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &node_pointer));
        }
        (true, Some(block_ref)) => {
            if !model.blocks.contains_key(block_ref) {
                let values = [("reference", shown_value(block_ref))];
                diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &ref_pointer));
            }
        }
        (false, Some(block_ref)) => {
            let values = [("found", shown_value(block_ref)), ("expected", EXPECTED_NO_BLOCK_REF)];
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &ref_pointer));
        }
        (false, None) => {}
    }
    let Some(repeat) = node.repeat else {
        return;
    };
    let repeat_pointer = join_pointer(&node_pointer, REPEAT_KEY);
    let found = repeat.to_string();
    if !is_instance {
        let values = [("found", found.as_str()), ("expected", EXPECTED_NO_REPEAT)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &repeat_pointer));
    } else if repeat == 0 {
        let values = [("found", found.as_str()), ("expected", EXPECTED_REPEAT)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &repeat_pointer));
    }
}

/// One block body: its ops are in the vocabulary, its ids are unique and it holds no block instances; its edges
/// name its own nodes and have no loop; it has one output node.
fn check_block_body(name: &str, graph: &ModelGraph, source: &MeaningSource, diagnostics: &mut Vec<Diagnostic>) {
    let body_pointer = join_pointer(BLOCKS_POINTER, name);
    check_body_nodes(&body_pointer, graph, source, diagnostics);
    let edges = check_body_edges(&body_pointer, graph, source, diagnostics);
    check_body_loops(&body_pointer, graph, &edges, source, diagnostics);
    check_body_output(&body_pointer, graph, &edges, source, diagnostics);
}

/// Each body node's id and op.
fn check_body_nodes(body_pointer: &str, graph: &ModelGraph, source: &MeaningSource, diagnostics: &mut Vec<Diagnostic>) {
    let nodes_pointer = join_pointer(body_pointer, NODES_KEY);
    let mut seen_ids = BTreeSet::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let node_pointer = format!("{nodes_pointer}/{index}");
        if !seen_ids.insert(node.node_id.as_str()) {
            let values = [("id", shown_value(&node.node_id))];
            let pointer = join_pointer(&node_pointer, "node_id");
            diagnostics.push(meaning_diagnostic(CODE_DUPLICATE_ID, &values, source, &pointer));
        }
        let op_pointer = join_pointer(&node_pointer, OP_KEY);
        if !MODEL_OPS.contains(&node.op.as_str()) {
            let values = [("reference", shown_value(&node.op))];
            diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &op_pointer));
        } else if node.op == BLOCK_OP {
            let values = [("found", BLOCK_OP), ("expected", EXPECTED_OP_IN_BLOCK)];
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &op_pointer));
        }
    }
}

/// Each body edge's ends name body nodes and its `to_input` is one the op takes. Returns the (from, to) node index of
/// each edge whose ends resolve, in edge order; the other checks read only these.
fn check_body_edges(
    body_pointer: &str,
    graph: &ModelGraph,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(usize, usize)> {
    let edges_pointer = join_pointer(body_pointer, EDGES_KEY);
    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        index_of.entry(node.node_id.as_str()).or_insert(index);
    }
    let mut resolved = Vec::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let edge_pointer = format!("{edges_pointer}/{edge_index}");
        let from = resolve_body_end(
            &index_of,
            &edge.from_node,
            &edge_pointer,
            "from_node",
            source,
            diagnostics,
        );
        let to = resolve_body_end(&index_of, &edge.to_node, &edge_pointer, "to_node", source, diagnostics);
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let op = graph.nodes[to].op.as_str();
        if let Some(input_count) = body_input_count(op).filter(|&count| edge.to_input as usize >= count) {
            let found = edge.to_input.to_string();
            let expected = format!("0 to {}: {op} takes {input_count} input(s)", input_count - 1);
            let values = [("found", found.as_str()), ("expected", expected.as_str())];
            let pointer = join_pointer(&edge_pointer, "to_input");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
        resolved.push((from, to));
    }
    resolved
}

/// The node index a body edge's end names, or `RRS-E2001` at `<edge>/<key>`.
fn resolve_body_end(
    index_of: &BTreeMap<&str, usize>,
    reference: &str,
    edge_pointer: &str,
    key: &str,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<usize> {
    let found = index_of.get(reference).copied();
    if found.is_none() {
        let values = [("reference", shown_value(reference))];
        let pointer = join_pointer(edge_pointer, key);
        diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &pointer));
    }
    found
}

/// How many inputs a body node's op takes, when the number is fixed: a concat takes as many as are connected, so it
/// has no fixed count and any `to_input` is checked against the feed later.
fn body_input_count(op: &str) -> Option<usize> {
    if op == MANY_INPUTS_OP {
        return None;
    }
    Some(fixed_input_count(op))
}

/// `RRS-E2003` at the body when its edges form a loop with no `rnn` node on it. Kahn's method, which keeps no call
/// stack: repeatedly take off the nodes with no edge coming in; what is left after none can be taken off is a loop.
fn check_body_loops(
    body_pointer: &str,
    graph: &ModelGraph,
    edges: &[(usize, usize)],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let recurrent: Vec<bool> = graph.nodes.iter().map(|node| node.op == RECURRENT_OP).collect();
    let mut incoming = vec![0usize; graph.nodes.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for &(from, to) in edges {
        if !recurrent[from] && !recurrent[to] {
            incoming[to] += 1;
            outgoing[from].push(to);
        }
    }
    let mut ready: Vec<usize> = (0..graph.nodes.len())
        .filter(|&node| incoming[node] == 0 && !recurrent[node])
        .collect();
    let mut taken_off = vec![false; graph.nodes.len()];
    while let Some(node) = ready.pop() {
        taken_off[node] = true;
        for &next in &outgoing[node] {
            incoming[next] -= 1;
            if incoming[next] == 0 {
                ready.push(next);
            }
        }
    }
    // Nodes that never came free are the loop and whatever it feeds: all of them are named, as the loop's graph.
    let names: Vec<&str> = (0..graph.nodes.len())
        .filter(|&node| !recurrent[node] && !taken_off[node])
        .map(|node| graph.nodes[node].node_id.as_str())
        .collect();
    if !names.is_empty() {
        let graph_nodes = names.join(", ");
        let values = [("graph_nodes", graph_nodes.as_str())];
        diagnostics.push(meaning_diagnostic(CODE_CYCLE, &values, source, body_pointer));
    }
}

/// The body's output is its one node that no edge leaves; any other count is `RRS-E2006` at the body.
fn check_body_output(
    body_pointer: &str,
    graph: &ModelGraph,
    edges: &[(usize, usize)],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut has_outgoing = vec![false; graph.nodes.len()];
    for &(from, _to) in edges {
        has_outgoing[from] = true;
    }
    let outputs = has_outgoing.iter().filter(|&&leaves| !leaves).count();
    if outputs != 1 {
        let found = format!("{outputs} nodes that no edge leaves");
        let values = [("found", found.as_str()), ("expected", EXPECTED_ONE_OUTPUT)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, body_pointer));
    }
}
