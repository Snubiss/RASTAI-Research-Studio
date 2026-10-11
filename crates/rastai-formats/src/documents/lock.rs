//! `LockFile`, a project's `rastai.lock` (`rastai.lock/1`, contracts 5.2): the exact environment, plugin versions,
//! model revisions and dataset hashes the project was last used with. `project.open` compares it with the machine and
//! offers what is missing (flows/platform.md, projects).
//!
//! The file is found by its place in the project folder, so its type comes from its `type` field. Optional fields
//! read as empty when absent and are always written.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::describe::types::{FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, field};
use crate::header::{SharedHeader, header_fields};

pub const LOCK_TYPE_NAME: &str = "rastai.lock";
pub(crate) const LOCK_SCHEMA_VERSION: u32 = 1;
const LOCK_SCHEMA_ID: &str = "rastai.lock/1";

pub(crate) const LOCK_FILE_NAME: &str = "LockFile";
const ENVIRONMENT_NAME: &str = "LockEnvironment";
const PLUGIN_NAME: &str = "LockedPlugin";
const MODEL_NAME: &str = "LockedModel";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockFile {
    #[serde(flatten)]
    pub header: SharedHeader,
    pub environment: LockEnvironment,
    /// plugin id to the version and archive hash in use.
    #[serde(default)]
    pub plugins: BTreeMap<String, LockedPlugin>,
    /// model ref to the revision and model hash in use.
    #[serde(default)]
    pub models: BTreeMap<String, LockedModel>,
    /// dataset document id to its snapshot's content hash.
    #[serde(default)]
    pub datasets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEnvironment {
    /// `cpu`, `cuda13`, ... (contracts 8.6).
    pub environment_id: String,
    pub lock_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPlugin {
    pub version: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedModel {
    pub revision: String,
    pub model_hash: String,
}

/// The lock file and its parts, for the registry (describe/registry.rs).
pub fn describe_lock_types() -> Vec<TypeDescription> {
    vec![
        describe_lock_file(),
        part(
            ENVIRONMENT_NAME,
            "The Python environment a project uses.",
            vec![
                field(
                    "environment_id",
                    FieldKind::String,
                    REQUIRED,
                    "Environment id, such as cpu or cuda13.",
                ),
                field(
                    "lock_hash",
                    FieldKind::String,
                    REQUIRED,
                    "Hash of the environment's uv lock file.",
                ),
            ],
        ),
        part(
            PLUGIN_NAME,
            "A plugin version a project uses.",
            vec![
                field("version", FieldKind::String, REQUIRED, "Plugin version."),
                field(
                    "content_hash",
                    FieldKind::String,
                    REQUIRED,
                    "Hash of the plugin archive.",
                ),
            ],
        ),
        part(
            MODEL_NAME,
            "A model revision a project uses.",
            vec![
                field(
                    "revision",
                    FieldKind::String,
                    REQUIRED,
                    "Revision, such as a commit hash.",
                ),
                field(
                    "model_hash",
                    FieldKind::String,
                    REQUIRED,
                    "Model hash of its weight files.",
                ),
            ],
        ),
    ]
}

fn describe_lock_file() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend([
        field(
            "environment",
            FieldKind::Type(ENVIRONMENT_NAME.to_string()),
            REQUIRED,
            "Environment id and lock hash.",
        ),
        field(
            "plugins",
            FieldKind::Map(Box::new(FieldKind::Type(PLUGIN_NAME.to_string()))),
            OPTIONAL,
            "Plugin id to version and archive hash.",
        ),
        field(
            "models",
            FieldKind::Map(Box::new(FieldKind::Type(MODEL_NAME.to_string()))),
            OPTIONAL,
            "Model ref to revision and model hash.",
        ),
        field(
            "datasets",
            FieldKind::Map(Box::new(FieldKind::String)),
            OPTIONAL,
            "Dataset id to snapshot content hash.",
        ),
    ]);
    TypeDescription {
        name: LOCK_FILE_NAME.to_string(),
        schema_id: LOCK_SCHEMA_ID.to_string(),
        doc: "What a project was last used with (rastai.lock).".to_string(),
        fields,
    }
}

fn part(name: &str, doc: &str, fields: Vec<FieldDescription>) -> TypeDescription {
    TypeDescription {
        name: name.to_string(),
        schema_id: LOCK_SCHEMA_ID.to_string(),
        doc: doc.to_string(),
        fields,
    }
}
