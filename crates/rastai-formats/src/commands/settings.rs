//! The `settings` area's types (contracts 6.4, 11.1): `settings.get` and `settings.set`.
//!
//! Settings live only in the `settings.json` files of contracts 11.1, so `settings.set` names the layer it writes
//! to, and `settings.get` says which layer each key came from.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{command_type, positional};
use crate::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};

const GET_COMMAND_ID: &str = "settings.get";
const SET_COMMAND_ID: &str = "settings.set";
const SETTING_KEY_SOURCE_TYPE: &str = "SettingKeySource";
/// The layers a value can come from, lowest first (contracts 11.1).
const SOURCE_LAYERS: &[&str] = &["default", "user", "project", "environment", "flag"];
/// The two layers `settings.set` can write; the others aren't files.
const WRITABLE_LEVELS: &[&str] = &["user", "project"];

/// Which settings to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsGetArgs {
    /// A section (`ui`) or a dotted key (`ui.theme`); empty for every setting.
    #[serde(default)]
    pub key: String,
}

/// The effective settings and where each key came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsGetResult {
    /// The merged sections, as in a `settings.json` file.
    pub values: Value,
    /// One entry per dotted key in `values`, sorted by key.
    pub sources: Vec<SettingKeySource>,
}

/// The layer one key's value came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingKeySource {
    pub key: String,
    /// One of `SOURCE_LAYERS`.
    pub source: String,
    /// For the `project` layer, the project folder whose `settings.json` holds the value.
    #[serde(default)]
    pub project_folder: String,
}

/// One key to set, at one level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSetArgs {
    /// A dotted key, such as `ui.theme`.
    pub key: String,
    /// Any JSON value the key's description allows.
    pub value: Value,
    /// `user` (the default) or `project`, the open project's `settings.json`. Left out when empty, because the
    /// empty text isn't one of the two values layer 2 accepts.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
}

/// The value now in effect, which a higher layer may still override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSetResult {
    pub key: String,
    pub value: Value,
    /// The layer the value now comes from (`SOURCE_LAYERS`).
    pub source: String,
}

pub fn describe_settings_get_args() -> TypeDescription {
    let fields = vec![positional(
        0,
        field(
            "key",
            FieldKind::String,
            OPTIONAL,
            "A section (ui) or a dotted key (ui.theme); empty or absent for every setting.",
        ),
    )];
    command_type("SettingsGetArgs", GET_COMMAND_ID, "Which settings to show.", fields)
}

pub fn describe_settings_get_result() -> TypeDescription {
    let fields = vec![
        field(
            "values",
            FieldKind::Json,
            REQUIRED,
            "The merged sections, as in a settings.json file.",
        ),
        field(
            "sources",
            list_of(FieldKind::Type(SETTING_KEY_SOURCE_TYPE.to_string())),
            REQUIRED,
            "The layer of each dotted key in values, sorted by key.",
        ),
    ];
    command_type(
        "SettingsGetResult",
        GET_COMMAND_ID,
        "The effective settings and the layer each key came from.",
        fields,
    )
}

pub fn describe_setting_key_source() -> TypeDescription {
    let fields = vec![
        field("key", FieldKind::String, REQUIRED, "A dotted key, such as ui.theme."),
        field(
            "source",
            enum_of(SOURCE_LAYERS),
            REQUIRED,
            "The layer its value came from.",
        ),
        field(
            "project_folder",
            FieldKind::String,
            OPTIONAL,
            "For the project layer, the project folder whose settings.json holds the value.",
        ),
    ];
    command_type(
        SETTING_KEY_SOURCE_TYPE,
        GET_COMMAND_ID,
        "The layer one key's value came from.",
        fields,
    )
}

pub fn describe_settings_set_args() -> TypeDescription {
    let fields = vec![
        positional(
            0,
            field("key", FieldKind::String, REQUIRED, "A dotted key, such as ui.theme."),
        ),
        field("value", FieldKind::Json, REQUIRED, "Any JSON value the key allows."),
        field(
            "level",
            enum_of(WRITABLE_LEVELS),
            OPTIONAL,
            "Which settings.json to write: user (the default) or project, the open project's.",
        ),
    ];
    command_type(
        "SettingsSetArgs",
        SET_COMMAND_ID,
        "One key to set, at the user or project level.",
        fields,
    )
}

pub fn describe_settings_set_result() -> TypeDescription {
    let fields = vec![
        field("key", FieldKind::String, REQUIRED, "The key that was set."),
        field(
            "value",
            FieldKind::Json,
            REQUIRED,
            "The value now in effect for the key.",
        ),
        field(
            "source",
            enum_of(SOURCE_LAYERS),
            REQUIRED,
            "The layer that value comes from; a higher layer may still override the one written.",
        ),
    ];
    command_type(
        "SettingsSetResult",
        SET_COMMAND_ID,
        "The key's value now in effect.",
        fields,
    )
}
