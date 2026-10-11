//! The training area's types (contracts 6.4, 6.5): `train.start` and the `TrainingPlan` it returns.
//!
//! M1 trains one recipe, `classification`, on a design file and a dataset path (TextCNN, plan M1-46). Model
//! references, recipe settings and the other recipes join these types with the milestones that build them (M5).

use serde::{Deserialize, Serialize};

use crate::commands::{command_type, positional};
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

const START_COMMAND_ID: &str = "train.start";
const TRAINING_PLAN_TYPE: &str = "TrainingPlan";

/// What to train: a recipe, the design to train and the data to train it on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainStartArgs {
    /// A recipe id from `recipe.list`; `classification` in M1.
    pub recipe: String,
    /// The `.rrsmodel` design to train, a path on the node.
    pub design_path: String,
    /// The dataset to train on, a path on the node.
    pub dataset_path: String,
    /// Eval suites to run on each new checkpoint (auto-run).
    #[serde(default)]
    pub eval_suite_ids: Vec<String>,
}

/// The run that was started, and the plan it runs with. A dry run returns the plan alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainStartResult {
    /// Empty on a dry run, which starts nothing.
    #[serde(default)]
    pub run_id: String,
    pub plan: TrainingPlan,
}

/// How the daemon decided to train (flows/training-evaluation.md 3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingPlan {
    pub recipe: String,
    /// `full`, `lora` or `qlora`.
    pub method: String,
    /// `bf16`, `fp16` or `fp32`.
    pub dtype: String,
    /// Empty for a CPU run.
    pub gpu_indices: Vec<u32>,
    pub micro_batch: u32,
    pub gradient_accumulation: u32,
    pub sequence_length: u32,
    pub memory_estimate_bytes: u64,
    pub time_estimate_seconds: u64,
}

pub fn describe_train_start_args() -> TypeDescription {
    let fields = vec![
        field(
            "recipe",
            FieldKind::String,
            REQUIRED,
            "A recipe id from recipe.list; classification in M1.",
        ),
        positional(
            0,
            field(
                "design_path",
                FieldKind::String,
                REQUIRED,
                "The .rrsmodel design to train, a path on the node.",
            ),
        ),
        positional(
            1,
            field(
                "dataset_path",
                FieldKind::String,
                REQUIRED,
                "The dataset to train on, a path on the node.",
            ),
        ),
        field(
            "eval_suite_ids",
            list_of(FieldKind::String),
            OPTIONAL,
            "Eval suites to run on each new checkpoint.",
        ),
    ];
    command_type(
        "TrainStartArgs",
        START_COMMAND_ID,
        "What to train: a recipe, the design to train and the data to train it on.",
        fields,
    )
}

pub fn describe_train_start_result() -> TypeDescription {
    let fields = vec![
        field(
            "run_id",
            FieldKind::String,
            OPTIONAL,
            "The new run; empty on a dry run, which starts nothing.",
        ),
        field(
            "plan",
            FieldKind::Type(TRAINING_PLAN_TYPE.to_string()),
            REQUIRED,
            "How the run trains.",
        ),
    ];
    command_type(
        "TrainStartResult",
        START_COMMAND_ID,
        "The run that was started, and the plan it runs with. A dry run returns the plan alone.",
        fields,
    )
}

pub fn describe_training_plan() -> TypeDescription {
    let fields = vec![
        field("recipe", FieldKind::String, REQUIRED, "The recipe id."),
        field("method", FieldKind::String, REQUIRED, "full, lora or qlora."),
        field("dtype", FieldKind::String, REQUIRED, "bf16, fp16 or fp32."),
        field(
            "gpu_indices",
            list_of(FieldKind::Integer),
            REQUIRED,
            "The GPUs on the node; empty for a CPU run.",
        ),
        field("micro_batch", FieldKind::Integer, REQUIRED, "Samples per forward pass."),
        field(
            "gradient_accumulation",
            FieldKind::Integer,
            REQUIRED,
            "Micro-batches per optimizer step.",
        ),
        field(
            "sequence_length",
            FieldKind::Integer,
            REQUIRED,
            "Tokens or positions per sample.",
        ),
        field(
            "memory_estimate_bytes",
            FieldKind::Integer,
            REQUIRED,
            "Estimated peak memory on each GPU.",
        ),
        field(
            "time_estimate_seconds",
            FieldKind::Integer,
            REQUIRED,
            "Estimated time to finish.",
        ),
    ];
    command_type(
        TRAINING_PLAN_TYPE,
        START_COMMAND_ID,
        "How the daemon decided to train: recipe, method, dtype, GPUs, batch sizes and the estimates.",
        fields,
    )
}
