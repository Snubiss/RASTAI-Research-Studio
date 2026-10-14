//! Layer 3 for `rastai.model/1`, part 2: shape inference along the graph. Each op with shapes (documents/
//! model_ops.rs) turns its input shapes into its output shape; a shape an op can't take is `RRS-E2002` at the edge
//! that carries it, or at the input map entry when the graph's own input doesn't fit its encoder node.
//!
//! The walk starts from the input map: an input's shape is `[defaults.batch_size, range length]`, token ids. It runs
//! only on a graph the other checks passed (validate/meaning/model_graph.rs), so every reference resolves and every
//! input of an op with shapes is connected exactly once. It stops quietly past an op whose shapes aren't built yet
//! and past a node it reported, so one broken edge gives one finding, not one per node after it.
//!
//! `infer_op_output_shapes` is the per-op rule `tests/op_shapes.rs` checks against the golden files in
//! `fixtures/ops/`, which the Python compiler and Rust live stats must match too (contracts 2.4 rule 10).

use std::collections::VecDeque;

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::model::ModelSpec;
use crate::documents::model_ops::{
    AttentionAttributes, ClassifierHeadAttributes, ConcatAttributes, Conv1dAttributes, EmbeddingAttributes,
    FeedForwardAttributes, LinearAttributes, OpAttributes, PoolAttributes, UnembedAttributes,
};
use crate::validate::meaning::{MeaningSource, meaning_diagnostic};

const CODE_SHAPE_MISMATCH: &str = "RRS-E2002";
const GRAPH_EDGES_POINTER: &str = "/graph/edges";
const INPUTS_POINTER: &str = "/inputs";
/// How an input map entry is named where a graph node would be: "input tokens gives shape ...".
const INPUT_PREFIX: &str = "input ";
/// What an expected shape says when a size would pass 64 bits, which only a hostile file reaches.
const EXPECTED_FITTING_SIZES: &str = "a shape whose sizes fit in 64 bits";
/// What an expected shape says for a window step of 0. Layer 3 reports that attribute before any shape is worked
/// out; this keeps the public rule from dividing by zero when it is called on its own.
const EXPECTED_MOVING_WINDOW: &str = "a window that moves: a stride of 1 or more";

/// Why an op can't take its inputs: which input, and the shape it wants there, written for a person
/// (`[batch, length, 128]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeProblem {
    pub input_index: usize,
    pub expected: String,
}

/// Where one input of a graph node comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    /// An edge, by its index in `graph.edges`.
    Edge(usize),
    /// An input map entry, by its index in `inputs`.
    InputMap(usize),
}

/// What the graph checks learned, which the shape walk needs. Every list is indexed like `graph.nodes`.
pub struct ShapeWalk<'a> {
    pub model: &'a ModelSpec,
    /// The node index each edge's `from_node` and `to_node` name.
    pub edge_ends: Vec<(usize, usize)>,
    /// Each node's typed attributes; None for an op whose shapes aren't built.
    pub attributes: Vec<Option<OpAttributes>>,
    /// Each node's inputs in order; only filled for nodes with attributes.
    pub inputs: Vec<Vec<InputSource>>,
}

/// Every shape mismatch along the graph, in the order the walk reaches them.
pub fn check_model_shapes(walk: &ShapeWalk, source: &MeaningSource) -> Vec<Diagnostic> {
    let node_count = walk.model.graph.nodes.len();
    let mut output_shapes: Vec<Option<Vec<u64>>> = vec![None; node_count];
    let mut diagnostics = Vec::new();
    for node_index in graph_order(walk) {
        let Some(attributes) = &walk.attributes[node_index] else {
            continue;
        };
        let Some(input_shapes) = known_input_shapes(walk, node_index, &output_shapes) else {
            continue;
        };
        match infer_op_output_shapes(attributes, &input_shapes) {
            Ok(mut shapes) => output_shapes[node_index] = shapes.pop(),
            Err(problem) => diagnostics.push(mismatch_diagnostic(walk, node_index, &input_shapes, &problem, source)),
        }
    }
    diagnostics
}

/// The nodes in an order where every edge points forward (Kahn's algorithm, document order among equals). A node
/// on a loop (only an `rnn` may close one) or after one is left out: its shapes aren't worked out.
fn graph_order(walk: &ShapeWalk) -> Vec<usize> {
    let node_count = walk.model.graph.nodes.len();
    let mut waiting_inputs = vec![0usize; node_count];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(from_index, to_index) in &walk.edge_ends {
        waiting_inputs[to_index] += 1;
        outgoing[from_index].push(to_index);
    }
    let mut ready: VecDeque<usize> = (0..node_count).filter(|&index| waiting_inputs[index] == 0).collect();
    let mut order = Vec::new();
    while let Some(node_index) = ready.pop_front() {
        order.push(node_index);
        for &next in &outgoing[node_index] {
            waiting_inputs[next] -= 1;
            if waiting_inputs[next] == 0 {
                ready.push_back(next);
            }
        }
    }
    order
}

/// The shapes of a node's inputs, or None when one isn't known (its source was skipped or reported).
fn known_input_shapes(
    walk: &ShapeWalk,
    node_index: usize,
    output_shapes: &[Option<Vec<u64>>],
) -> Option<Vec<Vec<u64>>> {
    let mut shapes = Vec::new();
    for input in &walk.inputs[node_index] {
        let shape = match *input {
            InputSource::Edge(edge_index) => output_shapes[walk.edge_ends[edge_index].0].clone()?,
            InputSource::InputMap(input_index) => input_map_shape(walk.model, input_index),
        };
        shapes.push(shape);
    }
    Some(shapes)
}

/// Token ids: `[batch_size, positions in the range]`. The graph checks made sure the range's end is past its start.
fn input_map_shape(model: &ModelSpec, input_index: usize) -> Vec<u64> {
    let range = model.inputs[input_index].range;
    vec![model.defaults.batch_size, range.end - range.start]
}

/// `RRS-E2002` at the edge or input map entry that brings the input the op can't take.
fn mismatch_diagnostic(
    walk: &ShapeWalk,
    node_index: usize,
    input_shapes: &[Vec<u64>],
    problem: &ShapeProblem,
    source: &MeaningSource,
) -> Diagnostic {
    let model = walk.model;
    let (from_name, pointer) = match walk.inputs[node_index][problem.input_index] {
        InputSource::Edge(edge_index) => {
            let from_node = &model.graph.edges[edge_index].from_node;
            (from_node.clone(), format!("{GRAPH_EDGES_POINTER}/{edge_index}"))
        }
        InputSource::InputMap(input_index) => {
            let input_name = &model.inputs[input_index].name;
            (
                format!("{INPUT_PREFIX}{input_name}"),
                format!("{INPUTS_POINTER}/{input_index}"),
            )
        }
    };
    let from_shape = shape_text(&input_shapes[problem.input_index]);
    let to_node = &model.graph.nodes[node_index].node_id;
    let values = [
        ("from_graph_node", from_name.as_str()),
        ("from_shape", from_shape.as_str()),
        ("to_graph_node", to_node.as_str()),
        ("to_shape", problem.expected.as_str()),
    ];
    meaning_diagnostic(CODE_SHAPE_MISMATCH, &values, source, &pointer)
}

/// One op's output shapes for its input shapes, in the order its inputs are connected. Every op built so far has
/// one output. Sizes are worked out with checked arithmetic, so a hostile file gets a finding, never a panic.
pub fn infer_op_output_shapes(
    attributes: &OpAttributes,
    input_shapes: &[Vec<u64>],
) -> Result<Vec<Vec<u64>>, ShapeProblem> {
    let Some(first) = input_shapes.first() else {
        return Err(problem(0, "an input".to_string()));
    };
    let output = match attributes {
        OpAttributes::Embedding(embedding) => embedding_shape(embedding, first)?,
        OpAttributes::RmsNorm(norm) => last_axis_shape(first, norm.size, norm.size)?,
        OpAttributes::LayerNorm(norm) => last_axis_shape(first, norm.size, norm.size)?,
        OpAttributes::Attention(attention) => attention_shape(attention, first)?,
        OpAttributes::FeedForward(feed_forward) => feed_forward_shape(feed_forward, first)?,
        OpAttributes::ResidualAdd(_) => residual_add_shape(input_shapes)?,
        OpAttributes::Unembed(unembed) => unembed_shape(unembed, first)?,
        OpAttributes::Conv1d(conv) => conv1d_shape(conv, first)?,
        OpAttributes::Pool(pool) => pool_shape(pool, first)?,
        OpAttributes::Linear(linear) => linear_shape(linear, first)?,
        OpAttributes::ClassifierHead(head) => classifier_head_shape(head, first)?,
        // Elementwise: the shape goes through unchanged.
        OpAttributes::Dropout(_) | OpAttributes::Activation(_) => first.clone(),
        OpAttributes::Concat(concat) => concat_shape(concat, input_shapes)?,
    };
    Ok(vec![output])
}

/// Token ids of any shape, each becoming a vector.
fn embedding_shape(embedding: &EmbeddingAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    if input.is_empty() {
        return Err(problem(0, "[batch, length]".to_string()));
    }
    let mut output = input.to_vec();
    output.push(embedding.embedding_size);
    Ok(output)
}

/// Self-attention keeps the sequence shape `[batch, length, hidden_size]`: the sequence must be three axes long.
fn attention_shape(attention: &AttentionAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    let expected = format!("[batch, length, {}]", attention.hidden_size);
    let [batch, length, hidden] = input else {
        return Err(problem(0, expected));
    };
    if *hidden != attention.hidden_size {
        return Err(problem(0, expected));
    }
    Ok(vec![*batch, *length, *hidden])
}

/// `[..., hidden_size]` to the same shape.
fn feed_forward_shape(feed_forward: &FeedForwardAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    last_axis_shape(input, feed_forward.hidden_size, feed_forward.hidden_size)
}

/// `[..., hidden_size]` to `[..., vocab_size]`: the logits.
fn unembed_shape(unembed: &UnembedAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    last_axis_shape(input, unembed.hidden_size, unembed.vocab_size)
}

/// Two inputs of one shape, which the sum keeps: the second must match the first, and the first is the output.
fn residual_add_shape(inputs: &[Vec<u64>]) -> Result<Vec<u64>, ShapeProblem> {
    let first = &inputs[0];
    match inputs.get(1) {
        Some(second) if second == first => Ok(first.clone()),
        _ => Err(problem(1, shape_text(first))),
    }
}

/// `[batch, length, in_channels]` to `[batch, out_length, out_channels]`, with PyTorch's length rule:
/// `out_length = (length + 2 * padding - dilation * (kernel_size - 1) - 1) / stride + 1`, rounded down.
fn conv1d_shape(conv: &Conv1dAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    let fitting = || problem(0, EXPECTED_FITTING_SIZES.to_string());
    // The window's span over the padded length: its first tap to its last.
    let span = conv
        .dilation
        .checked_mul(conv.kernel_size.saturating_sub(1))
        .and_then(|taps| taps.checked_add(1))
        .ok_or_else(fitting)?;
    if conv.stride == 0 {
        return Err(problem(0, EXPECTED_MOVING_WINDOW.to_string()));
    }
    let both_paddings = conv.padding.checked_mul(2).ok_or_else(fitting)?;
    let shortest_length = span.saturating_sub(both_paddings).max(1);
    let expected = sequence_text(shortest_length, conv.in_channels);
    let [batch, length, channels] = input else {
        return Err(problem(0, expected));
    };
    if *channels != conv.in_channels || *length < shortest_length {
        return Err(problem(0, expected));
    }
    let padded_length = length.checked_add(both_paddings).ok_or_else(fitting)?;
    let out_length = (padded_length - span) / conv.stride + 1;
    Ok(vec![*batch, out_length, conv.out_channels])
}

/// `[batch, length, channels]` to `[batch, channels]` over the whole length, or to `[batch, out_length, channels]`
/// over windows: `out_length = (length - kernel_size) / stride + 1`, rounded down.
fn pool_shape(pool: &PoolAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    let shortest_length = pool.kernel_size.unwrap_or(1);
    let [batch, length, channels] = input else {
        return Err(problem(0, sequence_text(shortest_length, 0)));
    };
    let Some(kernel_size) = pool.kernel_size else {
        return Ok(vec![*batch, *channels]);
    };
    if *length < kernel_size {
        return Err(problem(0, sequence_text(shortest_length, *channels)));
    }
    let stride = pool.stride.unwrap_or(kernel_size);
    if stride == 0 {
        return Err(problem(0, EXPECTED_MOVING_WINDOW.to_string()));
    }
    let out_length = (length - kernel_size) / stride + 1;
    Ok(vec![*batch, out_length, *channels])
}

fn linear_shape(linear: &LinearAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    last_axis_shape(input, linear.in_features, linear.out_features)
}

/// A classifier head is a linear layer to its classes.
fn classifier_head_shape(head: &ClassifierHeadAttributes, input: &[u64]) -> Result<Vec<u64>, ShapeProblem> {
    last_axis_shape(input, head.in_features, head.class_count)
}

/// `[..., in_size]` to `[..., out_size]`.
fn last_axis_shape(input: &[u64], in_size: u64, out_size: u64) -> Result<Vec<u64>, ShapeProblem> {
    let Some((last, leading)) = input.split_last() else {
        return Err(problem(0, format!("[..., {in_size}]")));
    };
    if *last != in_size {
        return Err(problem(0, format!("[..., {in_size}]")));
    }
    let mut output = leading.to_vec();
    output.push(out_size);
    Ok(output)
}

/// Every input has the first one's rank and sizes, except along the axis, where the sizes add up.
fn concat_shape(concat: &ConcatAttributes, inputs: &[Vec<u64>]) -> Result<Vec<u64>, ShapeProblem> {
    let first = &inputs[0];
    let Some(axis) = resolved_axis(concat.axis, first.len()) else {
        let needed_rank = if concat.axis < 0 {
            concat.axis.unsigned_abs()
        } else {
            concat.axis.unsigned_abs() + 1
        };
        return Err(problem(
            0,
            format!("a shape with {needed_rank} axes or more, for axis {}", concat.axis),
        ));
    };
    let mut output = first.clone();
    for (input_index, input) in inputs.iter().enumerate().skip(1) {
        if !agrees_except_axis(first, input, axis) {
            return Err(problem(input_index, concat_text(first, axis)));
        }
        output[axis] = output[axis]
            .checked_add(input[axis])
            .ok_or_else(|| problem(input_index, EXPECTED_FITTING_SIZES.to_string()))?;
    }
    Ok(output)
}

/// A concat axis as an index into a shape of this rank: -1 is the last axis. None when the shape has no such axis.
fn resolved_axis(axis: i64, rank: usize) -> Option<usize> {
    let rank = i64::try_from(rank).ok()?;
    let index = if axis < 0 { rank.checked_add(axis)? } else { axis };
    if (0..rank).contains(&index) {
        return usize::try_from(index).ok();
    }
    None
}

fn agrees_except_axis(first: &[u64], other: &[u64], axis: usize) -> bool {
    first.len() == other.len()
        && first
            .iter()
            .zip(other)
            .enumerate()
            .all(|(index, (left, right))| index == axis || left == right)
}

fn problem(input_index: usize, expected: String) -> ShapeProblem {
    ShapeProblem { input_index, expected }
}

/// A shape as a finding shows it: `[64, 128, 128]`.
pub fn shape_text(shape: &[u64]) -> String {
    let sizes: Vec<String> = shape.iter().map(|size| size.to_string()).collect();
    format!("[{}]", sizes.join(", "))
}

/// The sequence shape an op wants: `[batch, length, 128]`, or `[batch, length of 5 or more, 128]` when a window
/// needs a minimum length. Channels of 0 mean any.
fn sequence_text(shortest_length: u64, channels: u64) -> String {
    let length = if shortest_length > 1 {
        format!("length of {shortest_length} or more")
    } else {
        "length".to_string()
    };
    let channels = if channels == 0 {
        "channels".to_string()
    } else {
        channels.to_string()
    };
    format!("[batch, {length}, {channels}]")
}

/// The first input's shape with `any` along the concat axis: what every other input must look like.
fn concat_text(first: &[u64], axis: usize) -> String {
    let sizes: Vec<String> = first
        .iter()
        .enumerate()
        .map(|(index, size)| {
            if index == axis {
                "any".to_string()
            } else {
                size.to_string()
            }
        })
        .collect();
    format!("[{}]", sizes.join(", "))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::documents::model_ops::{ActivationAttributes, ActivationFunction, AttentionVariant, PoolMode};

    fn conv(kernel_size: u64, padding: u64) -> OpAttributes {
        OpAttributes::Conv1d(Conv1dAttributes {
            in_channels: 64,
            out_channels: 64,
            kernel_size,
            stride: 1,
            padding,
            dilation: 1,
            bias: true,
        })
    }

    fn output(attributes: &OpAttributes, inputs: &[Vec<u64>]) -> Vec<u64> {
        infer_op_output_shapes(attributes, inputs).unwrap().pop().unwrap()
    }

    #[test]
    fn an_even_kernel_with_half_padding_adds_one_position() {
        // TextCNN's kernel 4 with padding 2, as the owner's model has it: PyTorch gives length + 1.
        assert_eq!(output(&conv(4, 2), &[vec![2, 128, 64]]), vec![2, 129, 64]);
        assert_eq!(output(&conv(3, 1), &[vec![2, 128, 64]]), vec![2, 128, 64]);
    }

    #[test]
    fn a_conv_on_the_wrong_channels_or_too_short_a_length_says_what_it_wants() {
        let wrong_channels = infer_op_output_shapes(&conv(3, 0), &[vec![2, 10, 32]]).unwrap_err();
        assert_eq!(wrong_channels.expected, "[batch, length of 3 or more, 64]");
        assert!(infer_op_output_shapes(&conv(3, 0), &[vec![2, 2, 64]]).is_err());
        let wrong_rank = infer_op_output_shapes(&conv(1, 0), &[vec![2, 64]]).unwrap_err();
        assert_eq!(wrong_rank.expected, "[batch, length, 64]");
    }

    #[test]
    fn huge_sizes_are_a_problem_not_a_panic() {
        let huge = OpAttributes::Conv1d(Conv1dAttributes {
            in_channels: 1,
            out_channels: 1,
            kernel_size: u64::MAX,
            stride: 1,
            padding: u64::MAX,
            dilation: u64::MAX,
            bias: false,
        });
        assert!(infer_op_output_shapes(&huge, &[vec![1, 1, 1]]).is_err());
        let concat = OpAttributes::Concat(ConcatAttributes { axis: i64::MIN });
        assert!(infer_op_output_shapes(&concat, &[vec![1, 1]]).is_err());
        let sums = OpAttributes::Concat(ConcatAttributes { axis: 0 });
        assert!(infer_op_output_shapes(&sums, &[vec![u64::MAX], vec![1]]).is_err());
    }

    #[test]
    fn global_pooling_removes_the_length_axis() {
        let global = OpAttributes::Pool(PoolAttributes {
            mode: PoolMode::Max,
            kernel_size: None,
            stride: None,
        });
        assert_eq!(output(&global, &[vec![2, 130, 64]]), vec![2, 64]);
    }

    #[test]
    fn concat_names_the_input_that_disagrees() {
        let concat = OpAttributes::Concat(ConcatAttributes { axis: -1 });
        assert_eq!(output(&concat, &[vec![2, 8], vec![2, 4], vec![2, 1]]), vec![2, 13]);
        let disagreeing = infer_op_output_shapes(&concat, &[vec![2, 8], vec![3, 4]]).unwrap_err();
        assert_eq!(disagreeing, problem(1, "[2, any]".to_string()));
        let too_few_axes = infer_op_output_shapes(&OpAttributes::Concat(ConcatAttributes { axis: -3 }), &[vec![2, 8]]);
        assert_eq!(
            too_few_axes.unwrap_err().expected,
            "a shape with 3 axes or more, for axis -3"
        );
    }

    #[test]
    fn elementwise_ops_keep_the_shape_and_heads_change_the_last_axis() {
        let relu = OpAttributes::Activation(ActivationAttributes {
            function: ActivationFunction::Relu,
        });
        assert_eq!(output(&relu, &[vec![2, 7, 3]]), vec![2, 7, 3]);
        let head = OpAttributes::ClassifierHead(ClassifierHeadAttributes {
            in_features: 64,
            class_count: 4,
            bias: true,
        });
        assert_eq!(output(&head, &[vec![2, 64]]), vec![2, 4]);
        assert_eq!(
            infer_op_output_shapes(&head, &[vec![2, 32]]).unwrap_err().expected,
            "[..., 64]"
        );
    }

    #[test]
    fn shapes_read_as_lists() {
        assert_eq!(shape_text(&[64, 128, 128]), "[64, 128, 128]");
        assert_eq!(shape_text(&[]), "[]");
    }

    #[test]
    fn a_residual_sum_needs_two_inputs_of_one_shape() {
        let same = vec![vec![2, 3, 8], vec![2, 3, 8]];
        assert_eq!(residual_add_shape(&same), Ok(vec![2, 3, 8]));
        let narrower = vec![vec![2, 3, 8], vec![2, 3, 4]];
        assert_eq!(residual_add_shape(&narrower), Err(problem(1, "[2, 3, 8]".to_string())));
        let one_input = vec![vec![2, 3, 8]];
        assert_eq!(residual_add_shape(&one_input), Err(problem(1, "[2, 3, 8]".to_string())));
    }

    #[test]
    fn attention_wants_three_axes_of_its_hidden_size() {
        let attention = AttentionAttributes {
            variant: AttentionVariant::Mha,
            hidden_size: 16,
            head_count: 2,
            kv_head_count: None,
            head_dim: 8,
            window: None,
            qk_norm: false,
            rope: None,
        };
        assert_eq!(attention_shape(&attention, &[1, 6, 16]), Ok(vec![1, 6, 16]));
        assert_eq!(
            attention_shape(&attention, &[6, 16]),
            Err(problem(0, "[batch, length, 16]".to_string()))
        );
        assert_eq!(
            attention_shape(&attention, &[1, 6, 12]),
            Err(problem(0, "[batch, length, 16]".to_string()))
        );
    }
}
