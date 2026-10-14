//! Layer 3 for `rastai.model/1`, part 1: the graph and what refers to it. Findings come in the order the fields are
//! written, then the shapes:
//! - a graph node id used twice: `RRS-E2005` at the second;
//! - an op that isn't in the vocabulary of contracts 5.3: `RRS-E2001` at its `op`;
//! - attributes an op with shapes can't use: `RRS-E2006` at the attribute (a missing, unknown or wrong-kind one, a
//!   size of 0, a probability of 1). They are walked with layer 2's walker against the op's description in
//!   documents/model_ops.rs, but reported with layer 3's code: layer 3 never blocks loading;
//! - an edge, input map entry, head or placement that names no graph node: `RRS-E2001`;
//! - an input map range that is empty or runs past `defaults.sequence_length`, or a default size of 0: `RRS-E2006`;
//! - an edge into an input an op doesn't have: `RRS-E2006` at its `to_input`; an input of an op with shapes, or of a
//!   block instance (one input, op `block`), fed by nothing or by two things: `RRS-E2007`;
//! - a loop with no `rnn` graph node on it: `RRS-E2003` at the edge that closes it (block bodies get the same search,
//!   in model_blocks.rs);
//! - an input or output name used twice: `RRS-E2005`; a weights source that isn't a model ref: `RRS-E2006`;
//! - then, only when none of the above found an error, the shapes along the graph (validate/meaning/model_shapes.rs):
//!   a shape an op can't take is `RRS-E2002` at the edge that carries it.
//!
//! The loop search leaves out every `rnn` graph node and the edges at it, then looks for a loop in what is left: a
//! loop there is exactly a loop with no `rnn` on it. Searching the whole graph and dropping loops through an `rnn`
//! would miss a loop that shares its closing edge with one through an `rnn`. It walks with its own stack, never the
//! call stack, so a deep graph can't overflow it.

use std::collections::{BTreeMap, BTreeSet};

use rastai_diag::diagnostic::{Diagnostic, has_errors};
use serde_json::Value;

use crate::describe::structure_check::{MismatchKind, StructureCheck, StructureMismatch, find_mismatches};
use crate::describe::types::TypeDescription;
use crate::documents::model::{ModelGraph, ModelSpec};
use crate::documents::model_ops::{
    MANY_INPUTS_OP, MODEL_OPS, OpAttributes, RECURRENT_OP, describe_model_op_types, fixed_input_count,
    op_attribute_problems, op_attributes_from_value, op_attributes_name,
};
use crate::json::pointer::join_pointer;
use crate::validate::meaning::model_blocks::{BLOCK_OP, check_blocks};
use crate::validate::meaning::model_shapes::{InputSource, ShapeWalk, check_model_shapes};
use crate::validate::meaning::{MeaningSource, meaning_diagnostic, shown_value};

const CODE_UNRESOLVED: &str = "RRS-E2001";
const CODE_CYCLE: &str = "RRS-E2003";
const CODE_DUPLICATE_ID: &str = "RRS-E2005";
const CODE_VALUE_FORM: &str = "RRS-E2006";
const CODE_INPUT_CONNECTIONS: &str = "RRS-E2007";

const NODES_POINTER: &str = "/graph/nodes";
const EDGES_POINTER: &str = "/graph/edges";
const INPUTS_POINTER: &str = "/inputs";
const OUTPUTS_POINTER: &str = "/outputs";
const WEIGHTS_POINTER: &str = "/weights";
const PLACEMENT_POINTER: &str = "/placement";
const DEFAULTS_POINTER: &str = "/defaults";

/// The concat op's one signed attribute; every other integer attribute is a size of 0 or more.
const SIGNED_ATTRIBUTE: &str = "axis";
/// The model ref forms weights may come from (contracts 5.2, 8.6).
const WEIGHTS_SOURCE_PREFIXES: &[&str] = &["hf:", "blob:", "ckpt:"];
/// A loop through more graph nodes than this is shown with its first ones and a count, so one finding stays short.
const MAX_LOOP_NODES_SHOWN: usize = 8;

const FOUND_MISSING: &str = "(missing)";
const EXPECTED_SIZE: &str = "a whole number of 0 or more that fits in 64 bits";
const EXPECTED_SIGNED: &str = "a whole number from -9223372036854775808 to 9223372036854775807";
const EXPECTED_POSITIVE: &str = "a whole number of 1 or more";
const EXPECTED_WEIGHTS_SOURCE: &str = "a model ref: hf:<org>/<name>@<revision>, blob:<hash> or ckpt:<checkpoint_id>";

// The loop search's marks for each graph node.
const NOT_VISITED: u8 = 0;
const ON_PATH: u8 = 1;
const FINISHED: u8 = 2;

/// Every meaning problem of a ModelSpec.
pub fn check_model(model: &ModelSpec, source: &MeaningSource) -> Vec<Diagnostic> {
    let node_index = index_nodes(model);
    let mut diagnostics = check_node_ids(model, source);
    diagnostics.extend(check_blocks(model, source));
    let attributes = check_nodes(model, source, &mut diagnostics);
    let edge_ends = check_edges(model, &node_index, source, &mut diagnostics);
    diagnostics.extend(check_inputs(model, &node_index, source));
    let inputs = check_input_connections(model, &node_index, &attributes, source, &mut diagnostics);
    diagnostics.extend(check_loops(&model.graph, EDGES_POINTER, &edge_ends, source));
    diagnostics.extend(check_outputs(model, &node_index, source));
    diagnostics.extend(check_defaults(model, source));
    diagnostics.extend(check_weights(model, source));
    diagnostics.extend(check_placement(model, &node_index, source));
    if !has_errors(&diagnostics) {
        let resolved_ends = edge_ends.into_iter().flatten().collect();
        let walk = ShapeWalk {
            model,
            edge_ends: resolved_ends,
            attributes,
            inputs,
        };
        diagnostics.extend(check_model_shapes(&walk, source));
    }
    diagnostics
}

/// Graph node id to its index; the first node wins when an id is used twice (reported by `check_node_ids`).
fn index_nodes(model: &ModelSpec) -> BTreeMap<&str, usize> {
    let mut node_index = BTreeMap::new();
    for (index, node) in model.graph.nodes.iter().enumerate() {
        node_index.entry(node.node_id.as_str()).or_insert(index);
    }
    node_index
}

fn check_node_ids(model: &ModelSpec, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, node) in model.graph.nodes.iter().enumerate() {
        let node_id = node.node_id.as_str();
        if !seen.insert(node_id) {
            let values = [("id", shown_value(node_id))];
            let pointer = format!("{NODES_POINTER}/{index}/node_id");
            diagnostics.push(meaning_diagnostic(CODE_DUPLICATE_ID, &values, source, &pointer));
        }
    }
    diagnostics
}

/// Each node's op and attributes. Returns the typed attributes of every node whose op has shapes and whose
/// attributes are sound; None for the others.
fn check_nodes(
    model: &ModelSpec,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Option<OpAttributes>> {
    let op_types = describe_model_op_types();
    let mut typed_attributes = Vec::new();
    for (index, node) in model.graph.nodes.iter().enumerate() {
        let node_pointer = format!("{NODES_POINTER}/{index}");
        if !MODEL_OPS.contains(&node.op.as_str()) {
            let values = [("reference", shown_value(&node.op))];
            let pointer = join_pointer(&node_pointer, "op");
            diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &pointer));
        }
        let attributes_pointer = join_pointer(&node_pointer, "attributes");
        let typed = check_attributes(
            &node.op,
            &node.attributes,
            &op_types,
            &attributes_pointer,
            source,
            diagnostics,
        );
        typed_attributes.push(typed);
    }
    typed_attributes
}

/// One node's attributes against its op's description, then their values. None when the op has no shapes yet or
/// an attribute is reported.
pub(super) fn check_attributes(
    op: &str,
    attributes: &BTreeMap<String, Value>,
    op_types: &[TypeDescription],
    attributes_pointer: &str,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<OpAttributes> {
    let description = op_types
        .iter()
        .find(|item| Some(item.name.as_str()) == op_attributes_name(op))?;
    let check = StructureCheck {
        all_types: op_types,
        report_unknown_fields: true,
    };
    let value = Value::Object(attributes.clone().into_iter().collect());
    let mismatches = find_mismatches(&value, description, &check);
    for mismatch in &mismatches {
        diagnostics.push(attribute_mismatch_diagnostic(
            op,
            description,
            mismatch,
            attributes_pointer,
            source,
        ));
    }
    if !mismatches.is_empty() {
        return None;
    }
    let typed = match op_attributes_from_value(op, attributes)? {
        Ok(typed) => typed,
        Err(_) => {
            diagnostics.push(out_of_range_diagnostic(op, attributes, attributes_pointer, source));
            return None;
        }
    };
    let problems = op_attribute_problems(&typed);
    for problem in &problems {
        let found = attributes
            .get(problem.attribute)
            .map(Value::to_string)
            .unwrap_or_default();
        let values = [("found", found.as_str()), ("expected", problem.expected)];
        let pointer = join_pointer(attributes_pointer, problem.attribute);
        diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
    }
    problems.is_empty().then_some(typed)
}

/// `RRS-E2006` for an attribute the walk refused: "(missing) isn't an integer, which conv1d needs as kernel_size."
fn attribute_mismatch_diagnostic(
    op: &str,
    description: &TypeDescription,
    mismatch: &StructureMismatch,
    attributes_pointer: &str,
    source: &MeaningSource,
) -> Diagnostic {
    let attribute = mismatch.pointer.rsplit('/').next().unwrap_or_default();
    let (found, expected) = match mismatch.kind {
        MismatchKind::MissingField => (
            FOUND_MISSING.to_string(),
            format!("{}, which {op} needs as {attribute}", mismatch.expected),
        ),
        MismatchKind::UnknownField => {
            let known: Vec<&str> = description.fields.iter().map(|item| item.name.as_str()).collect();
            (
                attribute.to_string(),
                format!("an attribute of {op}: {}", known.join(", ")),
            )
        }
        MismatchKind::WrongKind | MismatchKind::ValueNotAllowed => (mismatch.found.clone(), mismatch.expected.clone()),
    };
    let values = [("found", found.as_str()), ("expected", expected.as_str())];
    let pointer = format!("{attributes_pointer}{}", mismatch.pointer);
    meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer)
}

/// `RRS-E2006` at the integer attribute serde refused: a size below 0, or a number past 64 bits.
fn out_of_range_diagnostic(
    op: &str,
    attributes: &BTreeMap<String, Value>,
    attributes_pointer: &str,
    source: &MeaningSource,
) -> Diagnostic {
    let refused = attributes
        .iter()
        .find(|(name, value)| !fits_attribute(value, is_signed_attribute(op, name)));
    let (pointer, found, expected) = match refused {
        Some((name, value)) => (
            join_pointer(attributes_pointer, name),
            value.to_string(),
            expected_integer_text(is_signed_attribute(op, name)),
        ),
        None => (attributes_pointer.to_string(), String::new(), EXPECTED_SIZE),
    };
    let values = [("found", shown_value(&found)), ("expected", expected)];
    meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer)
}

/// Whether an attribute may be below 0: only concat's axis, which counts back from the last axis.
fn is_signed_attribute(op: &str, name: &str) -> bool {
    op == MANY_INPUTS_OP && name == SIGNED_ATTRIBUTE
}

/// Whether serde can read a value as the attribute's integer. A value that isn't a number is never what serde
/// refused here, since layer 2's walk has already checked every attribute's kind.
fn fits_attribute(value: &Value, signed: bool) -> bool {
    if !value.is_number() {
        return true;
    }
    if signed { value.is_i64() } else { value.is_u64() }
}

fn expected_integer_text(signed: bool) -> &'static str {
    if signed { EXPECTED_SIGNED } else { EXPECTED_SIZE }
}

/// Both ends of every edge name a graph node. Returns the node indexes of each edge's ends, None for an edge with an
/// end that doesn't resolve.
fn check_edges(
    model: &ModelSpec,
    node_index: &BTreeMap<&str, usize>,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Option<(usize, usize)>> {
    let mut edge_ends = Vec::new();
    for (index, edge) in model.graph.edges.iter().enumerate() {
        let edge_pointer = format!("{EDGES_POINTER}/{index}");
        let from_index = resolve(
            node_index,
            &edge.from_node,
            &edge_pointer,
            "from_node",
            source,
            diagnostics,
        );
        let to_index = resolve(node_index, &edge.to_node, &edge_pointer, "to_node", source, diagnostics);
        edge_ends.push(from_index.zip(to_index));
    }
    edge_ends
}

/// The node index a reference names, or `RRS-E2001` at `<parent>/<key>`.
fn resolve(
    node_index: &BTreeMap<&str, usize>,
    reference: &str,
    parent_pointer: &str,
    key: &str,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<usize> {
    let found = node_index.get(reference).copied();
    if found.is_none() {
        let values = [("reference", shown_value(reference))];
        let pointer = join_pointer(parent_pointer, key);
        diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &pointer));
    }
    found
}

/// Each input map entry feeds a graph node, over a range of positions inside the sequence length, under a name no
/// other entry has.
fn check_inputs(model: &ModelSpec, node_index: &BTreeMap<&str, usize>, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let sequence_length = model.defaults.sequence_length;
    let mut seen = BTreeSet::new();
    for (index, input) in model.inputs.iter().enumerate() {
        let input_pointer = format!("{INPUTS_POINTER}/{index}");
        if !seen.insert(input.name.as_str()) {
            let values = [("id", shown_value(&input.name))];
            let pointer = join_pointer(&input_pointer, "name");
            diagnostics.push(meaning_diagnostic(CODE_DUPLICATE_ID, &values, source, &pointer));
        }
        let range = input.range;
        if range.start >= range.end || range.end > sequence_length {
            let found = format!("start {}, end {}", range.start, range.end);
            let expected = format!("a range with its end past its start and no further than {sequence_length}");
            let values = [("found", found.as_str()), ("expected", expected.as_str())];
            let pointer = join_pointer(&input_pointer, "range");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
        resolve(
            node_index,
            &input.encoder_node,
            &input_pointer,
            "encoder_node",
            source,
            &mut diagnostics,
        );
    }
    diagnostics
}

/// Every input of every op with shapes is fed exactly once, by an edge or an input map entry, and no edge feeds an
/// input its op doesn't have. Returns each node's inputs in order, for the shape walk.
fn check_input_connections(
    model: &ModelSpec,
    node_index: &BTreeMap<&str, usize>,
    attributes: &[Option<OpAttributes>],
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Vec<InputSource>> {
    let connections = input_connections(model, node_index, source, diagnostics);
    let mut inputs = Vec::new();
    for (index, node_connections) in connections.into_iter().enumerate() {
        // A second node with a taken id is already reported; its inputs would only repeat the first one's.
        let first_with_its_id = node_index.get(model.graph.nodes[index].node_id.as_str()) == Some(&index);
        let ordered = if first_with_its_id {
            check_node_connections(model, index, node_connections, source, diagnostics)
        } else {
            Vec::new()
        };
        // The shape walk only follows nodes it can work out.
        inputs.push(if attributes[index].is_some() {
            ordered
        } else {
            Vec::new()
        });
    }
    inputs
}

/// What feeds each input of each node with shapes: `connections[node][input]` lists the sources in file order.
fn input_connections(
    model: &ModelSpec,
    node_index: &BTreeMap<&str, usize>,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Vec<Vec<InputSource>>> {
    let nodes = &model.graph.nodes;
    let feed_counts = feed_counts(model, node_index);
    let mut connections: Vec<Vec<Vec<InputSource>>> = vec![Vec::new(); nodes.len()];
    for (input_index, input) in model.inputs.iter().enumerate() {
        if let Some(&target) = node_index.get(input.encoder_node.as_str()) {
            add_connection(&mut connections[target], 0, InputSource::InputMap(input_index));
        }
    }
    for (edge_index, edge) in model.graph.edges.iter().enumerate() {
        let Some(&target) = node_index.get(edge.to_node.as_str()) else {
            continue;
        };
        let op = nodes[target].op.as_str();
        if !connects_inputs(op) {
            continue;
        }
        let to_input = edge.to_input as usize;
        // A concat's inputs are numbered from 0 with no gap, so a number past its feed count can't be right; it
        // also keeps a hostile number from sizing the list below.
        let input_count = if op == MANY_INPUTS_OP {
            feed_counts[target]
        } else {
            fixed_input_count(op)
        };
        if to_input >= input_count {
            let found = edge.to_input.to_string();
            let expected = inputs_text(op, input_count);
            let values = [("found", found.as_str()), ("expected", expected.as_str())];
            let pointer = format!("{EDGES_POINTER}/{edge_index}/to_input");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
            continue;
        }
        add_connection(&mut connections[target], to_input, InputSource::Edge(edge_index));
    }
    connections
}

/// How many edges and input map entries feed each graph node.
fn feed_counts(model: &ModelSpec, node_index: &BTreeMap<&str, usize>) -> Vec<usize> {
    let mut counts = vec![0usize; model.graph.nodes.len()];
    let targets = model.graph.edges.iter().map(|edge| edge.to_node.as_str());
    let encoders = model.inputs.iter().map(|input| input.encoder_node.as_str());
    for target in targets.chain(encoders) {
        if let Some(&index) = node_index.get(target) {
            counts[index] += 1;
        }
    }
    counts
}

/// What a `to_input` should be: "0: conv1d takes one input", "0 to 1: residual_add takes 2 inputs", "0 to 2: this
/// concat has 3 inputs connected".
fn inputs_text(op: &str, input_count: usize) -> String {
    if op != MANY_INPUTS_OP {
        if input_count == 1 {
            return format!("0: {op} takes one input");
        }
        return format!("0 to {}: {op} takes {input_count} inputs", input_count - 1);
    }
    let last = input_count.saturating_sub(1);
    format!("0 to {last}: this {op} has {input_count} inputs connected")
}

/// Whether an op's inputs are connected and counted: the ops with shapes, and a block instance, whose one input is
/// the block's input.
fn connects_inputs(op: &str) -> bool {
    op_attributes_name(op).is_some() || op == BLOCK_OP
}

fn add_connection(node_connections: &mut Vec<Vec<InputSource>>, input: usize, connection: InputSource) {
    if node_connections.len() <= input {
        node_connections.resize(input + 1, Vec::new());
    }
    node_connections[input].push(connection);
}

/// `RRS-E2007` for each input of an op with shapes that nothing or more than one thing feeds. Returns the one
/// source of each input that has exactly one.
fn check_node_connections(
    model: &ModelSpec,
    index: usize,
    mut node_connections: Vec<Vec<InputSource>>,
    source: &MeaningSource,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<InputSource> {
    let node = &model.graph.nodes[index];
    if !connects_inputs(&node.op) {
        return Vec::new();
    }
    // Every input of an op with a fixed number of inputs is listed, even when nothing is connected to it; a concat
    // keeps at least its input 0.
    let fixed_count = fixed_input_count(&node.op);
    if node_connections.len() < fixed_count.max(1) {
        node_connections.resize(fixed_count.max(1), Vec::new());
    }
    let mut ordered = Vec::new();
    for (input, sources) in node_connections.iter().enumerate() {
        if let [only] = sources.as_slice() {
            ordered.push(*only);
            continue;
        }
        let pointer = match sources.get(1) {
            Some(second) => connection_pointer(*second),
            None => format!("{NODES_POINTER}/{index}"),
        };
        let input_text = input.to_string();
        let count_text = sources.len().to_string();
        let values = [
            ("input_index", input_text.as_str()),
            ("graph_node", node.node_id.as_str()),
            ("connection_count", count_text.as_str()),
        ];
        diagnostics.push(meaning_diagnostic(CODE_INPUT_CONNECTIONS, &values, source, &pointer));
    }
    ordered
}

fn connection_pointer(connection: InputSource) -> String {
    match connection {
        InputSource::Edge(edge_index) => format!("{EDGES_POINTER}/{edge_index}"),
        InputSource::InputMap(input_index) => format!("{INPUTS_POINTER}/{input_index}/encoder_node"),
    }
}

/// `RRS-E2003` at each edge that closes a loop with no `rnn` graph node on it. A depth-first search with its own
/// stack over the graph without its `rnn` nodes: an edge to a node still on the current path closes a loop through
/// the path's nodes from there. Every loop with no `rnn` has at least one such edge, so each one is reported. The
/// main graph and each block body share this search; `edges_pointer` is where the graph's edges live.
pub(super) fn check_loops(
    graph: &ModelGraph,
    edges_pointer: &str,
    edge_ends: &[Option<(usize, usize)>],
    source: &MeaningSource,
) -> Vec<Diagnostic> {
    let nodes = &graph.nodes;
    let outgoing = outgoing_edges_without_recurrence(graph, edge_ends);
    let mut marks = vec![NOT_VISITED; nodes.len()];
    let mut diagnostics = Vec::new();
    for start in 0..nodes.len() {
        if marks[start] != NOT_VISITED {
            continue;
        }
        marks[start] = ON_PATH;
        // Each entry is a node on the current path and how many of its outgoing edges were followed.
        let mut path: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(top) = path.last_mut() {
            let (node, followed) = *top;
            let Some(&(edge_index, next)) = outgoing[node].get(followed) else {
                marks[node] = FINISHED;
                path.pop();
                continue;
            };
            top.1 += 1;
            if marks[next] == NOT_VISITED {
                marks[next] = ON_PATH;
                path.push((next, 0));
            } else if marks[next] == ON_PATH {
                let loop_nodes = loop_from(&path, next);
                diagnostics.push(loop_diagnostic(graph, &loop_nodes, edges_pointer, edge_index, source));
            }
        }
    }
    diagnostics
}

/// Each node's outgoing edges as (edge index, node it goes to), leaving out every edge that starts or ends at an
/// `rnn` graph node, since a loop through an `rnn` is recurrence and not an error.
fn outgoing_edges_without_recurrence(
    graph: &ModelGraph,
    edge_ends: &[Option<(usize, usize)>],
) -> Vec<Vec<(usize, usize)>> {
    let nodes = &graph.nodes;
    let mut outgoing: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];
    for (edge_index, ends) in edge_ends.iter().enumerate() {
        let Some((from_index, to_index)) = *ends else {
            continue;
        };
        let touches_recurrence = nodes[from_index].op == RECURRENT_OP || nodes[to_index].op == RECURRENT_OP;
        if !touches_recurrence {
            outgoing[from_index].push((edge_index, to_index));
        }
    }
    outgoing
}

/// The nodes of the current path from `first` to its end: the loop an edge back to `first` closes.
fn loop_from(path: &[(usize, usize)], first: usize) -> Vec<usize> {
    let start = path.iter().position(|(node, _)| *node == first).unwrap_or_default();
    path[start..].iter().map(|(node, _)| *node).collect()
}

/// `RRS-E2003` for a loop with no `rnn` on it, at the edge that closes it.
fn loop_diagnostic(
    graph: &ModelGraph,
    loop_nodes: &[usize],
    edges_pointer: &str,
    edge_index: usize,
    source: &MeaningSource,
) -> Diagnostic {
    let nodes = &graph.nodes;
    let mut names: Vec<&str> = loop_nodes
        .iter()
        .take(MAX_LOOP_NODES_SHOWN)
        .map(|&index| nodes[index].node_id.as_str())
        .collect();
    let more = loop_nodes.len().saturating_sub(MAX_LOOP_NODES_SHOWN);
    let more_text = format!("{more} more");
    if more > 0 {
        names.push(&more_text);
    }
    let graph_nodes = names.join(", ");
    let values = [("graph_nodes", graph_nodes.as_str())];
    let pointer = format!("{edges_pointer}/{edge_index}");
    meaning_diagnostic(CODE_CYCLE, &values, source, &pointer)
}

/// Each head names a graph node, under a name no other head has.
fn check_outputs(model: &ModelSpec, node_index: &BTreeMap<&str, usize>, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, output) in model.outputs.iter().enumerate() {
        let output_pointer = format!("{OUTPUTS_POINTER}/{index}");
        if !seen.insert(output.name.as_str()) {
            let values = [("id", shown_value(&output.name))];
            let pointer = join_pointer(&output_pointer, "name");
            diagnostics.push(meaning_diagnostic(CODE_DUPLICATE_ID, &values, source, &pointer));
        }
        resolve(
            node_index,
            &output.graph_node,
            &output_pointer,
            "graph_node",
            source,
            &mut diagnostics,
        );
    }
    diagnostics
}

/// The sizes shapes are worked out with are 1 or more.
fn check_defaults(model: &ModelSpec, source: &MeaningSource) -> Vec<Diagnostic> {
    let defaults = &model.defaults;
    let mut diagnostics = Vec::new();
    for (key, size) in [
        ("sequence_length", defaults.sequence_length),
        ("batch_size", defaults.batch_size),
    ] {
        if size == 0 {
            let values = [("found", "0"), ("expected", EXPECTED_POSITIVE)];
            let pointer = join_pointer(DEFAULTS_POINTER, key);
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
    }
    diagnostics
}

/// Each weights source is a model ref of a kind weights come from.
fn check_weights(model: &ModelSpec, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, weights) in model.weights.iter().enumerate() {
        let text = weights.source.as_str();
        let is_model_ref = WEIGHTS_SOURCE_PREFIXES
            .iter()
            .any(|prefix| text.len() > prefix.len() && text.starts_with(prefix));
        if !is_model_ref {
            let values = [("found", shown_value(text)), ("expected", EXPECTED_WEIGHTS_SOURCE)];
            let pointer = format!("{WEIGHTS_POINTER}/{index}/source");
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
    }
    diagnostics
}

/// Each placed graph node exists.
fn check_placement(model: &ModelSpec, node_index: &BTreeMap<&str, usize>, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for node_id in model.placement.keys() {
        if !node_index.contains_key(node_id.as_str()) {
            let values = [("reference", shown_value(node_id))];
            let pointer = join_pointer(PLACEMENT_POINTER, node_id);
            diagnostics.push(meaning_diagnostic(CODE_UNRESOLVED, &values, source, &pointer));
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_is_the_path_from_the_node_an_edge_returns_to() {
        let path = [(4, 1), (2, 1), (7, 0), (9, 1)];
        assert_eq!(loop_from(&path, 7), vec![7, 9]);
        assert_eq!(loop_from(&path, 4), vec![4, 2, 7, 9]);
    }

    #[test]
    fn only_concats_axis_may_be_below_zero() {
        assert!(is_signed_attribute("concat", "axis"));
        assert!(!is_signed_attribute("conv1d", "axis"));
        assert!(!is_signed_attribute("concat", "kernel_size"));
    }

    #[test]
    fn a_signed_attribute_takes_numbers_below_zero_and_a_size_takes_numbers_past_the_signed_range() {
        let below_zero = Value::from(-1);
        let past_signed = Value::from(10_000_000_000_000_000_000u64);
        assert!(fits_attribute(&below_zero, true) && !fits_attribute(&below_zero, false));
        assert!(fits_attribute(&past_signed, false) && !fits_attribute(&past_signed, true));
        assert!(fits_attribute(&Value::from("text"), false));
    }

    #[test]
    fn a_refused_signed_attribute_is_not_described_as_a_size() {
        assert_eq!(expected_integer_text(true), EXPECTED_SIGNED);
        assert_eq!(expected_integer_text(false), EXPECTED_SIZE);
        assert!(!EXPECTED_SIGNED.contains("0 or more"));
    }

    #[test]
    fn connection_pointers_name_the_edge_or_the_input_map_entry() {
        assert_eq!(connection_pointer(InputSource::Edge(3)), "/graph/edges/3");
        assert_eq!(connection_pointer(InputSource::InputMap(0)), "/inputs/0/encoder_node");
    }

    #[test]
    fn connections_grow_to_the_input_they_name() {
        let mut node_connections = Vec::new();
        add_connection(&mut node_connections, 2, InputSource::Edge(5));
        assert_eq!(node_connections.len(), 3);
        assert!(node_connections[0].is_empty() && node_connections[1].is_empty());
        assert_eq!(node_connections[2], vec![InputSource::Edge(5)]);
    }
}
