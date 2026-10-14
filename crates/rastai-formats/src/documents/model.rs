//! `ModelSpec`, a `.rrsmodel` design (`rastai.model/1`, contracts 5.2): a graph of ops (contracts 5.3), the input
//! map that feeds it, its heads, the objective it trains for, the defaults its shapes are worked out with, the
//! pretrained weights it starts from and, once planned, which GPU holds each graph node.
//!
//! M1-18 built the slice TextCNN needs. M2-01 adds block definitions (`blocks`), block instances (`block_ref`) and
//! `repeat` for the decoder families, as optional fields. LoRA settings, opaque nodes and canvas positions join the
//! same schema version with the tasks that build them (M5-E03, M6-E01, M6-E04), each as an optional field, so every
//! file written now still reads then.
//!
//! An op's `attributes` are free JSON here and checked by layer 3 against the op's own description
//! (documents/model_ops.rs), because which attributes a node takes depends on its `op`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of, type_description,
};
use crate::header::{SharedHeader, header_fields};

/// The header's `type` value.
pub const MODEL_TYPE_NAME: &str = "rastai.model";
pub(crate) const MODEL_SCHEMA_VERSION: u32 = 1;
pub(crate) const MODEL_SCHEMA_ID: &str = "rastai.model/1";
/// Designs are found by this ending (`designs/<name>.rrsmodel`).
pub(crate) const MODEL_FILE_EXTENSION: &str = ".rrsmodel";

// The described type names, which `FieldKind::Type` refers to.
pub(crate) const MODEL_SPEC_NAME: &str = "ModelSpec";
const GRAPH_NAME: &str = "ModelGraph";
const NODE_NAME: &str = "GraphNode";
const EDGE_NAME: &str = "GraphEdge";
const INPUT_NAME: &str = "InputRange";
const SPAN_NAME: &str = "InputSpan";
const OUTPUT_NAME: &str = "OutputHead";
const OBJECTIVE_NAME: &str = "Objective";
const DEFAULTS_NAME: &str = "ModelDefaults";
const WEIGHTS_NAME: &str = "WeightsSource";

// The enum values, as serde writes `TrainMode`, `ObjectiveKind` and `ModelDtype`.
const TRAIN_MODE_VALUES: &[&str] = &["freeze", "lora", "train"];
const OBJECTIVE_KIND_VALUES: &[&str] = &["classification", "next_token"];
const DTYPE_VALUES: &[&str] = &["bf16", "fp16", "fp32"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    #[serde(flatten)]
    pub header: SharedHeader,
    pub graph: ModelGraph,
    /// The input map: which part of the model's input feeds which graph node.
    pub inputs: Vec<InputRange>,
    /// The heads: graph nodes whose outputs are the model's outputs.
    pub outputs: Vec<OutputHead>,
    /// Absent while a design isn't meant for training yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Objective>,
    pub defaults: ModelDefaults,
    /// Reusable graphs, by block id. A block instance (a graph node with op `block` and a `block_ref`) stamps one
    /// into this design; its nodes are not shape-checked until the decoder blocks are built (M2-03).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blocks: BTreeMap<String, ModelGraph>,
    /// Pretrained parts this design starts from; empty for a model trained from scratch. A list, so one design can
    /// compose several (plan section 10 row 11).
    #[serde(default)]
    pub weights: Vec<WeightsSource>,
    /// Graph node id to GPU index. Written only by `plan_layer_placement` (contracts 2.4 rule 10); empty means one
    /// device.
    #[serde(default)]
    pub placement: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelGraph {
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

/// One graph node: an op and its attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Unique in the graph; edges, the input map, heads and placement name it.
    pub node_id: String,
    /// An op of contracts 5.3, such as `conv1d`.
    pub op: String,
    /// What the canvas shows; empty means the node id.
    #[serde(default)]
    pub label: String,
    /// The op's attributes (documents/model_ops.rs describes each op's).
    #[serde(default)]
    pub attributes: BTreeMap<String, Value>,
    #[serde(default)]
    pub train_mode: TrainMode,
    /// Whether the node's outputs are observation points (contracts 5.3).
    #[serde(default = "observed_by_default")]
    pub observe: bool,
    /// The block a block instance stamps (a key of `blocks`); only on nodes with op `block`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_ref: Option<String>,
    /// How many copies of the instance's block the design stamps, one after another; 1 when absent. Only on nodes
    /// with op `block`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u64>,
}

/// How training treats a graph node's weights.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainMode {
    Freeze,
    Lora,
    #[default]
    Train,
}

/// One edge: the output of one graph node feeds one input of another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from_node: String,
    pub to_node: String,
    /// Which input of `to_node` it feeds, from 0. Only ops with several inputs (`concat`) have more than input 0;
    /// their inputs are taken in this order.
    #[serde(default)]
    pub to_input: u32,
}

/// One entry of the input map: a named part of the model's input and the graph node it feeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRange {
    pub name: String,
    pub range: InputSpan,
    /// The graph node that receives this input as its input 0, such as an embedding.
    pub encoder_node: String,
}

/// Positions in the model's input sequence, `start` included and `end` left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSpan {
    pub start: u64,
    pub end: u64,
}

/// One head: a graph node whose output is a model output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputHead {
    pub name: String,
    pub graph_node: String,
}

/// What the model is trained to do; the recipe of the same name trains it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    pub kind: ObjectiveKind,
    /// The recipe checks these, not this document.
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
}

/// The objectives of contracts 5.2: classification for TextCNN, and next_token, the decoder families' language
/// modeling objective (M2-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveKind {
    Classification,
    NextToken,
}

/// The sizes the design's shapes are worked out with, and its number format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDefaults {
    pub dtype: ModelDtype,
    pub sequence_length: u64,
    pub batch_size: u64,
}

/// The number formats a design runs in, the same names as a training plan's (contracts 6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDtype {
    Bf16,
    Fp16,
    Fp32,
}

/// Pretrained weights for the whole design or one part of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightsSource {
    /// A model ref (contracts 8.6): `hf:<org>/<name>@<revision>`, `blob:<hash>` or `ckpt:<checkpoint_id>`.
    pub source: String,
    /// The family that maps the source's tensor names to this design, such as `llama`; empty for Rastai's own.
    #[serde(default)]
    pub family: String,
    /// Source tensor name to the design's tensor name, for names the family doesn't map.
    #[serde(default)]
    pub name_mapping: BTreeMap<String, String>,
    /// Put before every mapped tensor name, so the weights fill one part of the design; empty for the whole design.
    #[serde(default)]
    pub prefix: String,
}

fn observed_by_default() -> bool {
    true
}

/// The ModelSpec and its parts, for the registry (describe/registry.rs). The op attribute types are listed by
/// `describe_model_op_types` (documents/model_ops.rs).
pub fn describe_model_types() -> Vec<TypeDescription> {
    vec![
        describe_model_spec(),
        describe_graph(),
        describe_node(),
        describe_edge(),
        describe_input(),
        describe_span(),
        describe_output(),
        describe_objective(),
        describe_defaults(),
        describe_weights(),
    ]
}

fn describe_model_spec() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend(model_fields());
    TypeDescription {
        name: MODEL_SPEC_NAME.to_string(),
        schema_id: MODEL_SCHEMA_ID.to_string(),
        doc: "A model design (.rrsmodel): a graph of ops, its inputs, heads, objective and defaults.".to_string(),
        fields,
    }
}

fn model_fields() -> Vec<FieldDescription> {
    vec![
        field("graph", type_kind(GRAPH_NAME), REQUIRED, "Graph nodes and edges."),
        field(
            "inputs",
            list_of(type_kind(INPUT_NAME)),
            REQUIRED,
            "The input map: which input feeds which graph node.",
        ),
        field(
            "outputs",
            list_of(type_kind(OUTPUT_NAME)),
            REQUIRED,
            "The heads: graph nodes whose outputs are the model's.",
        ),
        field(
            "objective",
            type_kind(OBJECTIVE_NAME),
            OPTIONAL,
            "What the model is trained to do.",
        ),
        field(
            "defaults",
            type_kind(DEFAULTS_NAME),
            REQUIRED,
            "Number format and the sizes shapes are worked out with.",
        ),
        field(
            "blocks",
            FieldKind::Map(Box::new(type_kind(GRAPH_NAME))),
            OPTIONAL,
            "Reusable graphs by block id, stamped by block instances.",
        ),
        field(
            "weights",
            list_of(type_kind(WEIGHTS_NAME)),
            OPTIONAL,
            "Pretrained parts it starts from; empty to train from scratch.",
        ),
        field(
            "placement",
            FieldKind::Map(Box::new(FieldKind::Integer)),
            OPTIONAL,
            "Graph node id to GPU index, written by the layer placement planner.",
        ),
    ]
}

fn describe_graph() -> TypeDescription {
    type_description(
        GRAPH_NAME,
        MODEL_SCHEMA_ID,
        "The graph of a design.",
        vec![
            field("nodes", list_of(type_kind(NODE_NAME)), REQUIRED, "Graph nodes."),
            field(
                "edges",
                list_of(type_kind(EDGE_NAME)),
                OPTIONAL,
                "Which output feeds which input.",
            ),
        ],
    )
}

fn describe_node() -> TypeDescription {
    let attributes = FieldKind::Map(Box::new(FieldKind::Json));
    type_description(
        NODE_NAME,
        MODEL_SCHEMA_ID,
        "One graph node: an op and its attributes.",
        vec![
            field("node_id", FieldKind::String, REQUIRED, "Unique id in the graph."),
            field(
                "op",
                FieldKind::String,
                REQUIRED,
                "An op, such as conv1d (contracts 5.3).",
            ),
            field(
                "label",
                FieldKind::String,
                OPTIONAL,
                "Name shown on the canvas; empty means the node id.",
            ),
            field("attributes", attributes, OPTIONAL, "The op's attributes."),
            field(
                "train_mode",
                enum_of(TRAIN_MODE_VALUES),
                OPTIONAL,
                "freeze, lora or train (the default).",
            ),
            field(
                "observe",
                FieldKind::Boolean,
                OPTIONAL,
                "Whether its outputs are observation points; true by default.",
            ),
            field(
                "block_ref",
                FieldKind::String,
                OPTIONAL,
                "The block a block instance (op block) stamps; a key of blocks.",
            ),
            field(
                "repeat",
                FieldKind::Integer,
                OPTIONAL,
                "How many copies of the block to stamp, one after another; 1 when left out.",
            ),
        ],
    )
}

fn describe_edge() -> TypeDescription {
    type_description(
        EDGE_NAME,
        MODEL_SCHEMA_ID,
        "The output of one graph node feeding an input of another.",
        vec![
            field(
                "from_node",
                FieldKind::String,
                REQUIRED,
                "The graph node whose output it carries.",
            ),
            field("to_node", FieldKind::String, REQUIRED, "The graph node it feeds."),
            field(
                "to_input",
                FieldKind::Integer,
                OPTIONAL,
                "Which input of to_node, from 0 (the default).",
            ),
        ],
    )
}

fn describe_input() -> TypeDescription {
    type_description(
        INPUT_NAME,
        MODEL_SCHEMA_ID,
        "One entry of the input map.",
        vec![
            field("name", FieldKind::String, REQUIRED, "The input's name, such as tokens."),
            field(
                "range",
                type_kind(SPAN_NAME),
                REQUIRED,
                "Its positions in the input sequence.",
            ),
            field(
                "encoder_node",
                FieldKind::String,
                REQUIRED,
                "The graph node it feeds, as that node's input 0.",
            ),
        ],
    )
}

fn describe_span() -> TypeDescription {
    type_description(
        SPAN_NAME,
        MODEL_SCHEMA_ID,
        "Positions in the input sequence, start included, end left out.",
        vec![
            field("start", FieldKind::Integer, REQUIRED, "First position."),
            field("end", FieldKind::Integer, REQUIRED, "One past the last position."),
        ],
    )
}

fn describe_output() -> TypeDescription {
    type_description(
        OUTPUT_NAME,
        MODEL_SCHEMA_ID,
        "One head of the model.",
        vec![
            field(
                "name",
                FieldKind::String,
                REQUIRED,
                "The output's name, such as logits.",
            ),
            field(
                "graph_node",
                FieldKind::String,
                REQUIRED,
                "The graph node whose output it is.",
            ),
        ],
    )
}

fn describe_objective() -> TypeDescription {
    let settings = FieldKind::Map(Box::new(FieldKind::Json));
    type_description(
        OBJECTIVE_NAME,
        MODEL_SCHEMA_ID,
        "What the model is trained to do.",
        vec![
            field(
                "kind",
                enum_of(OBJECTIVE_KIND_VALUES),
                REQUIRED,
                "The objective, such as classification.",
            ),
            field("settings", settings, OPTIONAL, "Its settings, checked by the recipe."),
        ],
    )
}

fn describe_defaults() -> TypeDescription {
    type_description(
        DEFAULTS_NAME,
        MODEL_SCHEMA_ID,
        "The number format and the sizes a design's shapes are worked out with.",
        vec![
            field("dtype", enum_of(DTYPE_VALUES), REQUIRED, "bf16, fp16 or fp32."),
            field(
                "sequence_length",
                FieldKind::Integer,
                REQUIRED,
                "Positions in one input sequence.",
            ),
            field("batch_size", FieldKind::Integer, REQUIRED, "Sequences in one batch."),
        ],
    )
}

fn describe_weights() -> TypeDescription {
    type_description(
        WEIGHTS_NAME,
        MODEL_SCHEMA_ID,
        "Pretrained weights for the whole design or one part of it.",
        vec![
            field(
                "source",
                FieldKind::String,
                REQUIRED,
                "Model ref: hf:<org>/<name>@<revision>, blob:<hash> or ckpt:<checkpoint_id>.",
            ),
            field(
                "family",
                FieldKind::String,
                OPTIONAL,
                "Family that maps its tensor names, such as llama.",
            ),
            field(
                "name_mapping",
                FieldKind::Map(Box::new(FieldKind::String)),
                OPTIONAL,
                "Source tensor name to the design's, beyond the family's mapping.",
            ),
            field(
                "prefix",
                FieldKind::String,
                OPTIONAL,
                "Put before every mapped name; empty for the whole design.",
            ),
        ],
    )
}

fn type_kind(type_name: &str) -> FieldKind {
    FieldKind::Type(type_name.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn written<T: Serialize>(values: &[T]) -> Vec<String> {
        values
            .iter()
            .map(|value| serde_json::to_value(value).unwrap().as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn enum_values_are_what_serde_writes() {
        let train_modes = [TrainMode::Freeze, TrainMode::Lora, TrainMode::Train];
        assert_eq!(written(&train_modes), TRAIN_MODE_VALUES);
        assert_eq!(
            written(&[ObjectiveKind::Classification, ObjectiveKind::NextToken]),
            OBJECTIVE_KIND_VALUES
        );
        assert_eq!(
            written(&[ModelDtype::Bf16, ModelDtype::Fp16, ModelDtype::Fp32]),
            DTYPE_VALUES
        );
    }

    #[test]
    fn a_bare_node_trains_and_is_observed() {
        let node: GraphNode = serde_json::from_str(r#"{"node_id": "a", "op": "dropout"}"#).unwrap();
        assert_eq!(node.train_mode, TrainMode::Train);
        assert!(node.observe);
        assert!(node.attributes.is_empty());
        let edge: GraphEdge = serde_json::from_str(r#"{"from_node": "a", "to_node": "b"}"#).unwrap();
        assert_eq!(edge.to_input, 0);
    }
}
