//! The dry-run plans of the install and download commands (contracts 6.5): what a command would do, with the disk
//! check it runs first. A dry run returns only the plan; a real run starts the job and returns the same plan.
//!
//! A plan is a command's result type, so its description carries the schema id of the command it belongs to
//! (`rastai.command.model.download/1`), like the rest of the command's types.

use serde::{Deserialize, Serialize};

use crate::commands::command_type;
use crate::describe::types::{FieldKind, REQUIRED, TypeDescription, field, list_of};

const MODEL_DOWNLOAD_COMMAND_ID: &str = "model.download";
const ENVIRONMENT_INSTALL_COMMAND_ID: &str = "system.env.install";

/// What `model.download` would fetch, and whether the disk holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDownloadPlan {
    pub model_ref: String,
    /// The file names the download fetches, in the order it fetches them.
    pub files: Vec<String>,
    pub headers_only: bool,
    /// The bytes the files need in total.
    pub size_bytes: u64,
    /// The free space on the Hugging Face cache's disk before the download starts.
    pub free_bytes: u64,
}

/// What `system.env.install` would create or sync for one backend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentInstallPlan {
    pub environment_id: String,
    pub backend: String,
    /// `blake3:<64 hex>` of the lockfile the sync uses (ADR-0009 decision 8).
    pub lock_hash: String,
    /// The bytes the backend's packages need, PyTorch included.
    pub size_bytes: u64,
    /// The free space under the node's data directory before the sync starts (RRS-S3001 when it is short).
    pub free_bytes: u64,
}

pub fn describe_model_download_plan() -> TypeDescription {
    let fields = vec![
        field(
            "model_ref",
            FieldKind::String,
            REQUIRED,
            "The model that would be downloaded.",
        ),
        field(
            "files",
            list_of(FieldKind::String),
            REQUIRED,
            "The files the download fetches, in order.",
        ),
        field(
            "headers_only",
            FieldKind::Boolean,
            REQUIRED,
            "True when only the safetensors headers would be fetched.",
        ),
        field(
            "size_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The bytes the files need in total.",
        ),
        field(
            "free_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The free space on the cache's disk before the download starts.",
        ),
    ];
    command_type(
        "ModelDownloadPlan",
        MODEL_DOWNLOAD_COMMAND_ID,
        "What a model download would fetch, with the disk check.",
        fields,
    )
}

pub fn describe_environment_install_plan() -> TypeDescription {
    let fields = vec![
        field(
            "environment_id",
            FieldKind::String,
            REQUIRED,
            "The environment that would be synced.",
        ),
        field("backend", FieldKind::String, REQUIRED, "The backend it is for."),
        field(
            "lock_hash",
            FieldKind::String,
            REQUIRED,
            "The hash of the lockfile the sync uses, as blake3:<64 hex>.",
        ),
        field(
            "size_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The bytes the backend's packages need, PyTorch included.",
        ),
        field(
            "free_bytes",
            FieldKind::Integer,
            REQUIRED,
            "The free space under the data directory before the sync starts.",
        ),
    ];
    command_type(
        "EnvironmentInstallPlan",
        ENVIRONMENT_INSTALL_COMMAND_ID,
        "What an environment install would create or sync, with the disk check.",
        fields,
    )
}
