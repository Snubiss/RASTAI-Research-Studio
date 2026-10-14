//! The ModelSpec op vocabulary (contracts 5.3) and the attributes of each op whose shapes are built.
//!
//! A graph node's `attributes` are free JSON in `rastai.model/1`, because which attributes it takes depends on its
//! `op`. Each op with shape rules has a described attributes type here; layer 3 (validate/meaning/model_graph.rs)
//! walks the node's attributes against it with layer 2's walker, then reads them into the typed struct.
//!
//! Sequence tensors are channels-last everywhere: `[batch, length, channels]`, the layout an embedding gives, so
//! `conv1d` and `pool` read it the same way and no transpose op is needed; the Python compiler transposes inside its
//! `conv1d` and `pool` modules. The decoder families (M2-01) add their ops here: each has a golden file in
//! `fixtures/ops/`. The rest of the vocabulary is accepted by layer 3 and has no shapes until its contract adds it.

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
    "rms_norm",
    "layer_norm",
    "attention",
    "feed_forward",
    "residual_add",
    "unembed",
    "conv1d",
    "pool",
    "linear",
    "dropout",
    "activation",
    "concat",
    "classifier_head",
];

/// The one op whose number of inputs is the number of things connected to it, in `to_input` order.
pub const MANY_INPUTS_OP: &str = "concat";
/// The one op with exactly two inputs, which must have the same shape: a residual connection's sum.
pub const TWO_INPUTS_OP: &str = "residual_add";

// The described attribute types, by op, and the nested type of attention's rotary settings.
const EMBEDDING_NAME: &str = "EmbeddingAttributes";
const RMS_NORM_NAME: &str = "RmsNormAttributes";
const LAYER_NORM_NAME: &str = "LayerNormAttributes";
const ATTENTION_NAME: &str = "AttentionAttributes";
const ROPE_NAME: &str = "RopeSettings";
const FEED_FORWARD_NAME: &str = "FeedForwardAttributes";
const RESIDUAL_ADD_NAME: &str = "ResidualAddAttributes";
const UNEMBED_NAME: &str = "UnembedAttributes";
const CONV1D_NAME: &str = "Conv1dAttributes";
const POOL_NAME: &str = "PoolAttributes";
const LINEAR_NAME: &str = "LinearAttributes";
const DROPOUT_NAME: &str = "DropoutAttributes";
const ACTIVATION_NAME: &str = "ActivationAttributes";
const CONCAT_NAME: &str = "ConcatAttributes";
const CLASSIFIER_HEAD_NAME: &str = "ClassifierHeadAttributes";

// The enum values, as serde writes each enum.
const POOL_MODE_VALUES: &[&str] = &["max", "mean"];
const ACTIVATION_FUNCTION_VALUES: &[&str] = &["relu", "gelu", "silu", "tanh", "sigmoid"];
const ATTENTION_VARIANT_VALUES: &[&str] = &["mha", "gqa", "sliding"];
const FEED_FORWARD_VARIANT_VALUES: &[&str] = &["dense", "gated"];
const ROPE_SCALING_VALUES: &[&str] = &["none", "linear", "llama3", "yarn"];

/// Concatenation joins along the last axis unless told otherwise: channels or features, as TextCNN's branches need.
const DEFAULT_CONCAT_AXIS: i64 = -1;
/// A classifier needs at least two classes for its cross-entropy objective to mean anything.
const MIN_CLASS_COUNT: u64 = 2;

/// The RMSNorm epsilon of Gemma 3; Llama uses 1e-5 with its own, which a design sets.
const DEFAULT_RMS_EPSILON: f64 = 1e-6;
const DEFAULT_LAYER_EPSILON: f64 = 1e-5;
/// The RoPE base frequency of Llama and Gemma 3's global layers.
const DEFAULT_ROPE_BASE: f64 = 10_000.0;
/// Llama 3's frequency bands: wavelengths longer than the low band's are scaled, shorter than the high band's are
/// not, and the bands between blend. These are its published defaults.
const DEFAULT_LOW_FREQ_FACTOR: f64 = 1.0;
const DEFAULT_HIGH_FREQ_FACTOR: f64 = 4.0;
/// YaRN's ramp bounds in rotations over the original context (the published defaults).
const DEFAULT_BETA_FAST: f64 = 32.0;
const DEFAULT_BETA_SLOW: f64 = 1.0;
const UNIT: f64 = 1.0;
/// The embedding scale of a design that doesn't scale its embedding (Gemma 3 multiplies by sqrt(hidden size)).
const DEFAULT_EMBEDDING_SCALE: f64 = 1.0;

// What a value problem says the value should be.
const EXPECTED_POSITIVE: &str = "a whole number of 1 or more";
const EXPECTED_POSITIVE_NUMBER: &str = "a number above 0";
const EXPECTED_CLASS_COUNT: &str = "a whole number of 2 or more: a classifier needs two classes at least";
const EXPECTED_PROBABILITY: &str = "a probability from 0 up to, but not including, 1";
const EXPECTED_WINDOW: &str = "a window of 1 or more tokens, and only for variant sliding";
const EXPECTED_KV_HEADS: &str = "a head count of 1 or more that divides head_count; for variant gqa, fewer than \
     head_count; for variant mha, left out or equal to head_count";
const EXPECTED_ROPE: &str = "a base above 0, a factor above 0, and original_max_position of 1 or more for llama3 and \
     yarn";

/// Token ids to vectors: `[...]` to `[..., embedding_size]`. `scale` multiplies each vector (Gemma 3 scales by the
/// square root of its size); it changes no shape and no count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingAttributes {
    pub vocab_size: u64,
    pub embedding_size: u64,
    #[serde(default = "default_embedding_scale")]
    pub scale: f64,
}

/// RMS normalization over the last axis: `[..., size]` to `[..., size]`, with one learned scale per element.
/// `scale_offset` adds one to the scale (Gemma 3 stores the scale as an offset from one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RmsNormAttributes {
    pub size: u64,
    #[serde(default)]
    pub scale_offset: bool,
    #[serde(default = "default_rms_epsilon")]
    pub epsilon: f64,
}

/// Layer normalization over the last axis: `[..., size]` to `[..., size]`, with a learned scale and, when `bias` is
/// true, a learned shift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerNormAttributes {
    pub size: u64,
    #[serde(default = "enabled")]
    pub bias: bool,
    #[serde(default = "default_layer_epsilon")]
    pub epsilon: f64,
}

/// The attention variants of contracts 5.3. `mha` has one key and value head per query head; `gqa` groups query
/// heads over fewer key and value heads; `sliding` looks back over `window` tokens only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionVariant {
    Mha,
    Gqa,
    Sliding,
}

/// Self-attention over one sequence: `[batch, length, hidden_size]` to `[batch, length, hidden_size]`.
///
/// The op holds its own projections: query to `head_count * head_dim`, key and value to `kv_head_count * head_dim`
/// (the same as `head_count` when left out), and the output projection back to `hidden_size`. The projections have no
/// bias, which the decoder families don't use. `qk_norm` adds an RMS norm of `head_dim` on queries and on keys.
/// `rope` rotates queries and keys by position; it is part of attention because it acts after the projections and
/// on the query and key heads only. `window` is the number of tokens a sliding layer looks back over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionAttributes {
    pub variant: AttentionVariant,
    pub hidden_size: u64,
    pub head_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_head_count: Option<u64>,
    pub head_dim: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    #[serde(default)]
    pub qk_norm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rope: Option<RopeSettings>,
}

/// How rotary positions are scaled for a context longer than the one the model was trained on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RopeScaling {
    /// The frequencies as the base gives them. Not `None`: that name reads as Option's absent value next to an
    /// attention's `rope`. Its serde value is still `none`.
    #[default]
    #[serde(rename = "none")]
    NoScaling,
    /// Every frequency divided by `factor`.
    Linear,
    /// Llama 3's per-band scaling by `factor`, with the frequency bands of the two factors below.
    Llama3,
    /// YaRN: interpolation and extrapolation blended per band, with the ramp bounds below.
    Yarn,
}

/// The rotary position embedding of an attention op: its base frequency and how it is scaled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RopeSettings {
    #[serde(default = "default_rope_base")]
    pub base: f64,
    #[serde(default)]
    pub scaling: RopeScaling,
    #[serde(default = "unit")]
    pub factor: f64,
    /// The context length the model was trained on; llama3 and yarn need it.
    #[serde(default)]
    pub original_max_position: u64,
    #[serde(default = "default_low_freq_factor")]
    pub low_freq_factor: f64,
    #[serde(default = "default_high_freq_factor")]
    pub high_freq_factor: f64,
    #[serde(default = "default_beta_fast")]
    pub beta_fast: f64,
    #[serde(default = "default_beta_slow")]
    pub beta_slow: f64,
}

/// The two feed-forward variants of contracts 5.3: `dense` is up, activation, down; `gated` is the gated linear unit,
/// which multiplies the activated gate projection by the up projection before the down projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedForwardVariant {
    Dense,
    Gated,
}

/// A position-wise feed-forward block: `[..., hidden_size]` to `[..., hidden_size]`, through `intermediate_size`.
/// Like attention it has no biases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedForwardAttributes {
    pub variant: FeedForwardVariant,
    pub hidden_size: u64,
    pub intermediate_size: u64,
    pub activation: ActivationFunction,
}

/// The sum of two inputs of the same shape: a residual connection. It has no attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidualAddAttributes {}

/// The output projection to the vocabulary: `[..., hidden_size]` to `[..., vocab_size]`. `tied` shares the weight
/// with the embedding (so the op holds no parameters of its own, but still does its matrix multiplication).
/// `final_softcap` caps the logits: `cap * tanh(logit / cap)`, which changes no shape and no count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnembedAttributes {
    pub hidden_size: u64,
    pub vocab_size: u64,
    #[serde(default)]
    pub tied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_softcap: Option<f64>,
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
    RmsNorm(RmsNormAttributes),
    LayerNorm(LayerNormAttributes),
    Attention(AttentionAttributes),
    FeedForward(FeedForwardAttributes),
    ResidualAdd(ResidualAddAttributes),
    Unembed(UnembedAttributes),
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

fn unit() -> f64 {
    UNIT
}

fn enabled() -> bool {
    true
}

fn last_axis() -> i64 {
    DEFAULT_CONCAT_AXIS
}

fn default_embedding_scale() -> f64 {
    DEFAULT_EMBEDDING_SCALE
}

fn default_rms_epsilon() -> f64 {
    DEFAULT_RMS_EPSILON
}

fn default_layer_epsilon() -> f64 {
    DEFAULT_LAYER_EPSILON
}

fn default_rope_base() -> f64 {
    DEFAULT_ROPE_BASE
}

fn default_low_freq_factor() -> f64 {
    DEFAULT_LOW_FREQ_FACTOR
}

fn default_high_freq_factor() -> f64 {
    DEFAULT_HIGH_FREQ_FACTOR
}

fn default_beta_fast() -> f64 {
    DEFAULT_BETA_FAST
}

fn default_beta_slow() -> f64 {
    DEFAULT_BETA_SLOW
}

/// The described attributes type of an op, or None for an op whose attributes aren't built yet.
pub fn op_attributes_name(op: &str) -> Option<&'static str> {
    let name = match op {
        "embedding" => EMBEDDING_NAME,
        "rms_norm" => RMS_NORM_NAME,
        "layer_norm" => LAYER_NORM_NAME,
        "attention" => ATTENTION_NAME,
        "feed_forward" => FEED_FORWARD_NAME,
        "residual_add" => RESIDUAL_ADD_NAME,
        "unembed" => UNEMBED_NAME,
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

/// How many inputs an op takes when the number is fixed. `concat` takes as many as are connected, so it has none.
pub fn fixed_input_count(op: &str) -> usize {
    if op == TWO_INPUTS_OP { 2 } else { 1 }
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
        "rms_norm" => typed(value).map(OpAttributes::RmsNorm),
        "layer_norm" => typed(value).map(OpAttributes::LayerNorm),
        "attention" => typed(value).map(OpAttributes::Attention),
        "feed_forward" => typed(value).map(OpAttributes::FeedForward),
        "residual_add" => typed(value).map(OpAttributes::ResidualAdd),
        "unembed" => typed(value).map(OpAttributes::Unembed),
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

/// Every attribute whose value is of the right kind but can't be used: a size of 0, a probability of 1 or more, an
/// attention whose head counts or window don't fit its variant.
pub fn op_attribute_problems(attributes: &OpAttributes) -> Vec<AttributeProblem> {
    let mut problems = Vec::new();
    match attributes {
        OpAttributes::Embedding(embedding) => check_embedding(&mut problems, embedding),
        OpAttributes::RmsNorm(norm) => {
            require_positive(&mut problems, "size", norm.size);
            require_positive_epsilon(&mut problems, norm.epsilon);
        }
        OpAttributes::LayerNorm(norm) => {
            require_positive(&mut problems, "size", norm.size);
            require_positive_epsilon(&mut problems, norm.epsilon);
        }
        OpAttributes::Attention(attention) => check_attention(&mut problems, attention),
        OpAttributes::FeedForward(feed_forward) => {
            require_positive(&mut problems, "hidden_size", feed_forward.hidden_size);
            require_positive(&mut problems, "intermediate_size", feed_forward.intermediate_size);
        }
        // No attributes, so no value can be wrong.
        OpAttributes::ResidualAdd(_) => {}
        OpAttributes::Unembed(unembed) => check_unembed(&mut problems, unembed),
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

fn check_embedding(problems: &mut Vec<AttributeProblem>, embedding: &EmbeddingAttributes) {
    require_positive(problems, "vocab_size", embedding.vocab_size);
    require_positive(problems, "embedding_size", embedding.embedding_size);
    if !above_zero(embedding.scale) {
        problems.push(problem("scale", EXPECTED_POSITIVE_NUMBER));
    }
}

fn check_unembed(problems: &mut Vec<AttributeProblem>, unembed: &UnembedAttributes) {
    require_positive(problems, "hidden_size", unembed.hidden_size);
    require_positive(problems, "vocab_size", unembed.vocab_size);
    if unembed.final_softcap.is_some_and(|cap| !above_zero(cap)) {
        problems.push(problem("final_softcap", EXPECTED_POSITIVE_NUMBER));
    }
}

/// The head counts, window and rotary settings of an attention op, checked against its variant. `kv_head_count`
/// left out means one key and value head per query head.
fn check_attention(problems: &mut Vec<AttributeProblem>, attention: &AttentionAttributes) {
    require_positive(problems, "hidden_size", attention.hidden_size);
    require_positive(problems, "head_count", attention.head_count);
    require_positive(problems, "head_dim", attention.head_dim);
    // A head count of 0 is reported once, at head_count: no key or value head count can be checked against it.
    if attention.head_count > 0 && !kv_heads_fit(attention) {
        problems.push(problem("kv_head_count", EXPECTED_KV_HEADS));
    }
    let window_fits = match attention.variant {
        AttentionVariant::Sliding => attention.window.is_some_and(|window| window > 0),
        AttentionVariant::Mha | AttentionVariant::Gqa => attention.window.is_none(),
    };
    if !window_fits {
        problems.push(problem("window", EXPECTED_WINDOW));
    }
    if attention.rope.as_ref().is_some_and(|rope| !rope_fits(rope)) {
        problems.push(problem("rope", EXPECTED_ROPE));
    }
}

/// Whether the key and value head count suits the variant. A left-out count means one per query head.
fn kv_heads_fit(attention: &AttentionAttributes) -> bool {
    let kv_head_count = attention.kv_head_count.unwrap_or(attention.head_count);
    let divides = kv_head_count > 0 && attention.head_count.is_multiple_of(kv_head_count);
    let shares_heads = kv_head_count < attention.head_count;
    match attention.variant {
        AttentionVariant::Mha => kv_head_count == attention.head_count,
        AttentionVariant::Gqa => divides && shares_heads,
        AttentionVariant::Sliding => divides,
    }
}

fn rope_fits(rope: &RopeSettings) -> bool {
    let scaled_with_length = match rope.scaling {
        RopeScaling::Llama3 | RopeScaling::Yarn => rope.original_max_position > 0,
        RopeScaling::NoScaling | RopeScaling::Linear => true,
    };
    above_zero(rope.base) && above_zero(rope.factor) && scaled_with_length
}

/// False for 0, a negative number and NaN, which a comparison with 0 can't say is positive.
fn above_zero(value: f64) -> bool {
    value > 0.0
}

fn require_positive(problems: &mut Vec<AttributeProblem>, attribute: &'static str, value: u64) {
    if value == 0 {
        problems.push(problem(attribute, EXPECTED_POSITIVE));
    }
}

fn require_positive_epsilon(problems: &mut Vec<AttributeProblem>, epsilon: f64) {
    if !above_zero(epsilon) {
        problems.push(problem("epsilon", EXPECTED_POSITIVE_NUMBER));
    }
}

fn problem(attribute: &'static str, expected: &'static str) -> AttributeProblem {
    AttributeProblem { attribute, expected }
}

/// Every op attributes type, for the registry (describe/registry.rs), with the nested rotary settings last. They
/// share the ModelSpec's schema id, so the generated JavaScript and Python put them beside it.
pub fn describe_model_op_types() -> Vec<TypeDescription> {
    vec![
        describe_embedding(),
        describe_rms_norm(),
        describe_layer_norm(),
        describe_attention(),
        describe_feed_forward(),
        describe_residual_add(),
        describe_unembed(),
        describe_conv1d(),
        describe_pool(),
        describe_linear(),
        describe_dropout(),
        describe_activation(),
        describe_concat(),
        describe_classifier_head(),
        describe_rope(),
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
            field(
                "scale",
                FieldKind::Number,
                OPTIONAL,
                "Multiplier of each vector; 1 by default. Gemma 3 uses the square root of embedding_size.",
            ),
        ],
    )
}

fn describe_rms_norm() -> TypeDescription {
    type_description(
        RMS_NORM_NAME,
        MODEL_SCHEMA_ID,
        "rms_norm over the last axis: [..., size] to [..., size].",
        vec![
            field("size", FieldKind::Integer, REQUIRED, "Size of its last axis."),
            field(
                "scale_offset",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether the learned scale is added to one (Gemma 3); false by default.",
            ),
            field(
                "epsilon",
                FieldKind::Number,
                OPTIONAL,
                "Added under the square root; 1e-6 by default.",
            ),
        ],
    )
}

fn describe_layer_norm() -> TypeDescription {
    type_description(
        LAYER_NORM_NAME,
        MODEL_SCHEMA_ID,
        "layer_norm over the last axis: [..., size] to [..., size].",
        vec![
            field("size", FieldKind::Integer, REQUIRED, "Size of its last axis."),
            field(
                "bias",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether it learns a shift as well as a scale; true by default.",
            ),
            field(
                "epsilon",
                FieldKind::Number,
                OPTIONAL,
                "Added under the square root; 1e-5 by default.",
            ),
        ],
    )
}

fn describe_attention() -> TypeDescription {
    type_description(
        ATTENTION_NAME,
        MODEL_SCHEMA_ID,
        "attention over one sequence: [batch, length, hidden_size] to the same shape, with its own projections.",
        vec![
            field(
                "variant",
                enum_of(ATTENTION_VARIANT_VALUES),
                REQUIRED,
                "mha (one key and value head per query head), gqa (grouped key and value heads) or sliding (a window \
                 of tokens only).",
            ),
            field(
                "hidden_size",
                FieldKind::Integer,
                REQUIRED,
                "Size of its input's last axis and of its output.",
            ),
            field("head_count", FieldKind::Integer, REQUIRED, "Query heads."),
            field(
                "kv_head_count",
                FieldKind::Integer,
                OPTIONAL,
                "Key and value heads; head_count when left out. Fewer than head_count for gqa; it must divide \
                 head_count.",
            ),
            field("head_dim", FieldKind::Integer, REQUIRED, "Size of each head."),
            field(
                "window",
                FieldKind::Integer,
                OPTIONAL,
                "Tokens a sliding layer looks back over; required for sliding and left out otherwise.",
            ),
            field(
                "qk_norm",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether queries and keys are RMS-normalized per head before rotation; false by default.",
            ),
            field(
                "rope",
                FieldKind::Type(ROPE_NAME.to_string()),
                OPTIONAL,
                "Rotary position embedding of queries and keys; none when left out.",
            ),
        ],
    )
}

fn describe_rope() -> TypeDescription {
    type_description(
        ROPE_NAME,
        MODEL_SCHEMA_ID,
        "Rotary position embedding of an attention op's queries and keys.",
        vec![
            field("base", FieldKind::Number, OPTIONAL, "Base frequency; 10000 by default."),
            field(
                "scaling",
                enum_of(ROPE_SCALING_VALUES),
                OPTIONAL,
                "none (the default), linear, llama3 or yarn.",
            ),
            field(
                "factor",
                FieldKind::Number,
                OPTIONAL,
                "How far the context is extended; 1 by default.",
            ),
            field(
                "original_max_position",
                FieldKind::Integer,
                OPTIONAL,
                "Context length the model was trained on; required for llama3 and yarn.",
            ),
            field(
                "low_freq_factor",
                FieldKind::Number,
                OPTIONAL,
                "llama3: the low frequency band's factor; 1 by default.",
            ),
            field(
                "high_freq_factor",
                FieldKind::Number,
                OPTIONAL,
                "llama3: the high frequency band's factor; 4 by default.",
            ),
            field(
                "beta_fast",
                FieldKind::Number,
                OPTIONAL,
                "yarn: the fast rotation bound; 32 by default.",
            ),
            field(
                "beta_slow",
                FieldKind::Number,
                OPTIONAL,
                "yarn: the slow rotation bound; 1 by default.",
            ),
        ],
    )
}

fn describe_feed_forward() -> TypeDescription {
    type_description(
        FEED_FORWARD_NAME,
        MODEL_SCHEMA_ID,
        "feed_forward: [..., hidden_size] to the same shape, through intermediate_size.",
        vec![
            field(
                "variant",
                enum_of(FEED_FORWARD_VARIANT_VALUES),
                REQUIRED,
                "dense (up, activation, down) or gated (activated gate times up, then down).",
            ),
            field(
                "hidden_size",
                FieldKind::Integer,
                REQUIRED,
                "Size of its input and output.",
            ),
            field(
                "intermediate_size",
                FieldKind::Integer,
                REQUIRED,
                "Size of the hidden layer between its projections.",
            ),
            field(
                "activation",
                enum_of(ACTIVATION_FUNCTION_VALUES),
                REQUIRED,
                "The function on the up projection (dense) or the gate (gated).",
            ),
        ],
    )
}

fn describe_residual_add() -> TypeDescription {
    type_description(
        RESIDUAL_ADD_NAME,
        MODEL_SCHEMA_ID,
        "residual_add: the sum of its two inputs, which have the same shape.",
        Vec::new(),
    )
}

fn describe_unembed() -> TypeDescription {
    type_description(
        UNEMBED_NAME,
        MODEL_SCHEMA_ID,
        "unembed: [..., hidden_size] to [..., vocab_size], the logits.",
        vec![
            field(
                "hidden_size",
                FieldKind::Integer,
                REQUIRED,
                "Size of its input's last axis.",
            ),
            field(
                "vocab_size",
                FieldKind::Integer,
                REQUIRED,
                "Size of its output's last axis.",
            ),
            field(
                "tied",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether its weight is the embedding's, so it holds no parameters of its own; false by default.",
            ),
            field(
                "final_softcap",
                FieldKind::Number,
                OPTIONAL,
                "Caps the logits to within plus or minus this value, by cap * tanh(logit / cap).",
            ),
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
        assert!(MODEL_OPS.contains(&TWO_INPUTS_OP));
        assert_eq!(op_attributes_name("rope"), None);
        assert_eq!(op_attributes_name("block"), None);
        assert_eq!(MODEL_OPS.len(), 26);
    }

    #[test]
    fn every_described_attributes_type_belongs_to_one_op_or_is_the_rope_settings() {
        let names: Vec<String> = describe_model_op_types().into_iter().map(|item| item.name).collect();
        let mut by_op: Vec<String> = OPS_WITH_SHAPES
            .iter()
            .map(|op| op_attributes_name(op).unwrap().to_string())
            .collect();
        by_op.push(ROPE_NAME.to_string());
        let mut sorted_names = names.clone();
        sorted_names.sort();
        by_op.sort();
        assert_eq!(sorted_names, by_op);
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
        assert!(op_attributes_from_value("unknown_op", &BTreeMap::new()).is_none());
    }

    #[test]
    fn decoder_attributes_take_their_defaults() {
        let norm = op_attributes_from_value("rms_norm", &attributes_of(json!({"size": 16})));
        let Some(Ok(OpAttributes::RmsNorm(norm))) = norm else {
            panic!("{norm:?}");
        };
        assert!(!norm.scale_offset);
        assert_eq!(norm.epsilon, DEFAULT_RMS_EPSILON);
        let attention = op_attributes_from_value(
            "attention",
            &attributes_of(json!({"variant": "mha", "hidden_size": 16, "head_count": 2, "head_dim": 8})),
        );
        let Some(Ok(OpAttributes::Attention(attention))) = attention else {
            panic!("{attention:?}");
        };
        assert_eq!(attention.kv_head_count, None);
        assert!(!attention.qk_norm);
        assert!(attention.rope.is_none());
        let rope: RopeSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            (rope.base, rope.factor, rope.scaling),
            (10_000.0, 1.0, RopeScaling::NoScaling)
        );
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

    fn attention(variant: AttentionVariant, kv_head_count: Option<u64>, window: Option<u64>) -> OpAttributes {
        OpAttributes::Attention(AttentionAttributes {
            variant,
            hidden_size: 16,
            head_count: 4,
            kv_head_count,
            head_dim: 4,
            window,
            qk_norm: false,
            rope: None,
        })
    }

    #[test]
    fn attention_head_counts_must_fit_the_variant() {
        let fitting = [
            attention(AttentionVariant::Mha, None, None),
            attention(AttentionVariant::Gqa, Some(2), None),
            attention(AttentionVariant::Sliding, Some(1), Some(8)),
            attention(AttentionVariant::Sliding, None, Some(8)),
        ];
        for typed in &fitting {
            assert!(op_attribute_problems(typed).is_empty(), "{typed:?}");
        }
        let refused = [
            attention(AttentionVariant::Mha, Some(2), None),
            attention(AttentionVariant::Gqa, None, None),
            attention(AttentionVariant::Gqa, Some(3), None),
            attention(AttentionVariant::Gqa, Some(0), None),
            attention(AttentionVariant::Sliding, Some(1), None),
            attention(AttentionVariant::Mha, None, Some(8)),
        ];
        for typed in &refused {
            assert!(!op_attribute_problems(typed).is_empty(), "{typed:?}");
        }
    }

    #[test]
    fn a_zero_head_count_is_one_problem_at_head_count() {
        let mut typed = attention(AttentionVariant::Gqa, Some(2), None);
        if let OpAttributes::Attention(attributes) = &mut typed {
            attributes.head_count = 0;
        }
        let problems = op_attribute_problems(&typed);
        let attributes: Vec<&str> = problems.iter().map(|problem| problem.attribute).collect();
        assert_eq!(attributes, ["head_count"]);
    }

    #[test]
    fn rope_scaling_that_needs_a_length_names_it() {
        let mut rope = RopeSettings {
            base: 10_000.0,
            scaling: RopeScaling::Llama3,
            factor: 8.0,
            original_max_position: 0,
            low_freq_factor: 1.0,
            high_freq_factor: 4.0,
            beta_fast: 32.0,
            beta_slow: 1.0,
        };
        assert!(!rope_fits(&rope));
        rope.original_max_position = 8192;
        assert!(rope_fits(&rope));
        rope.scaling = RopeScaling::Linear;
        rope.original_max_position = 0;
        assert!(rope_fits(&rope));
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
        let variants: Vec<Value> = [AttentionVariant::Mha, AttentionVariant::Gqa, AttentionVariant::Sliding]
            .iter()
            .map(|variant| serde_json::to_value(variant).unwrap())
            .collect();
        assert_eq!(variants, ATTENTION_VARIANT_VALUES);
        let variants: Vec<Value> = [FeedForwardVariant::Dense, FeedForwardVariant::Gated]
            .iter()
            .map(|variant| serde_json::to_value(variant).unwrap())
            .collect();
        assert_eq!(variants, FEED_FORWARD_VARIANT_VALUES);
        let scalings: Vec<Value> = [
            RopeScaling::NoScaling,
            RopeScaling::Linear,
            RopeScaling::Llama3,
            RopeScaling::Yarn,
        ]
        .iter()
        .map(|scaling| serde_json::to_value(scaling).unwrap())
        .collect();
        assert_eq!(scalings, ROPE_SCALING_VALUES);
    }
}
