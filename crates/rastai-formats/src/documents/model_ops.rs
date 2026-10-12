//! The ModelSpec op vocabulary (contracts 5.3) and the attributes of each op whose shapes are built.
//!
//! A graph node's `attributes` are free JSON in `rastai.model/1`, because which attributes it takes depends on its
//! `op`. Each op with shape rules has a described attributes type here; layer 3 (validate/meaning/model_graph.rs)
//! walks the node's attributes against it with layer 2's walker, then reads them into the typed struct.
//!
//! M1 builds the ops TextCNN needs. Sequence tensors are channels-last everywhere: `[batch, length, channels]`, the
//! layout an embedding gives, so `conv1d` and `pool` read it the same way and no transpose op is needed; the Python
//! compiler transposes inside its `conv1d` and `pool` modules. The other ops of the vocabulary get their attributes
//! with the contracts that build them (M2-01 for the decoder families); until then layer 3 checks only that they
//! are in the vocabulary, and shapes are not worked out past them.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, type_description};
use crate::documents::model::MODEL_SCHEMA_ID;

/// Every op of contracts 5.3, in its order.
pub const MODEL_OPS: &[&str] = &[
    "embedding",
    "rms_norm",
    "layer_norm",
    "linear",
    "attention",
    "rope",
    "alibi",
    "feed_forward",
    "moe_router",
    "moe_expert",
    "moe",
    "residual_add",
    "attn_res",
    "unembed",
    "conv1d",
    "conv2d",
    "pool",
    "rnn",
    "dropout",
    "activation",
    "concat",
    "cross_attention",
    "fusion",
    "classifier_head",
    "block",
    "opaque",
];

/// The op that may close a loop in the graph: a cycle through it is recurrence, any other cycle is an error.
pub const RECURRENT_OP: &str = "rnn";

/// The ops whose attributes and shapes are built, each with its golden file `fixtures/ops/<op>.json`.
pub const OPS_WITH_SHAPES: &[&str] = &[
    "embedding",
    "conv1d",
    "pool",
    "linear",
    "dropout",
    "activation",
    "concat",
    "classifier_head",
];

/// The one op that takes several inputs, in `to_input` order; every other op with shapes takes exactly one.
pub const MANY_INPUTS_OP: &str = "concat";

// The described attribute types, by op.
const EMBEDDING_NAME: &str = "EmbeddingAttributes";
const CONV1D_NAME: &str = "Conv1dAttributes";
const POOL_NAME: &str = "PoolAttributes";
const LINEAR_NAME: &str = "LinearAttributes";
const DROPOUT_NAME: &str = "DropoutAttributes";
const ACTIVATION_NAME: &str = "ActivationAttributes";
const CONCAT_NAME: &str = "ConcatAttributes";
const CLASSIFIER_HEAD_NAME: &str = "ClassifierHeadAttributes";

// The enum values, as serde writes `PoolMode` and `ActivationFunction`.
const POOL_MODE_VALUES: &[&str] = &["max", "mean"];
const ACTIVATION_FUNCTION_VALUES: &[&str] = &["relu", "gelu", "silu", "tanh", "sigmoid"];

/// Concatenation joins along the last axis unless told otherwise: channels or features, as TextCNN's branches need.
const DEFAULT_CONCAT_AXIS: i64 = -1;
/// A classifier needs at least two classes for its cross-entropy objective to mean anything.
const MIN_CLASS_COUNT: u64 = 2;

// What a value problem says the value should be.
const EXPECTED_POSITIVE: &str = "a whole number of 1 or more";
const EXPECTED_CLASS_COUNT: &str = "a whole number of 2 or more: a classifier needs two classes at least";
const EXPECTED_PROBABILITY: &str = "a probability from 0 up to, but not including, 1";

/// Token ids to vectors: `[...]` to `[..., embedding_size]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingAttributes {
    pub vocab_size: u64,
    pub embedding_size: u64,
}

/// A 1D convolution over the length axis: `[batch, length, in_channels]` to `[batch, out_length, out_channels]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conv1dAttributes {
    pub in_channels: u64,
    pub out_channels: u64,
    pub kernel_size: u64,
    #[serde(default = "one")]
    pub stride: u64,
    /// Zeros added at each end of the length axis.
    #[serde(default)]
    pub padding: u64,
    #[serde(default = "one")]
    pub dilation: u64,
    #[serde(default = "enabled")]
    pub bias: bool,
}

/// Max or mean pooling over the length axis. Without `kernel_size` it pools the whole length and removes that axis:
/// `[batch, length, channels]` to `[batch, channels]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolAttributes {
    pub mode: PoolMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_size: Option<u64>,
    /// The window's step; the window's length when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stride: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolMode {
    Max,
    Mean,
}

/// `[..., in_features]` to `[..., out_features]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearAttributes {
    pub in_features: u64,
    pub out_features: u64,
    #[serde(default = "enabled")]
    pub bias: bool,
}

/// Zeroes each element with this probability while training; the shape is unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropoutAttributes {
    pub probability: f64,
}

/// An elementwise function; the shape is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationAttributes {
    pub function: ActivationFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationFunction {
    Relu,
    Gelu,
    Silu,
    Tanh,
    Sigmoid,
}

/// Joins its inputs, in `to_input` order, along one axis; the other axes must agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcatAttributes {
    /// Counted from the end when negative: -1 is the last axis.
    #[serde(default = "last_axis")]
    pub axis: i64,
}

/// The logits for a classification objective: `[..., in_features]` to `[..., class_count]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifierHeadAttributes {
    pub in_features: u64,
    pub class_count: u64,
    #[serde(default = "enabled")]
    pub bias: bool,
}

/// A node's attributes as the typed struct of its op.
#[derive(Debug, Clone, PartialEq)]
pub enum OpAttributes {
    Embedding(EmbeddingAttributes),
    Conv1d(Conv1dAttributes),
    Pool(PoolAttributes),
    Linear(LinearAttributes),
    Dropout(DropoutAttributes),
    Activation(ActivationAttributes),
    Concat(ConcatAttributes),
    ClassifierHead(ClassifierHeadAttributes),
}

/// One attribute whose value an op can't use, and what it should be (`RRS-E2006`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeProblem {
    pub attribute: &'static str,
    pub expected: &'static str,
}

fn one() -> u64 {
    1
}

fn enabled() -> bool {
    true
}

fn last_axis() -> i64 {
    DEFAULT_CONCAT_AXIS
}

/// The described attributes type of an op, or None for an op whose attributes aren't built yet.
pub fn op_attributes_name(op: &str) -> Option<&'static str> {
    let name = match op {
        "embedding" => EMBEDDING_NAME,
        "conv1d" => CONV1D_NAME,
        "pool" => POOL_NAME,
        "linear" => LINEAR_NAME,
        "dropout" => DROPOUT_NAME,
        "activation" => ACTIVATION_NAME,
        "concat" => CONCAT_NAME,
        "classifier_head" => CLASSIFIER_HEAD_NAME,
        _ => return None,
    };
    Some(name)
}

/// The typed attributes for attributes that passed the walk against the op's description. The error is serde's
/// reason: a size below zero or past 64 bits, which the walk's `Integer` kind lets through. None when the op has no
/// attributes type.
pub fn op_attributes_from_value(
    op: &str,
    attributes: &BTreeMap<String, Value>,
) -> Option<Result<OpAttributes, String>> {
    let value = Value::Object(attributes.clone().into_iter().collect());
    let typed = match op {
        "embedding" => typed(value).map(OpAttributes::Embedding),
        "conv1d" => typed(value).map(OpAttributes::Conv1d),
        "pool" => typed(value).map(OpAttributes::Pool),
        "linear" => typed(value).map(OpAttributes::Linear),
        "dropout" => typed(value).map(OpAttributes::Dropout),
        "activation" => typed(value).map(OpAttributes::Activation),
        "concat" => typed(value).map(OpAttributes::Concat),
        "classifier_head" => typed(value).map(OpAttributes::ClassifierHead),
        _ => return None,
    };
    Some(typed)
}

fn typed<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// Every attribute whose value is of the right kind but can't be used: a size of 0, a probability of 1 or more.
pub fn op_attribute_problems(attributes: &OpAttributes) -> Vec<AttributeProblem> {
    let mut problems = Vec::new();
    match attributes {
        OpAttributes::Embedding(embedding) => {
            require_positive(&mut problems, "vocab_size", embedding.vocab_size);
            require_positive(&mut problems, "embedding_size", embedding.embedding_size);
        }
        OpAttributes::Conv1d(conv) => {
            require_positive(&mut problems, "in_channels", conv.in_channels);
            require_positive(&mut problems, "out_channels", conv.out_channels);
            require_positive(&mut problems, "kernel_size", conv.kernel_size);
            require_positive(&mut problems, "stride", conv.stride);
            require_positive(&mut problems, "dilation", conv.dilation);
        }
        OpAttributes::Pool(pool) => {
            if let Some(kernel_size) = pool.kernel_size {
                require_positive(&mut problems, "kernel_size", kernel_size);
            }
            if let Some(stride) = pool.stride {
                require_positive(&mut problems, "stride", stride);
            }
        }
        OpAttributes::Linear(linear) => {
            require_positive(&mut problems, "in_features", linear.in_features);
            require_positive(&mut problems, "out_features", linear.out_features);
        }
        OpAttributes::Dropout(dropout) => {
            let probability = dropout.probability;
            if !(0.0..1.0).contains(&probability) {
                problems.push(problem("probability", EXPECTED_PROBABILITY));
            }
        }
        OpAttributes::ClassifierHead(head) => {
            require_positive(&mut problems, "in_features", head.in_features);
            if head.class_count < MIN_CLASS_COUNT {
                problems.push(problem("class_count", EXPECTED_CLASS_COUNT));
            }
        }
        // Any function is fine; the axis is checked against the inputs' rank with the shapes.
        OpAttributes::Activation(_) | OpAttributes::Concat(_) => {}
    }
    problems
}

fn require_positive(problems: &mut Vec<AttributeProblem>, attribute: &'static str, value: u64) {
    if value == 0 {
        problems.push(problem(attribute, EXPECTED_POSITIVE));
    }
}

fn problem(attribute: &'static str, expected: &'static str) -> AttributeProblem {
    AttributeProblem { attribute, expected }
}

/// Every op attributes type, for the registry (describe/registry.rs). They share the ModelSpec's schema id, so the
/// generated JavaScript and Python put them beside it.
pub fn describe_model_op_types() -> Vec<TypeDescription> {
    vec![
        describe_embedding(),
        describe_conv1d(),
        describe_pool(),
        describe_linear(),
        describe_dropout(),
        describe_activation(),
        describe_concat(),
        describe_classifier_head(),
    ]
}

fn describe_embedding() -> TypeDescription {
    type_description(
        EMBEDDING_NAME,
        MODEL_SCHEMA_ID,
        "embedding: token ids [...] to vectors [..., embedding_size].",
        vec![
            field(
                "vocab_size",
                FieldKind::Integer,
                REQUIRED,
                "Token ids it knows, from 0.",
            ),
            field("embedding_size", FieldKind::Integer, REQUIRED, "Length of each vector."),
        ],
    )
}

fn describe_conv1d() -> TypeDescription {
    type_description(
        CONV1D_NAME,
        MODEL_SCHEMA_ID,
        "conv1d over the length axis: [batch, length, in_channels] to [batch, out_length, out_channels].",
        vec![
            field("in_channels", FieldKind::Integer, REQUIRED, "Channels of its input."),
            field("out_channels", FieldKind::Integer, REQUIRED, "Channels of its output."),
            field("kernel_size", FieldKind::Integer, REQUIRED, "Window length."),
            field("stride", FieldKind::Integer, OPTIONAL, "Window step; 1 by default."),
            field(
                "padding",
                FieldKind::Integer,
                OPTIONAL,
                "Zeros added at each end of the length; 0 by default.",
            ),
            field(
                "dilation",
                FieldKind::Integer,
                OPTIONAL,
                "Spacing of the window's taps; 1 by default.",
            ),
            field(
                "bias",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether it adds a bias; true by default.",
            ),
        ],
    )
}

fn describe_pool() -> TypeDescription {
    type_description(
        POOL_NAME,
        MODEL_SCHEMA_ID,
        "pool over the length axis; without kernel_size, over the whole length: [batch, length, channels] to \
         [batch, channels].",
        vec![
            field("mode", enum_of(POOL_MODE_VALUES), REQUIRED, "max or mean."),
            field(
                "kernel_size",
                FieldKind::Integer,
                OPTIONAL,
                "Window length; left out to pool the whole length.",
            ),
            field(
                "stride",
                FieldKind::Integer,
                OPTIONAL,
                "Window step; the window length by default.",
            ),
        ],
    )
}

fn describe_linear() -> TypeDescription {
    type_description(
        LINEAR_NAME,
        MODEL_SCHEMA_ID,
        "linear: [..., in_features] to [..., out_features].",
        vec![
            field(
                "in_features",
                FieldKind::Integer,
                REQUIRED,
                "Size of its input's last axis.",
            ),
            field(
                "out_features",
                FieldKind::Integer,
                REQUIRED,
                "Size of its output's last axis.",
            ),
            field(
                "bias",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether it adds a bias; true by default.",
            ),
        ],
    )
}

fn describe_dropout() -> TypeDescription {
    type_description(
        DROPOUT_NAME,
        MODEL_SCHEMA_ID,
        "dropout: zeroes elements while training; the shape is unchanged.",
        vec![field(
            "probability",
            FieldKind::Number,
            REQUIRED,
            "Chance of zeroing each element, from 0 up to 1.",
        )],
    )
}

fn describe_activation() -> TypeDescription {
    type_description(
        ACTIVATION_NAME,
        MODEL_SCHEMA_ID,
        "activation: an elementwise function; the shape is unchanged.",
        vec![field(
            "function",
            enum_of(ACTIVATION_FUNCTION_VALUES),
            REQUIRED,
            "relu, gelu, silu, tanh or sigmoid.",
        )],
    )
}

fn describe_concat() -> TypeDescription {
    type_description(
        CONCAT_NAME,
        MODEL_SCHEMA_ID,
        "concat: joins its inputs in to_input order along one axis; the other axes must agree.",
        vec![field(
            "axis",
            FieldKind::Integer,
            OPTIONAL,
            "The axis to join along, negative from the end; -1 (the last) by default.",
        )],
    )
}

fn describe_classifier_head() -> TypeDescription {
    type_description(
        CLASSIFIER_HEAD_NAME,
        MODEL_SCHEMA_ID,
        "classifier_head: the logits of a classification objective, [..., in_features] to [..., class_count].",
        vec![
            field(
                "in_features",
                FieldKind::Integer,
                REQUIRED,
                "Size of its input's last axis.",
            ),
            field(
                "class_count",
                FieldKind::Integer,
                REQUIRED,
                "Classes it scores, 2 or more.",
            ),
            field(
                "bias",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether it adds a bias; true by default.",
            ),
        ],
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn attributes_of(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn every_op_with_shapes_is_in_the_vocabulary_and_has_an_attributes_type() {
        for op in OPS_WITH_SHAPES {
            assert!(MODEL_OPS.contains(op), "{op}");
            assert!(op_attributes_name(op).is_some(), "{op}");
        }
        assert!(MODEL_OPS.contains(&RECURRENT_OP) && MODEL_OPS.contains(&MANY_INPUTS_OP));
        assert_eq!(op_attributes_name("attention"), None);
        assert_eq!(MODEL_OPS.len(), 26);
    }

    #[test]
    fn every_described_attributes_type_belongs_to_one_op() {
        let names: Vec<String> = describe_model_op_types().into_iter().map(|item| item.name).collect();
        let by_op: Vec<String> = OPS_WITH_SHAPES
            .iter()
            .map(|op| op_attributes_name(op).unwrap().to_string())
            .collect();
        assert_eq!(names, by_op);
    }

    #[test]
    fn absent_attributes_take_their_defaults() {
        let typed = op_attributes_from_value(
            "conv1d",
            &attributes_of(json!({"in_channels": 8, "out_channels": 4, "kernel_size": 3})),
        );
        let Some(Ok(OpAttributes::Conv1d(conv))) = typed else {
            panic!("{typed:?}");
        };
        assert_eq!((conv.stride, conv.padding, conv.dilation, conv.bias), (1, 0, 1, true));
        let concat = op_attributes_from_value("concat", &BTreeMap::new());
        assert_eq!(concat, Some(Ok(OpAttributes::Concat(ConcatAttributes { axis: -1 }))));
        assert!(op_attributes_from_value("attention", &BTreeMap::new()).is_none());
    }

    #[test]
    fn a_negative_size_is_refused_by_serde() {
        let typed = op_attributes_from_value("linear", &attributes_of(json!({"in_features": -1, "out_features": 2})));
        assert!(matches!(typed, Some(Err(_))));
    }

    #[test]
    fn zero_sizes_one_class_and_a_certain_drop_are_problems() {
        let conv = OpAttributes::Conv1d(Conv1dAttributes {
            in_channels: 0,
            out_channels: 4,
            kernel_size: 0,
            stride: 1,
            padding: 0,
            dilation: 1,
            bias: true,
        });
        let attributes: Vec<&str> = op_attribute_problems(&conv).iter().map(|item| item.attribute).collect();
        assert_eq!(attributes, vec!["in_channels", "kernel_size"]);
        let head = OpAttributes::ClassifierHead(ClassifierHeadAttributes {
            in_features: 4,
            class_count: 1,
            bias: true,
        });
        assert_eq!(op_attribute_problems(&head)[0].attribute, "class_count");
        for probability in [-0.1, 1.0, 1.5] {
            let dropout = OpAttributes::Dropout(DropoutAttributes { probability });
            assert_eq!(op_attribute_problems(&dropout).len(), 1, "{probability}");
        }
        let kept = OpAttributes::Dropout(DropoutAttributes { probability: 0.0 });
        assert!(op_attribute_problems(&kept).is_empty());
    }

    #[test]
    fn enum_values_are_what_serde_writes() {
        let modes: Vec<Value> = [PoolMode::Max, PoolMode::Mean]
            .iter()
            .map(|mode| serde_json::to_value(mode).unwrap())
            .collect();
        assert_eq!(modes, POOL_MODE_VALUES);
        let functions = [
            ActivationFunction::Relu,
            ActivationFunction::Gelu,
            ActivationFunction::Silu,
            ActivationFunction::Tanh,
            ActivationFunction::Sigmoid,
        ];
        let written: Vec<Value> = functions
            .iter()
            .map(|function| serde_json::to_value(function).unwrap())
            .collect();
        assert_eq!(written, ACTIVATION_FUNCTION_VALUES);
    }
}
