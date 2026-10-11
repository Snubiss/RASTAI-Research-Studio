//! The `plugin` area's types (contracts 6.4, 9.7): `plugin.list`, which the UI reads at every start to register
//! each enabled plugin's contributions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::command_type;
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

const LIST_COMMAND_ID: &str = "plugin.list";
const PLUGIN_SUMMARY_TYPE: &str = "PluginSummary";
/// Why a plugin can't load travels as catalog diagnostics in their protobuf JSON (contracts 4.1).
const DIAGNOSTIC_MESSAGE: &str = "rastai.common.v1.Diagnostic";

/// `plugin.list` takes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginListArgs {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginListResult {
    pub plugins: Vec<PluginSummary>,
}

/// One installed plugin, as the `plugins` table and its manifest describe it (contracts 8.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSummary {
    pub plugin_id: String,
    pub version: String,
    /// A plugin state of contracts 9.7, such as `enabled`.
    pub state: String,
    /// Where the host found it: `preinstalled`, `installed` or `dev`.
    pub source: String,
    pub granted_permissions: Vec<String>,
    /// The plugin's `rastai-plugin.json` manifest (`rastai.plugin/1`), with the permissions it requests.
    pub manifest: Value,
    /// Why it is `incompatible` or `failed`, as protobuf JSON `Diagnostic`s; empty when it loaded.
    #[serde(default)]
    pub diagnostics: Vec<Value>,
}

pub fn describe_plugin_list_args() -> TypeDescription {
    command_type(
        "PluginListArgs",
        LIST_COMMAND_ID,
        "plugin.list takes no arguments.",
        Vec::new(),
    )
}

pub fn describe_plugin_list_result() -> TypeDescription {
    let fields = vec![field(
        "plugins",
        list_of(FieldKind::Type(PLUGIN_SUMMARY_TYPE.to_string())),
        REQUIRED,
        "Every installed plugin, enabled or not.",
    )];
    command_type("PluginListResult", LIST_COMMAND_ID, "The installed plugins.", fields)
}

pub fn describe_plugin_summary() -> TypeDescription {
    let fields = vec![
        field("plugin_id", FieldKind::String, REQUIRED, "The plugin's id."),
        field("version", FieldKind::String, REQUIRED, "Its installed version."),
        field(
            "state",
            FieldKind::String,
            REQUIRED,
            "Its plugin state, such as enabled.",
        ),
        field(
            "source",
            FieldKind::String,
            REQUIRED,
            "Where the host found it: preinstalled, installed or dev.",
        ),
        field(
            "granted_permissions",
            list_of(FieldKind::String),
            REQUIRED,
            "The permissions the user granted.",
        ),
        field(
            "manifest",
            FieldKind::Json,
            REQUIRED,
            "Its rastai-plugin.json manifest, with the permissions it requests.",
        ),
        field(
            "diagnostics",
            list_of(FieldKind::Proto(DIAGNOSTIC_MESSAGE.to_string())),
            OPTIONAL,
            "Why it is incompatible or failed; empty when it loaded.",
        ),
    ];
    command_type(
        PLUGIN_SUMMARY_TYPE,
        LIST_COMMAND_ID,
        "One installed plugin: its state, manifest and granted permissions.",
        fields,
    )
}
