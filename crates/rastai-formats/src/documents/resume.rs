//! `ResumeState`, `resume.json` in a checkpoint folder (`rastai.resume/1`, contracts 5.2): where training stood when
//! the checkpoint was written, so a resumed job continues at the same step, sample and random state. The weights,
//! optimizer and random generator states are safetensors files beside it, named here.
//!
//! Optional fields read as empty when absent and are always written.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::describe::types::{FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, field};
use crate::header::{SharedHeader, header_fields};

pub const RESUME_TYPE_NAME: &str = "rastai.resume";
pub(crate) const RESUME_SCHEMA_VERSION: u32 = 1;
const RESUME_SCHEMA_ID: &str = "rastai.resume/1";

pub(crate) const RESUME_STATE_NAME: &str = "ResumeState";
const DATALOADER_NAME: &str = "ResumeDataloader";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeState {
    #[serde(flatten)]
    pub header: SharedHeader,
    pub run_id: String,
    /// Optimizer steps taken.
    pub step: u64,
    pub epoch: u64,
    pub samples_seen: u64,
    pub dataloader: ResumeDataloader,
    /// The random generator states, normally `rng.safetensors`.
    pub rng_file: String,
    pub weights_file: String,
    #[serde(default)]
    pub optimizer_file: String,
    /// The learning-rate scheduler's state, as the recipe wrote it.
    #[serde(default)]
    pub scheduler: BTreeMap<String, Value>,
    /// Whatever else the recipe keeps between steps.
    #[serde(default)]
    pub recipe_state: BTreeMap<String, Value>,
    /// RFC 3339 UTC.
    pub created_at: String,
}

/// Where the data loader stood, so the next sample is the one that would have come next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeDataloader {
    pub dataset_hash: String,
    pub split: String,
    pub shuffle_seed: u64,
    pub epoch: u64,
    /// Samples already taken in this epoch.
    pub position: u64,
}

/// The resume state and its part, for the registry (describe/registry.rs).
pub fn describe_resume_types() -> Vec<TypeDescription> {
    vec![describe_resume_state(), describe_dataloader()]
}

fn describe_resume_state() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend([
        field(
            "run_id",
            FieldKind::String,
            REQUIRED,
            "The run the checkpoint belongs to.",
        ),
        field("step", FieldKind::Integer, REQUIRED, "Optimizer steps taken."),
        field("epoch", FieldKind::Integer, REQUIRED, "Current epoch, from 0."),
        field(
            "samples_seen",
            FieldKind::Integer,
            REQUIRED,
            "Training samples seen so far.",
        ),
        field(
            "dataloader",
            FieldKind::Type(DATALOADER_NAME.to_string()),
            REQUIRED,
            "Where the data loader stood.",
        ),
        field(
            "rng_file",
            FieldKind::String,
            REQUIRED,
            "Random generator states, e.g. rng.safetensors.",
        ),
        field(
            "weights_file",
            FieldKind::String,
            REQUIRED,
            "Weights file in the checkpoint folder.",
        ),
        field(
            "optimizer_file",
            FieldKind::String,
            OPTIONAL,
            "Optimizer state file, if the recipe has one.",
        ),
        json_object_field("scheduler", "Learning-rate scheduler state."),
        json_object_field("recipe_state", "Other state the recipe keeps between steps."),
        field(
            "created_at",
            FieldKind::String,
            REQUIRED,
            "RFC 3339 UTC time it was written.",
        ),
    ]);
    TypeDescription {
        name: RESUME_STATE_NAME.to_string(),
        schema_id: RESUME_SCHEMA_ID.to_string(),
        doc: "Where training stood at a checkpoint (resume.json).".to_string(),
        fields,
    }
}

fn describe_dataloader() -> TypeDescription {
    TypeDescription {
        name: DATALOADER_NAME.to_string(),
        schema_id: RESUME_SCHEMA_ID.to_string(),
        doc: "The data loader's position.".to_string(),
        fields: vec![
            field(
                "dataset_hash",
                FieldKind::String,
                REQUIRED,
                "Content hash of the dataset snapshot.",
            ),
            field("split", FieldKind::String, REQUIRED, "Split being read, such as train."),
            field(
                "shuffle_seed",
                FieldKind::Integer,
                REQUIRED,
                "Seed of the shuffle order.",
            ),
            field("epoch", FieldKind::Integer, REQUIRED, "Epoch of the shuffle order."),
            field(
                "position",
                FieldKind::Integer,
                REQUIRED,
                "Samples already taken in this epoch.",
            ),
        ],
    }
}

fn json_object_field(name: &str, doc: &str) -> FieldDescription {
    field(name, FieldKind::Map(Box::new(FieldKind::Json)), OPTIONAL, doc)
}
