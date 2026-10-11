//! `Project`, the `project.json` of a `<name>.rrsproj/` folder (`rastai.project/1`, contracts 5.2), and the folder
//! layout around it.
//!
//! The project's data policy is not a field here: it is `privacy.data_policy` in the project's own `settings.json`
//! (contracts 11.1, one store per layer). Layer 3 (validate/meaning/project.rs) checks the name and the default node.

use serde::{Deserialize, Serialize};

use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, field};
use crate::header::{SharedHeader, header_fields};

pub const PROJECT_TYPE_NAME: &str = "rastai.project";
pub(crate) const PROJECT_SCHEMA_VERSION: u32 = 1;
const PROJECT_SCHEMA_ID: &str = "rastai.project/1";

pub(crate) const PROJECT_NAME: &str = "Project";

/// The project document inside the folder.
pub const PROJECT_FILE: &str = "project.json";
/// The folders every project has (contracts 5.2), created empty by `project.create`.
pub const PROJECT_FOLDERS: &[&str] = &[
    "designs",
    "harnesses",
    "data",
    "data/converters",
    "evals",
    "replays",
    "presets",
    "layouts",
    "runs",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    #[serde(flatten)]
    pub header: SharedHeader,
    pub name: String,
    /// `local` or a node id. Empty means the user's `compute.default_node` setting.
    #[serde(default)]
    pub default_node: String,
}

/// The project document, for the registry (describe/registry.rs).
pub fn describe_project_types() -> Vec<TypeDescription> {
    let mut fields = header_fields();
    fields.extend([
        field("name", FieldKind::String, REQUIRED, "The project's name."),
        field(
            "default_node",
            FieldKind::String,
            OPTIONAL,
            "local or a node id; empty means the compute.default_node setting.",
        ),
    ]);
    vec![TypeDescription {
        name: PROJECT_NAME.to_string(),
        schema_id: PROJECT_SCHEMA_ID.to_string(),
        doc: "A project (project.json in a .rrsproj folder).".to_string(),
        fields,
    }]
}
