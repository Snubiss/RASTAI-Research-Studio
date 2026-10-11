//! `RunRecord`, `runs/<run_id>/run.json` (`rastai.run_record/1`, contracts 5.2): what a run was started with, so
//! `rastai rerun` and a rebuild of `rastai.db` can reproduce it. The daemon writes it once when the run is created.
//!
//! Optional fields read as empty when absent. Plain values are always written; a part only some runs have (a chain,
//! an import source, a wire type) is an `Option` and is left out when absent, because an empty object would read
//! back as a different value.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of,
};
use crate::header::{SharedHeader, header_fields};

pub const RUN_RECORD_TYPE_NAME: &str = "rastai.run_record";
pub(crate) const RUN_RECORD_SCHEMA_VERSION: u32 = 1;
const RUN_RECORD_SCHEMA_ID: &str = "rastai.run_record/1";

pub(crate) const RUN_RECORD_NAME: &str = "RunRecord";
const RECIPE_NAME: &str = "RunRecipe";
const BASE_MODEL_NAME: &str = "RunBaseModel";
const CHAIN_NAME: &str = "RunChain";
const CHAIN_STEP_NAME: &str = "RunChainStep";
const SOURCE_NAME: &str = "RunSource";

// The wire types stored as their protobuf JSON (contracts 4; layout 4.2).
const JOB_SPEC_MESSAGE: &str = "rastai.node.v1.JobSpec";
const HARDWARE_FINGERPRINT_MESSAGE: &str = "rastai.common.v1.HardwareFingerprint";

/// The run kinds of contracts 9.5, as serde writes `RunKind`.
const RUN_KIND_VALUES: &[&str] = &[
    "train",
    "teach",
    "rlhf",
    "sweep",
    "eval",
    "lens",
    "data_import",
    "data_check",
    "export",
    "imported",
    "external",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    #[serde(flatten)]
    pub header: SharedHeader,
    pub run_id: String,
    pub kind: RunKind,
    /// Protobuf JSON of `rastai.node.v1.JobSpec`. A chain run has no job of its own, and an imported one had none
    /// here, so it can be absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_spec: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<RunRecipe>,
    #[serde(default)]
    pub dataset_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model: Option<RunBaseModel>,
    /// plugin id to the version that ran.
    #[serde(default)]
    pub plugin_versions: BTreeMap<String, String>,
    #[serde(default)]
    pub environment_lock_hash: String,
    /// Absent for runs whose seed isn't known (imported runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Protobuf JSON of `rastai.common.v1.HardwareFingerprint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<Value>,
    #[serde(default)]
    pub git_commit: String,
    #[serde(default)]
    pub app_version: String,
    /// The chain run this run is a step of, or the run it reruns.
    #[serde(default)]
    pub parent_run_id: String,
    /// Chain runs only (Teach it, RLHF, sweeps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<RunChain>,
    /// Imported runs only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<RunSource>,
    /// RFC 3339 UTC.
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Train,
    Teach,
    Rlhf,
    Sweep,
    Eval,
    Lens,
    DataImport,
    DataCheck,
    Export,
    Imported,
    External,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecipe {
    pub name: String,
    /// The recipe's settings as accepted at launch; the recipe checks them, not this document.
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBaseModel {
    pub model_ref: String,
    #[serde(default)]
    pub model_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunChain {
    pub steps: Vec<RunChainStep>,
}

/// One step of a chain. Its child run id is assigned when the chain is created, so a restart never starts a step
/// twice (flows/training-evaluation.md 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunChainStep {
    pub step_name: String,
    pub child_run_id: String,
    pub command_id: String,
    #[serde(default)]
    pub arguments_json: String,
    /// Skipped with an info diagnostic when its plugin is disabled.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSource {
    /// What the run was imported from, such as a TensorBoard folder.
    pub kind: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub original_run_id: String,
}

/// The run record and its parts, for the registry (describe/registry.rs).
pub fn describe_run_record_types() -> Vec<TypeDescription> {
    vec![
        describe_run_record(),
        describe_recipe(),
        describe_base_model(),
        describe_chain(),
        describe_chain_step(),
        describe_source(),
    ]
}

fn describe_run_record() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend(what_ran_fields());
    fields.extend(provenance_fields());
    TypeDescription {
        name: RUN_RECORD_NAME.to_string(),
        schema_id: RUN_RECORD_SCHEMA_ID.to_string(),
        doc: "What a run was started with (runs/<run_id>/run.json).".to_string(),
        fields,
    }
}

/// The run, its job, recipe, data and model.
fn what_ran_fields() -> Vec<FieldDescription> {
    let job_spec = FieldKind::Proto(JOB_SPEC_MESSAGE.to_string());
    vec![
        field("run_id", FieldKind::String, REQUIRED, "The run's id."),
        field("kind", enum_of(RUN_KIND_VALUES), REQUIRED, "Run kind (contracts 9.5)."),
        field("job_spec", job_spec, OPTIONAL, "The job that ran it."),
        field("recipe", type_kind(RECIPE_NAME), OPTIONAL, "Recipe and settings."),
        field(
            "dataset_hashes",
            list_of(FieldKind::String),
            OPTIONAL,
            "Each dataset snapshot's hash.",
        ),
        field(
            "base_model",
            type_kind(BASE_MODEL_NAME),
            OPTIONAL,
            "Model it started from.",
        ),
        field(
            "plugin_versions",
            string_map(),
            OPTIONAL,
            "Plugin id to the version that ran.",
        ),
        field(
            "environment_lock_hash",
            FieldKind::String,
            OPTIONAL,
            "The environment's lock hash.",
        ),
        field("seed", FieldKind::Integer, OPTIONAL, "Random seed."),
    ]
}

/// Where it ran, what started it, and when.
fn provenance_fields() -> Vec<FieldDescription> {
    let fingerprint = FieldKind::Proto(HARDWARE_FINGERPRINT_MESSAGE.to_string());
    vec![
        field("hardware_fingerprint", fingerprint, OPTIONAL, "The hardware it ran on."),
        field(
            "git_commit",
            FieldKind::String,
            OPTIONAL,
            "Commit of the user's code, if any.",
        ),
        field(
            "app_version",
            FieldKind::String,
            OPTIONAL,
            "Rastai version that started it.",
        ),
        field(
            "parent_run_id",
            FieldKind::String,
            OPTIONAL,
            "Chain run or rerun source.",
        ),
        field("chain", type_kind(CHAIN_NAME), OPTIONAL, "Chain runs: the steps."),
        field("source", type_kind(SOURCE_NAME), OPTIONAL, "Imported runs: the source."),
        field("created_at", FieldKind::String, REQUIRED, "RFC 3339 UTC creation time."),
    ]
}

fn describe_recipe() -> TypeDescription {
    let settings = FieldKind::Map(Box::new(FieldKind::Json));
    part(
        RECIPE_NAME,
        "The recipe a run used.",
        vec![
            field("name", FieldKind::String, REQUIRED, "Recipe name."),
            field("settings", settings, OPTIONAL, "Its settings as accepted at launch."),
        ],
    )
}

fn describe_base_model() -> TypeDescription {
    part(
        BASE_MODEL_NAME,
        "The model a run started from.",
        vec![
            field("model_ref", FieldKind::String, REQUIRED, "Model ref (contracts 8.6)."),
            field("model_hash", FieldKind::String, OPTIONAL, "Hash of its weight files."),
        ],
    )
}

fn describe_chain() -> TypeDescription {
    let steps = list_of(type_kind(CHAIN_STEP_NAME));
    part(
        CHAIN_NAME,
        "The steps of a chain run.",
        vec![field("steps", steps, REQUIRED, "Steps in order.")],
    )
}

fn describe_source() -> TypeDescription {
    part(
        SOURCE_NAME,
        "Where an imported run came from.",
        vec![
            field("kind", FieldKind::String, REQUIRED, "Its source, such as tensorboard."),
            field(
                "location",
                FieldKind::String,
                OPTIONAL,
                "Folder or URL it was read from.",
            ),
            field(
                "original_run_id",
                FieldKind::String,
                OPTIONAL,
                "Its id in the other tool.",
            ),
        ],
    )
}

fn describe_chain_step() -> TypeDescription {
    part(
        CHAIN_STEP_NAME,
        "One step of a chain run.",
        vec![
            field("step_name", FieldKind::String, REQUIRED, "Such as train or verify."),
            field(
                "child_run_id",
                FieldKind::String,
                REQUIRED,
                "Run id assigned to the step.",
            ),
            field("command_id", FieldKind::String, REQUIRED, "Command the step runs."),
            field(
                "arguments_json",
                FieldKind::String,
                OPTIONAL,
                "Its arguments as JSON text.",
            ),
            field(
                "optional",
                FieldKind::Boolean,
                OPTIONAL,
                "Skipped when its plugin is off.",
            ),
        ],
    )
}

fn type_kind(type_name: &str) -> FieldKind {
    FieldKind::Type(type_name.to_string())
}

fn string_map() -> FieldKind {
    FieldKind::Map(Box::new(FieldKind::String))
}

fn part(name: &str, doc: &str, fields: Vec<FieldDescription>) -> TypeDescription {
    TypeDescription {
        name: name.to_string(),
        schema_id: RUN_RECORD_SCHEMA_ID.to_string(),
        doc: doc.to_string(),
        fields,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const ALL_KINDS: &[RunKind] = &[
        RunKind::Train,
        RunKind::Teach,
        RunKind::Rlhf,
        RunKind::Sweep,
        RunKind::Eval,
        RunKind::Lens,
        RunKind::DataImport,
        RunKind::DataCheck,
        RunKind::Export,
        RunKind::Imported,
        RunKind::External,
    ];

    #[test]
    fn kind_values_are_what_serde_writes() {
        let written: Vec<String> = ALL_KINDS
            .iter()
            .map(|kind| serde_json::to_value(kind).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(written, RUN_KIND_VALUES);
    }
}
