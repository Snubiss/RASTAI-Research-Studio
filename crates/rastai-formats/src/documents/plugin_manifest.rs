//! `PluginManifest`, a plugin's `rastai-plugin.json` (`rastai.plugin/1`, contracts 5.2, 5.5; frontend layout 10.1):
//! its id, version, the host it needs, and every contribution it makes, by id.
//!
//! `contributes` has one list per extension point of contracts 5.5, all 31, so a manifest that uses any of them is
//! never refused as an unknown field; the plugin host refuses the capabilities it doesn't provide yet (`RRS-P1002`).
//! The entries the M1 host and UI read are typed here (commands, sections, panels, settings pages, themes, event
//! handlers); the others are plain JSON objects until their extension point is built (M8, layout 4.13).
//!
//! Layer 3 (validate/meaning/plugin_manifest.rs) checks the plugin id and that every command sits in its namespace.
//! Optional fields read as empty when absent and are always written; a part only some plugins have (`ui`, `python`,
//! `provider`) is left out when absent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of, type_description,
};
use crate::header::{SharedHeader, header_fields};

pub const PLUGIN_MANIFEST_TYPE_NAME: &str = "rastai.plugin";
pub(crate) const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
const PLUGIN_MANIFEST_SCHEMA_ID: &str = "rastai.plugin/1";

pub(crate) const PLUGIN_MANIFEST_NAME: &str = "PluginManifest";
const ENGINES_NAME: &str = "PluginEngines";
const CONTRIBUTIONS_NAME: &str = "PluginContributions";
const UI_NAME: &str = "PluginUi";
const PYTHON_NAME: &str = "PluginPython";
const PROVIDER_NAME: &str = "PluginProvider";
const COMMAND_NAME: &str = "CommandContribution";
const SECTION_NAME: &str = "SectionContribution";
const PANEL_NAME: &str = "PanelContribution";
const SETTINGS_PAGE_NAME: &str = "SettingsPageContribution";
const THEME_NAME: &str = "ThemeContribution";
const EVENT_HANDLER_NAME: &str = "EventHandlerContribution";

/// The values of `ui.kind`, as serde writes `PluginUiKind`.
const PLUGIN_UI_KIND_VALUES: &[&str] = &["in_process", "iframe"];
/// The values of a command's `kind` (contracts 6.1), as serde writes `PluginCommandKind`.
const PLUGIN_COMMAND_KIND_VALUES: &[&str] = &["sync", "job", "stream"];
/// The values of a panel's `placement`, as serde writes `PanelPlacement`.
const PANEL_PLACEMENT_VALUES: &[&str] = &["right", "bottom", "tab"];

/// The contribution keys whose entries are not typed yet, in contracts 5.5's order, with what each holds. The typed
/// keys are described in `describe_contributions`.
const UNTYPED_CONTRIBUTIONS: &[(&str, &str)] = &[
    ("icon_sets", "rastai.icon_set/1 files (M8)."),
    ("locales", "Catalog and UI string files (Phase 2)."),
    ("macros", "Palette actions: a title and a list of command calls (M8)."),
    ("layouts", "rastai.layout/1 files."),
    ("keybindings", "Keybinding entries."),
    ("templates", "Starter documents and projects."),
    ("file_types", "File extensions the plugin opens."),
    ("importers", "File importers."),
    ("exporters", "File exporters."),
    ("converters", "Dataset converters."),
    ("data_sources", "Streaming data sources (experimental)."),
    ("recipes", "Training recipes."),
    ("reward_functions", "Reward functions."),
    ("node_types", "Designer node types."),
    ("chat_templates", "Chat template files (M8)."),
    ("tokenizers", "Tokenizer files (experimental)."),
    ("samplers", "Samplers and logits processors."),
    ("sandbox_images", "OCI images pinned by digest."),
    ("lenses", "Psychology lenses."),
    ("sae_loaders", "SAE loaders."),
    ("metrics", "Evaluation metrics."),
    ("benchmark_loaders", "Benchmark loaders."),
    ("adapters", "Gateway adapters (experimental)."),
    ("assistant_skills", "Assistant skills (Phase 2)."),
    ("compute_targets", "Compute targets (Phase 3)."),
];

/// One contribution entry of a key that isn't typed yet: a JSON object.
pub type UntypedContribution = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    #[serde(flatten)]
    pub header: SharedHeader,
    /// One id segment (contracts 6.1); the first segment of every command the plugin adds.
    pub plugin_id: String,
    pub display_name: String,
    pub version: String,
    pub engines: PluginEngines,
    /// `"<name>@<major>"`, such as `designer.node@1` (contracts 5.5).
    #[serde(default)]
    pub requires_capabilities: Vec<String>,
    #[serde(default)]
    pub contributes: PluginContributions,
    /// Permission names (contracts 6.3).
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<PluginUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<PluginPython>,
    /// Phase 3: provider binaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<PluginProvider>,
    /// plugin id to a semver range.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// Folder of the plugin's own error catalog, if it has one.
    #[serde(default)]
    pub catalog: String,
    #[serde(default)]
    pub experimental: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEngines {
    /// A semver range of the plugin API.
    pub rastai: String,
}

/// One list per contribution key of contracts 5.5.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginContributions {
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub sections: Vec<SectionContribution>,
    #[serde(default)]
    pub panels: Vec<PanelContribution>,
    #[serde(default)]
    pub settings_pages: Vec<SettingsPageContribution>,
    #[serde(default)]
    pub themes: Vec<ThemeContribution>,
    #[serde(default)]
    pub icon_sets: Vec<UntypedContribution>,
    #[serde(default)]
    pub locales: Vec<UntypedContribution>,
    #[serde(default)]
    pub macros: Vec<UntypedContribution>,
    #[serde(default)]
    pub layouts: Vec<UntypedContribution>,
    #[serde(default)]
    pub keybindings: Vec<UntypedContribution>,
    #[serde(default)]
    pub templates: Vec<UntypedContribution>,
    #[serde(default)]
    pub file_types: Vec<UntypedContribution>,
    #[serde(default)]
    pub importers: Vec<UntypedContribution>,
    #[serde(default)]
    pub exporters: Vec<UntypedContribution>,
    #[serde(default)]
    pub converters: Vec<UntypedContribution>,
    #[serde(default)]
    pub data_sources: Vec<UntypedContribution>,
    #[serde(default)]
    pub recipes: Vec<UntypedContribution>,
    #[serde(default)]
    pub reward_functions: Vec<UntypedContribution>,
    #[serde(default)]
    pub node_types: Vec<UntypedContribution>,
    #[serde(default)]
    pub event_handlers: Vec<EventHandlerContribution>,
    #[serde(default)]
    pub chat_templates: Vec<UntypedContribution>,
    #[serde(default)]
    pub tokenizers: Vec<UntypedContribution>,
    #[serde(default)]
    pub samplers: Vec<UntypedContribution>,
    #[serde(default)]
    pub sandbox_images: Vec<UntypedContribution>,
    #[serde(default)]
    pub lenses: Vec<UntypedContribution>,
    #[serde(default)]
    pub sae_loaders: Vec<UntypedContribution>,
    #[serde(default)]
    pub metrics: Vec<UntypedContribution>,
    #[serde(default)]
    pub benchmark_loaders: Vec<UntypedContribution>,
    #[serde(default)]
    pub adapters: Vec<UntypedContribution>,
    #[serde(default)]
    pub assistant_skills: Vec<UntypedContribution>,
    #[serde(default)]
    pub compute_targets: Vec<UntypedContribution>,
}

/// A command the plugin adds. Its argument and result types are in the plugin's `types.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandContribution {
    pub command_id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    /// The scope a caller needs (contracts 6.3).
    #[serde(default)]
    pub scope: String,
    pub kind: PluginCommandKind,
    pub arguments_type: String,
    pub result_type: String,
    /// Shown in Basic mode too.
    #[serde(default)]
    pub basic: bool,
    #[serde(default)]
    pub default_shortcut: String,
    #[serde(default)]
    pub undoable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionContribution {
    /// Equal to the plugin id: a plugin has at most one section.
    pub section_id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// Position in the sidebar; lower comes first.
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub default_layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelContribution {
    pub panel_id: String,
    pub title: String,
    /// The section it docks into; empty for any.
    #[serde(default)]
    pub dock_section: String,
    pub placement: PanelPlacement,
    #[serde(default)]
    pub basic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPageContribution {
    pub page_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeContribution {
    pub theme_id: String,
    pub title: String,
    /// A `rastai.theme/1` file in the plugin, such as `themes/night.theme.json`.
    pub tokens_file: String,
}

/// Runs one of the plugin's own commands when a durable event of `event_type` is indexed (layout 12.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventHandlerContribution {
    pub event_type: String,
    pub command_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginUi {
    /// A path under the plugin's `ui/` folder.
    pub entry: String,
    pub kind: PluginUiKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPython {
    pub module: String,
    #[serde(default)]
    pub lockfile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginProvider {
    /// platform to the binary's path in the plugin.
    pub binaries: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiKind {
    /// Loaded into the app; first-party plugins only.
    InProcess,
    Iframe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCommandKind {
    Sync,
    Job,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelPlacement {
    Right,
    Bottom,
    Tab,
}

/// The manifest and every part of it, for the registry (describe/registry.rs).
pub fn describe_plugin_manifest_types() -> Vec<TypeDescription> {
    vec![
        describe_plugin_manifest(),
        describe_engines(),
        describe_contributions(),
        describe_command_contribution(),
        describe_section_contribution(),
        describe_panel_contribution(),
        describe_settings_page_contribution(),
        describe_theme_contribution(),
        describe_event_handler_contribution(),
        describe_plugin_ui(),
        describe_plugin_python(),
        describe_plugin_provider(),
    ]
}

fn describe_plugin_manifest() -> TypeDescription {
    let mut fields = header_fields();
    fields.extend(identity_fields());
    fields.extend(code_fields());
    TypeDescription {
        name: PLUGIN_MANIFEST_NAME.to_string(),
        schema_id: PLUGIN_MANIFEST_SCHEMA_ID.to_string(),
        doc: "A plugin's manifest (rastai-plugin.json).".to_string(),
        fields,
    }
}

/// Who the plugin is, what it needs and what it adds.
fn identity_fields() -> Vec<FieldDescription> {
    let strings = list_of(FieldKind::String);
    vec![
        field(
            "plugin_id",
            FieldKind::String,
            REQUIRED,
            "One id segment; its commands' first.",
        ),
        field("display_name", FieldKind::String, REQUIRED, "Name shown to people."),
        field("version", FieldKind::String, REQUIRED, "Plugin version (semver)."),
        type_field(
            "engines",
            ENGINES_NAME,
            REQUIRED,
            "The plugin API versions it supports.",
        ),
        field("requires_capabilities", strings.clone(), OPTIONAL, "As <name>@<major>."),
        type_field(
            "contributes",
            CONTRIBUTIONS_NAME,
            OPTIONAL,
            "Every contribution, by id.",
        ),
        field("permissions", strings, OPTIONAL, "Permission names (contracts 6.3)."),
    ]
}

/// Where its code is, and what else it depends on.
fn code_fields() -> Vec<FieldDescription> {
    let ranges = FieldKind::Map(Box::new(FieldKind::String));
    vec![
        type_field("ui", UI_NAME, OPTIONAL, "The plugin's UI entry."),
        type_field("python", PYTHON_NAME, OPTIONAL, "The plugin's Python module."),
        type_field("provider", PROVIDER_NAME, OPTIONAL, "Provider binaries (Phase 3)."),
        field("dependencies", ranges, OPTIONAL, "Plugin id to semver range."),
        field("catalog", FieldKind::String, OPTIONAL, "Folder of its error catalog."),
        field(
            "experimental",
            FieldKind::Boolean,
            OPTIONAL,
            "Uses experimental extension points.",
        ),
    ]
}

fn describe_engines() -> TypeDescription {
    type_description(
        ENGINES_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "The hosts the plugin runs on.",
        vec![field(
            "rastai",
            FieldKind::String,
            REQUIRED,
            "Semver range of the plugin API.",
        )],
    )
}

fn describe_settings_page_contribution() -> TypeDescription {
    type_description(
        SETTINGS_PAGE_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "A settings page.",
        vec![
            field("page_id", FieldKind::String, REQUIRED, "Page id."),
            field("title", FieldKind::String, REQUIRED, "Page title."),
        ],
    )
}

fn describe_theme_contribution() -> TypeDescription {
    type_description(
        THEME_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "A theme (from M8).",
        vec![
            field(
                "theme_id",
                FieldKind::String,
                REQUIRED,
                "Theme id, the value of ui.theme.",
            ),
            field("title", FieldKind::String, REQUIRED, "Shown in the theme picker."),
            field(
                "tokens_file",
                FieldKind::String,
                REQUIRED,
                "A rastai.theme/1 file in the plugin.",
            ),
        ],
    )
}

fn describe_event_handler_contribution() -> TypeDescription {
    type_description(
        EVENT_HANDLER_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "A command run when a durable event is indexed.",
        vec![
            field(
                "event_type",
                FieldKind::String,
                REQUIRED,
                "Durable event type (contracts 7.3).",
            ),
            field(
                "command_id",
                FieldKind::String,
                REQUIRED,
                "One of the plugin's own commands.",
            ),
        ],
    )
}

fn describe_plugin_python() -> TypeDescription {
    type_description(
        PYTHON_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "The plugin's Python module.",
        vec![
            field(
                "module",
                FieldKind::String,
                REQUIRED,
                "Module with register() and handle_command().",
            ),
            field("lockfile", FieldKind::String, OPTIONAL, "The module's uv lock file."),
        ],
    )
}

fn describe_plugin_provider() -> TypeDescription {
    let binaries = FieldKind::Map(Box::new(FieldKind::String));
    type_description(
        PROVIDER_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "Provider binaries (Phase 3).",
        vec![field("binaries", binaries, REQUIRED, "Platform to the binary's path.")],
    )
}

/// The 31 contribution keys in contracts 5.5's order: the typed ones by their entry type, the rest as objects.
fn describe_contributions() -> TypeDescription {
    let mut fields = vec![
        typed_list("commands", COMMAND_NAME, "Commands."),
        typed_list("sections", SECTION_NAME, "Sections."),
        typed_list("panels", PANEL_NAME, "Panels."),
        typed_list("settings_pages", SETTINGS_PAGE_NAME, "Settings pages."),
        typed_list("themes", THEME_NAME, "Themes (from M8)."),
    ];
    for (key, doc) in UNTYPED_CONTRIBUTIONS {
        fields.push(field(
            key,
            list_of(FieldKind::Map(Box::new(FieldKind::Json))),
            OPTIONAL,
            doc,
        ));
        // event_handlers is typed and sits between node_types and chat_templates in the contracts' order.
        if *key == "node_types" {
            fields.push(typed_list("event_handlers", EVENT_HANDLER_NAME, "Event handlers."));
        }
    }
    type_description(
        CONTRIBUTIONS_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "One list per extension point (contracts 5.5).",
        fields,
    )
}

fn describe_command_contribution() -> TypeDescription {
    type_description(
        COMMAND_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "A command the plugin adds; its types are in types.json.",
        vec![
            field("command_id", FieldKind::String, REQUIRED, "Starts with the plugin id."),
            field("title", FieldKind::String, REQUIRED, "Palette entry."),
            field("summary", FieldKind::String, OPTIONAL, "One sentence on what it does."),
            field(
                "scope",
                FieldKind::String,
                OPTIONAL,
                "Scope a caller needs (contracts 6.3).",
            ),
            field(
                "kind",
                enum_of(PLUGIN_COMMAND_KIND_VALUES),
                REQUIRED,
                "sync, job or stream.",
            ),
            field(
                "arguments_type",
                FieldKind::String,
                REQUIRED,
                "Arguments type in types.json.",
            ),
            field("result_type", FieldKind::String, REQUIRED, "Result type in types.json."),
            field("basic", FieldKind::Boolean, OPTIONAL, "Shown in Basic mode too."),
            field("default_shortcut", FieldKind::String, OPTIONAL, "Default key binding."),
            field("undoable", FieldKind::Boolean, OPTIONAL, "Records an undo step."),
        ],
    )
}

fn describe_section_contribution() -> TypeDescription {
    type_description(
        SECTION_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "The plugin's sidebar section.",
        vec![
            field("section_id", FieldKind::String, REQUIRED, "Equal to the plugin id."),
            field("title", FieldKind::String, REQUIRED, "Sidebar title."),
            field("icon", FieldKind::String, OPTIONAL, "Icon id."),
            field(
                "order",
                FieldKind::Integer,
                OPTIONAL,
                "Sidebar position; lower comes first.",
            ),
            field(
                "default_layout",
                FieldKind::String,
                OPTIONAL,
                "Layout the section opens with.",
            ),
        ],
    )
}

fn describe_panel_contribution() -> TypeDescription {
    type_description(
        PANEL_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "A panel.",
        vec![
            field("panel_id", FieldKind::String, REQUIRED, "Panel id."),
            field("title", FieldKind::String, REQUIRED, "Panel title."),
            field(
                "dock_section",
                FieldKind::String,
                OPTIONAL,
                "Section it docks into; empty for any.",
            ),
            field(
                "placement",
                enum_of(PANEL_PLACEMENT_VALUES),
                REQUIRED,
                "right, bottom or tab.",
            ),
            field("basic", FieldKind::Boolean, OPTIONAL, "Shown in Basic mode too."),
        ],
    )
}

fn describe_plugin_ui() -> TypeDescription {
    type_description(
        UI_NAME,
        PLUGIN_MANIFEST_SCHEMA_ID,
        "The plugin's UI entry.",
        vec![
            field(
                "entry",
                FieldKind::String,
                REQUIRED,
                "Path under the plugin's ui/ folder.",
            ),
            field(
                "kind",
                enum_of(PLUGIN_UI_KIND_VALUES),
                REQUIRED,
                "in_process (first-party only) or iframe.",
            ),
        ],
    )
}

fn type_field(name: &str, type_name: &str, required: bool, doc: &str) -> FieldDescription {
    field(name, FieldKind::Type(type_name.to_string()), required, doc)
}

fn typed_list(name: &str, type_name: &str, doc: &str) -> FieldDescription {
    field(name, list_of(FieldKind::Type(type_name.to_string())), OPTIONAL, doc)
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
        assert_eq!(
            written(&[PluginUiKind::InProcess, PluginUiKind::Iframe]),
            PLUGIN_UI_KIND_VALUES
        );
        let kinds = [
            PluginCommandKind::Sync,
            PluginCommandKind::Job,
            PluginCommandKind::Stream,
        ];
        assert_eq!(written(&kinds), PLUGIN_COMMAND_KIND_VALUES);
        let placements = [PanelPlacement::Right, PanelPlacement::Bottom, PanelPlacement::Tab];
        assert_eq!(written(&placements), PANEL_PLACEMENT_VALUES);
    }

    #[test]
    fn contributes_has_the_31_keys_of_contracts_5_5() {
        let names: Vec<String> = describe_contributions()
            .fields
            .iter()
            .map(|item| item.name.clone())
            .collect();
        assert_eq!(names.len(), 31);
        let position = |key: &str| names.iter().position(|name| name == key).unwrap();
        assert_eq!(position("event_handlers"), position("node_types") + 1);
        assert_eq!(names.last().map(String::as_str), Some("compute_targets"));
    }
}
