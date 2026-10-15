//! The `model` area's types (contracts 6.4, 6.5): listing, inspecting and downloading models. A download's result
//! holds its plan (`ModelDownloadPlan`, documents/plans.rs), so a dry run and a real run answer with the same type.
//!
//! `ModelSummary`, `ModelInspectResult`'s `GpuFit` and `ModelDownloadArgs`'s `ModelDownloadFile` are the parts these
//! commands are built from; each is described once, here, under the command that first needs it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::command_type;
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};
use crate::documents::plans::ModelDownloadPlan;

const LIST_COMMAND_ID: &str = "model.list";
const INSPECT_COMMAND_ID: &str = "model.inspect";
const DOWNLOAD_COMMAND_ID: &str = "model.download";
const MODEL_SUMMARY_TYPE: &str = "ModelSummary";
const GPU_FIT_TYPE: &str = "GpuFit";
const MODEL_DOWNLOAD_FILE_TYPE: &str = "ModelDownloadFile";
const MODEL_DOWNLOAD_PLAN_TYPE: &str = "ModelDownloadPlan";
/// Where a model comes from (contracts 6.4): the local folder, the Hugging Face cache, or a third-party API.
const MODEL_SOURCES: &[&str] = &["local", "hf_cache", "api"];

/// One model in `model.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSummary {
    pub model_ref: String,
    pub source: String,
    pub size_bytes: u64,
    /// Empty when the model names no license.
    #[serde(default)]
    pub license: String,
}

/// `model.list` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelListArgs {}

/// The models the daemon can see.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelListResult {
    pub models: Vec<ModelSummary>,
}

/// Whether one model's weights fit on one GPU, with the VRAM they would need there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuFit {
    pub gpu_index: u32,
    pub vram_bytes_needed: u64,
    pub fits: bool,
}

/// `model.inspect` names one model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInspectArgs {
    pub model_ref: String,
}

/// A model's config, family, size, license, replay twin and fit per GPU.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInspectResult {
    pub model_ref: String,
    pub family: String,
    pub size_bytes: u64,
    pub license: String,
    /// The full-precision model a quantized one replays on; empty when there is none.
    #[serde(default)]
    pub replay_twin: String,
    pub fits: Vec<GpuFit>,
    /// The model's `config.json`, as it is.
    pub config: Value,
}

/// One file of a download. `url` and `hash` are for a file that isn't in the Hugging Face repository, such as a
/// Neuronpedia label file; both are empty for a repository file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadFile {
    pub name: String,
    #[serde(default)]
    pub url: String,
    /// `blake3:<64 hex>` of the file's content, checked after the download.
    #[serde(default)]
    pub hash: String,
}

/// What `model.download` answers: its plan. A dry run returns only this; a real run also starts the job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadResult {
    pub plan: ModelDownloadPlan,
}

/// `model.download` fetches a model into the Hugging Face cache, resumable, after a disk check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadArgs {
    pub model_ref: String,
    /// The branch, tag or commit to fetch; empty for the repository's default branch. A commit pins the files, so a
    /// run that names one gets the same weights every time.
    #[serde(default)]
    pub revision: String,
    /// Chosen files, such as SAE layers. Empty means the whole repository.
    #[serde(default)]
    pub files: Vec<ModelDownloadFile>,
    /// Fetch only the safetensors headers, which is enough for `model.inspect`.
    #[serde(default)]
    pub headers_only: bool,
}

pub fn describe_model_list_args() -> TypeDescription {
    command_type(
        "ModelListArgs",
        LIST_COMMAND_ID,
        "model.list takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_model_list_result() -> TypeDescription {
    let fields = vec![field(
        "models",
        list_of(FieldKind::Type(MODEL_SUMMARY_TYPE.to_string())),
        REQUIRED,
        "The local, Hugging Face cache and API models.",
    )];
    command_type(
        "ModelListResult",
        LIST_COMMAND_ID,
        "The models the daemon can see.",
        fields,
    )
}

pub fn describe_model_summary() -> TypeDescription {
    let fields = vec![
        field("model_ref", FieldKind::String, REQUIRED, "The model's reference."),
        field(
            "source",
            enum_of(MODEL_SOURCES),
            REQUIRED,
            "Where the model is: local, hf_cache or api.",
        ),
        field(
            "size_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The bytes the model's files take.",
        ),
        field(
            "license",
            FieldKind::String,
            OPTIONAL,
            "The license the model names, if any.",
        ),
    ];
    command_type(MODEL_SUMMARY_TYPE, LIST_COMMAND_ID, "One model in model.list.", fields)
}

pub fn describe_model_inspect_args() -> TypeDescription {
    let fields = vec![field("model_ref", FieldKind::String, REQUIRED, "The model to inspect.")];
    command_type("ModelInspectArgs", INSPECT_COMMAND_ID, "The model to inspect.", fields)
}

pub fn describe_model_inspect_result() -> TypeDescription {
    let fields = vec![
        field("model_ref", FieldKind::String, REQUIRED, "The model inspected."),
        field(
            "family",
            FieldKind::String,
            REQUIRED,
            "The model family, such as llama.",
        ),
        field(
            "size_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The bytes the model's weights take.",
        ),
        field(
            "license",
            FieldKind::String,
            REQUIRED,
            "The license the model names, or empty.",
        ),
        field(
            "replay_twin",
            FieldKind::String,
            OPTIONAL,
            "The full-precision model a quantized one replays on; empty when there is none.",
        ),
        field(
            "fits",
            list_of(FieldKind::Type(GPU_FIT_TYPE.to_string())),
            REQUIRED,
            "Whether the weights fit on each visible GPU.",
        ),
        field(
            "config",
            FieldKind::Json,
            REQUIRED,
            "The model's config.json, as it is.",
        ),
    ];
    command_type(
        "ModelInspectResult",
        INSPECT_COMMAND_ID,
        "A model's config, family, size and fit.",
        fields,
    )
}

pub fn describe_gpu_fit() -> TypeDescription {
    let fields = vec![
        field("gpu_index", FieldKind::Integer, REQUIRED, "The GPU's physical index."),
        field(
            "vram_bytes_needed",
            FieldKind::Integer,
            REQUIRED,
            "The VRAM the weights need on that GPU.",
        ),
        field(
            "fits",
            FieldKind::Boolean,
            REQUIRED,
            "True when that GPU has the VRAM free.",
        ),
    ];
    command_type(GPU_FIT_TYPE, INSPECT_COMMAND_ID, "One GPU's fit for a model.", fields)
}

pub fn describe_model_download_args() -> TypeDescription {
    let fields = vec![
        field(
            "model_ref",
            FieldKind::String,
            REQUIRED,
            "The model to download, a Hugging Face repository id.",
        ),
        field(
            "revision",
            FieldKind::String,
            OPTIONAL,
            "The branch, tag or commit to download; left out for the repository's default branch. A commit pins the files.",
        ),
        field(
            "files",
            list_of(FieldKind::Type(MODEL_DOWNLOAD_FILE_TYPE.to_string())),
            OPTIONAL,
            "Chosen files, such as SAE layers; left out for the whole repository.",
        ),
        field(
            "headers_only",
            FieldKind::Boolean,
            OPTIONAL,
            "Fetch only the safetensors headers; left out for the whole files.",
        ),
    ];
    command_type(
        "ModelDownloadArgs",
        DOWNLOAD_COMMAND_ID,
        "Download a model into the Hugging Face cache.",
        fields,
    )
}

pub fn describe_model_download_result() -> TypeDescription {
    let fields = vec![field(
        "plan",
        FieldKind::Type(MODEL_DOWNLOAD_PLAN_TYPE.to_string()),
        REQUIRED,
        "What the download would fetch, with the disk check.",
    )];
    command_type(
        "ModelDownloadResult",
        DOWNLOAD_COMMAND_ID,
        "The download's plan; a real run also starts the job.",
        fields,
    )
}

pub fn describe_model_download_file() -> TypeDescription {
    let fields = vec![
        field(
            "name",
            FieldKind::String,
            REQUIRED,
            "The file's name in the repository or the cache.",
        ),
        field(
            "url",
            FieldKind::String,
            OPTIONAL,
            "Where to fetch a file outside the repository; left out for a repository file. Set together with `hash`.",
        ),
        field(
            "hash",
            FieldKind::String,
            OPTIONAL,
            "blake3:<64 hex> of the file's content, checked after the download. Required whenever `url` is set.",
        ),
    ];
    command_type(
        MODEL_DOWNLOAD_FILE_TYPE,
        DOWNLOAD_COMMAND_ID,
        "One file of a download.",
        fields,
    )
}
