//! Layer 3 for `rastai.model/1`, block definitions and block instances (M2-01, contracts 5.2 and 5.3). Findings:
//! - a block instance (op `block`) with no `block_ref`, or a `block_ref` on any other op: `RRS-E2006`;
//! - a `block_ref` that names no block in `blocks`: `RRS-E2001` at the reference;
//! - a `repeat` of 0, or one on an op other than `block`: `RRS-E2006`;
//! - inside a block body: an op that isn't in the vocabulary (`RRS-E2001`), an id used twice (`RRS-E2005`), or a
//!   block instance, which would let a block contain itself (`RRS-E2006`).
//!
//! A block body's ops get no attribute or shape checks here: the decoder blocks are built with M2-03, and the shape
//! walk stops at a block instance until then, the same as at any op without shapes.

use std::collections::BTreeSet;

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::model::{GraphNode, ModelGraph, ModelSpec};
use crate::documents::model_ops::MODEL_OPS;
use crate::json::pointer::join_pointer;
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_UNRESOLVED: &str = "RRS-E2001";
const CODE_DUPLICATE_ID: &str = "RRS-E2005";
const CODE_VALUE_FORM: &str = "RRS-E2006";

const BLOCK_OP: &str = "block";
const NODES_POINTER: &str = "/graph/nodes";
const BLOCKS_POINTER: &str = "/blocks";
const BLOCK_REF_KEY: &str = "block_ref";
const REPEAT_KEY: &str = "repeat";
const OP_KEY: &str = "op";
const NODES_KEY: &str = "nodes";

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

/// One block body: its ops are in the vocabulary, its ids are unique and it holds no block instances.
fn check_block_body(name: &str, graph: &ModelGraph, source: &MeaningSource, diagnostics: &mut Vec<Diagnostic>) {
    let body_pointer = join_pointer(BLOCKS_POINTER, name);
    let nodes_pointer = join_pointer(&body_pointer, NODES_KEY);
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
