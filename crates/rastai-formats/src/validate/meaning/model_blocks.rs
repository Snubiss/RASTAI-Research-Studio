//! Layer 3 for `rastai.model/1`, block definitions and block instances (M2-01, contracts 5.2 and 5.3). Findings:
//! - a block instance (op `block`) with no `block_ref`, or a `block_ref` on any other op: `RRS-E2006`;
//! - a `block_ref` that names no block in `blocks`: `RRS-E2001` at the reference;
//! - a `repeat` of 0, or one on an op other than `block`: `RRS-E2006`;
//! - a block instance's input: `to_input` past 0 is `RRS-E2006`, and an instance fed by nothing or by two things is
//!   `RRS-E2007` (the instance's one input is checked in model_graph.rs with every other op's inputs);
//! - inside a block body: an op that isn't in the vocabulary (`RRS-E2001`), an id used twice (`RRS-E2005`), a
//!   block instance, which would let a block contain itself (`RRS-E2006`), or a `block_ref` or `repeat` on any node
//!   (`RRS-E2006`);
//! - a body node's attributes, checked as any graph node's are (`RRS-E2006`, model_graph.rs `check_attributes`);
//! - an edge that names a node of another graph, or no node (`RRS-E2001`); a `to_input` the op doesn't have
//!   (`RRS-E2006`); an input of a body node fed by two body edges (`RRS-E2007`, at the second edge);
//! - a loop with no `rnn` on it (`RRS-E2003`, at the edge that closes it, the search model_graph.rs shares);
//! - a body with other than one output node (`RRS-E2006`, at the body).
//!
//! The block interface (contracts 5.2): an input of a body node that no edge of the body feeds takes the block
//! instance's input, so a layer's first norm and its residual add both read the layer's input. The body's output is
//! its one node that no edge leaves; a block instance (op `block`) stands for that node's outputs.
//!
//! A body node's shapes aren't worked out here: the decoder blocks are built with M2-03, and the shape walk stops at
//! a block instance until then, the same as at any op without shapes.

use std::collections::{BTreeMap, BTreeSet};

use rastai_diag::diagnostic::Diagnostic;

use crate::describe::types::TypeDescription;
use crate::documents::model::{GraphNode, ModelGraph, ModelSpec};
use crate::documents::model_ops::{
    MANY_INPUTS_OP, MODEL_OPS, describe_model_op_types, fixed_input_count, op_attributes_name,
};
use crate::json::pointer::join_pointer;
use crate::validate::meaning::model_graph::{check_attributes, check_loops};
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_UNRESOLVED: &str = "RRS-E2001";
const CODE_DUPLICATE_ID: &str = "RRS-E2005";
const CODE_VALUE_FORM: &str = "RRS-E2006";
const CODE_INPUT_CONNECTIONS: &str = "RRS-E2007";

/// The op of a block instance. model_graph.rs reads it too, for the instance's one input.
pub(super) const BLOCK_OP: &str = "block";
const NODES_POINTER: &str = "/graph/nodes";
const BLOCKS_POINTER: &str = "/blocks";
const BLOCK_REF_KEY: &str = "block_ref";
const REPEAT_KEY: &str = "repeat";
const OP_KEY: &str = "op";
const NODES_KEY: &str = "nodes";
const EDGES_KEY: &str = "edges";

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
    let op_types = describe_model_op_types();
    for (name, graph) in &model.blocks {
        check_block_body(name, graph, &op_types, source, &mut diagnostics);
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

/// One block body: its ops are in the vocabulary, its ids are unique and it holds no block instances; its nodes'
/// attributes are sound; its edges name its own nodes, each input is fed at most once, it has no loop without `rnn`,
/// and it has one output node.
fn check_block_body(
    name: &str,
    graph: &ModelGraph,
    op_types: &[TypeDescription],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let body_pointer = join_pointer(BLOCKS_POINTER, name);
    check_body_nodes(&body_pointer, graph, source, diagnostics);
    check_body_attributes(&body_pointer, graph, op_types, source, diagnostics);
    let edge_ends = check_body_edges(&body_pointer, graph, source, diagnostics);
    check_body_feeds(&body_pointer, graph, &edge_ends, source, diagnostics);
    let edges_pointer = join_pointer(&body_pointer, EDGES_KEY);
    diagnostics.extend(check_loops(graph, &edges_pointer, &edge_ends, source));
    check_body_output(&body_pointer, graph, &edge_ends, source, diagnostics);
}

/// Each body node's id, op, and block fields. A body node is never a block instance, so `block_ref` and `repeat` have
/// no meaning on it and any of them is `RRS-E2006`.
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
        check_body_block_fields(&node_pointer, node, source, diagnostics);
    }
}

/// A body node's `block_ref` and `repeat`, each refused when set: `RRS-E2006` at the field.
fn check_body_block_fields(
    node_pointer: &str,
    node: &GraphNode,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(block_ref) = &node.block_ref {
        let ref_pointer = join_pointer(node_pointer, BLOCK_REF_KEY);
        let values = [("found", shown_value(block_ref)), ("expected", EXPECTED_NO_BLOCK_REF)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &ref_pointer));
    }
    if let Some(repeat) = node.repeat {
        let repeat_pointer = join_pointer(node_pointer, REPEAT_KEY);
        let found = repeat.to_string();
        let values = [("found", found.as_str()), ("expected", EXPECTED_NO_REPEAT)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &repeat_pointer));
    }
}

/// Checks each body node's attributes against its op, as the top-level check does for every graph node
/// (`check_attributes`). Only the findings are kept: nothing later reads the typed values.
fn check_body_attributes(
    body_pointer: &str,
    graph: &ModelGraph,
    op_types: &[TypeDescription],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let nodes_pointer = join_pointer(body_pointer, NODES_KEY);
    for (index, node) in graph.nodes.iter().enumerate() {
        let attributes_pointer = join_pointer(&format!("{nodes_pointer}/{index}"), "attributes");
        check_attributes(
            &node.op,
            &node.attributes,
            op_types,
            &attributes_pointer,
            source,
            diagnostics,
        );
    }
}

/// Each body edge's ends name body nodes. Returns the (from, to) node index of each edge, in edge order, None for an
/// edge with an end that doesn't resolve; the other checks read only these.
fn check_body_edges(
    body_pointer: &str,
    graph: &ModelGraph,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Option<(usize, usize)>> {
    let edges_pointer = join_pointer(body_pointer, EDGES_KEY);
    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        index_of.entry(node.node_id.as_str()).or_insert(index);
    }
    let mut edge_ends = Vec::new();
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
        edge_ends.push(from.zip(to));
    }
    edge_ends
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

/// Each body edge's `to_input` is one its op takes (`RRS-E2006`), and each input is fed by at most one body edge
/// (`RRS-E2007`, at the second one). An input no body edge feeds takes the block instance's input, so it isn't an
/// error here. A concat takes as many inputs as are connected, so only its feeds are counted.
fn check_body_feeds(
    body_pointer: &str,
    graph: &ModelGraph,
    edge_ends: &[Option<(usize, usize)>],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let edges_pointer = join_pointer(body_pointer, EDGES_KEY);
    // (node index, input) to the edges that feed it, in edge order.
    let mut feeds: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (edge_index, ends) in edge_ends.iter().enumerate() {
        let Some((_from, to)) = *ends else {
            continue;
        };
        let op = graph.nodes[to].op.as_str();
        let to_input = graph.edges[edge_index].to_input as usize;
        if let Some(input_count) = body_input_count(op)
            && to_input >= input_count
        {
            let found = graph.edges[edge_index].to_input.to_string();
            let expected = format!("0 to {}: {op} takes {input_count} input(s)", input_count - 1);
            let values = [("found", found.as_str()), ("expected", expected.as_str())];
            let pointer = format!("{edges_pointer}/{edge_index}/to_input");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
            continue;
        }
        feeds.entry((to, to_input)).or_default().push(edge_index);
    }
    for ((node_index, input), edges) in &feeds {
        if edges.len() < 2 {
            continue;
        }
        let input_text = input.to_string();
        let count_text = edges.len().to_string();
        let node_id = graph.nodes[*node_index].node_id.as_str();
        let values = [
            ("input_index", input_text.as_str()),
            ("graph_node", node_id),
            ("connection_count", count_text.as_str()),
        ];
        let pointer = format!("{edges_pointer}/{}", edges[1]);
        diagnostics.push(meaning_diagnostic(CODE_INPUT_CONNECTIONS, &values, source, &pointer));
    }
}

/// How many inputs a body node's op takes, when the number is fixed. A concat takes as many as are connected, so it
/// has no fixed count and any `to_input` is taken as it comes. An op with no shapes has no count either, the same as
/// in the main graph (model_graph.rs `connects_inputs`): an op outside the vocabulary gets its `RRS-E2001` and nothing
/// more, and a vocabulary op without shapes is accepted with any `to_input`.
fn body_input_count(op: &str) -> Option<usize> {
    if op == MANY_INPUTS_OP || op_attributes_name(op).is_none() {
        return None;
    }
    Some(fixed_input_count(op))
}

/// The body's output is its one node that no edge leaves; any other count is `RRS-E2006` at the body.
fn check_body_output(
    body_pointer: &str,
    graph: &ModelGraph,
    edge_ends: &[Option<(usize, usize)>],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut has_outgoing = vec![false; graph.nodes.len()];
    for &(from, _to) in edge_ends.iter().flatten() {
        has_outgoing[from] = true;
    }
    let outputs = has_outgoing.iter().filter(|&&leaves| !leaves).count();
    if outputs != 1 {
        let found = format!("{outputs} nodes that no edge leaves");
        let values = [("found", found.as_str()), ("expected", EXPECTED_ONE_OUTPUT)];
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, body_pointer));
    }
}
