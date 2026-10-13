//! `SettingsDocument`, a `settings.json` (`rastai.settings/1`, contracts 5.2, 11): the user's file in `<config>/`
//! and a project's own file in its `.rrsproj/` folder. The files are the only settings store (contracts 11.1);
//! rastai-store writes them atomically and rastai-core merges the layers with the compiled defaults.
//!
//! A settings file is a layer: it holds only the keys it sets, and a later layer overrides single keys, not whole
//! sections. So every section and every key is an `Option` that is left out when absent, and nested objects
//! (`inference.sampling`) are optional key by key too. A key the contracts default to `null` (no quota, no seed) is
//! left out to mean that default. The sections and keys are those of contracts 11.2 that M1 builds; `archive`
//! (M11), `eval.judge_min_*` (M5), `plugins.sources` (Phase 2) and `storage.db_snapshot_*` (L2) join with their
//! milestones.

use serde::{Deserialize, Serialize};

use crate::describe::types::{
    FieldDescription, FieldKind, OPTIONAL, TypeDescription, enum_of, field, list_of, type_description,
};
use crate::header::{SharedHeader, header_fields};

pub const SETTINGS_TYPE_NAME: &str = "rastai.settings";
pub(crate) const SETTINGS_SCHEMA_VERSION: u32 = 1;
const SETTINGS_SCHEMA_ID: &str = "rastai.settings/1";

pub(crate) const SETTINGS_DOCUMENT_NAME: &str = "SettingsDocument";

// Each section and nested object: its described type's name and its doc, written once here.
const UI_NAME: &str = "SettingsUi";
const UI_DOC: &str = "Appearance and interface mode.";
const DAEMON_NAME: &str = "SettingsDaemon";
const DAEMON_DOC: &str = "The daemon's listener.";
const GATEWAY_NAME: &str = "SettingsGateway";
const GATEWAY_DOC: &str = "The inference gateway and its log.";
const INFERENCE_NAME: &str = "SettingsInference";
const INFERENCE_DOC: &str = "Default runtime, model and sampling.";
const SAMPLING_NAME: &str = "SettingsSampling";
const SAMPLING_DOC: &str = "Default sampling settings.";
const PSYCHOLOGY_NAME: &str = "SettingsPsychology";
const PSYCHOLOGY_DOC: &str = "The Psychology plugin.";
const TRAINING_NAME: &str = "SettingsTraining";
const TRAINING_DOC: &str = "Training defaults.";
const LORA_NAME: &str = "SettingsLora";
const LORA_DOC: &str = "Default LoRA size.";
const CHECKPOINT_NAME: &str = "SettingsCheckpoint";
const CHECKPOINT_DOC: &str = "Checkpoint schedule.";
const EVAL_NAME: &str = "SettingsEval";
const EVAL_DOC: &str = "The Evaluation plugin.";
const COMPUTE_NAME: &str = "SettingsCompute";
const COMPUTE_DOC: &str = "Nodes and GPUs used by default.";
const GPU_ASSIGNMENT_NAME: &str = "SettingsGpuAssignment";
const GPU_ASSIGNMENT_DOC: &str = "GPU indices per use; an empty list means any GPU.";
const NODES_NAME: &str = "SettingsNodes";
const NODES_DOC: &str = "Remote node connections.";
const STORAGE_NAME: &str = "SettingsStorage";
const STORAGE_DOC: &str = "Data folders, quota and cleanup.";
const HF_NAME: &str = "SettingsHf";
const HF_DOC: &str = "Hugging Face cache and offline mode.";
const PLUGINS_NAME: &str = "SettingsPlugins";
const PLUGINS_DOC: &str = "Plugin development and experimental APIs.";
const PRIVACY_NAME: &str = "SettingsPrivacy";
const PRIVACY_DOC: &str = "Data policy and crash reports.";
const SANDBOX_NAME: &str = "SettingsSandbox";
const SANDBOX_DOC: &str = "Agent Gym sandboxes.";
const SANDBOX_CAPS_NAME: &str = "SettingsSandboxCaps";
const SANDBOX_CAPS_DOC: &str = "Default sandbox limits.";
const LOGGING_NAME: &str = "SettingsLogging";
const LOGGING_DOC: &str = "Log level and rotation.";
const UPDATES_NAME: &str = "SettingsUpdates";
const UPDATES_DOC: &str = "Update channel.";

// The values of each enumerated key, as serde writes its Rust enum.
const UI_MODE_VALUES: &[&str] = &["basic", "advanced"];
const DENSITY_VALUES: &[&str] = &["comfortable", "compact"];
const RUNTIME_VALUES: &[&str] = &["inspectable", "fast"];
const DATA_POLICY_VALUES: &[&str] = &["local-only", "trusted-remote", "any"];
const LOG_LEVEL_VALUES: &[&str] = &["debug", "info", "warn", "error"];

/// The sections in contracts 11.2's order, each with its described type.
const SECTIONS: &[(&str, &str, &str)] = &[
    ("ui", UI_NAME, UI_DOC),
    ("daemon", DAEMON_NAME, DAEMON_DOC),
    ("gateway", GATEWAY_NAME, GATEWAY_DOC),
    ("inference", INFERENCE_NAME, INFERENCE_DOC),
    ("psychology", PSYCHOLOGY_NAME, PSYCHOLOGY_DOC),
    ("training", TRAINING_NAME, TRAINING_DOC),
    ("eval", EVAL_NAME, EVAL_DOC),
    ("compute", COMPUTE_NAME, COMPUTE_DOC),
    ("nodes", NODES_NAME, NODES_DOC),
    ("storage", STORAGE_NAME, STORAGE_DOC),
    ("hf", HF_NAME, HF_DOC),
    ("plugins", PLUGINS_NAME, PLUGINS_DOC),
    ("privacy", PRIVACY_NAME, PRIVACY_DOC),
    ("sandbox", SANDBOX_NAME, SANDBOX_DOC),
    ("logging", LOGGING_NAME, LOGGING_DOC),
    ("updates", UPDATES_NAME, UPDATES_DOC),
];

/// One settings file. The header comes first, then one optional object per section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsDocument {
    #[serde(flatten)]
    pub header: SharedHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<SettingsUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon: Option<SettingsDaemon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<SettingsGateway>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<SettingsInference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub psychology: Option<SettingsPsychology>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training: Option<SettingsTraining>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval: Option<SettingsEval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute: Option<SettingsCompute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<SettingsNodes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<SettingsStorage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hf: Option<SettingsHf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<SettingsPlugins>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<SettingsPrivacy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SettingsSandbox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<SettingsLogging>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<SettingsUpdates>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUi {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<UiModeSetting>,
    /// `system` (follow the OS), `light`, `dark`, `high_contrast` (the built-in high-contrast theme), or a plugin
    /// theme id (M8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<DensitySetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_motion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorblind_palette: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsDaemon {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_browser: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus_buffer_events: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsGateway {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incognito_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redact_pii_on_export: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_adapter: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsInference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_runtime: Option<RuntimeSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SettingsSampling>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_attention_kernel_for_replay: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsSampling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Left out: a new seed for every generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsPsychology {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_panel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overhead_budget_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sae_layers_default: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_layer_top_k: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sae_fit_max_fvu_increase: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsTraining {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_only_loss: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_checkpointing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lora: Option<SettingsLora>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<SettingsCheckpoint>,
    /// Left out: no cap beyond the GPU's free memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_cap_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resume: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rlhf_reward_model_min_accuracy: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsLora {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsCheckpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_last: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preemption_budget_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsEval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_prompt_set: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_on_new_checkpoint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_default: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsCompute {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_assignment: Option<SettingsGpuAssignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_reserve_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_cap: Option<u64>,
}

/// GPU indices per use; an empty list means any GPU.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsGpuAssignment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsNodes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_heartbeats_unreachable: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_binary: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsStorage {
    /// Left out: the data folders of contracts 8.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    /// Left out: no quota.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gc_interval_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_space_margin_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsHf {
    /// Left out: the Hugging Face tools' own cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPlugins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_apis: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPrivacy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_policy: Option<DataPolicySetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_reports: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_egress_badge: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSandbox {
    /// Left out or empty: find Podman, then Docker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_caps: Option<SettingsSandboxCaps>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSandboxCaps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsLogging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<LogLevelSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_files: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUpdates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_check: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiModeSetting {
    Basic,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensitySetting {
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSetting {
    Inspectable,
    Fast,
}

/// The values have hyphens (contracts 11.2), so each is renamed by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataPolicySetting {
    #[serde(rename = "local-only")]
    LocalOnly,
    #[serde(rename = "trusted-remote")]
    TrustedRemote,
    #[serde(rename = "any")]
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevelSetting {
    Debug,
    Info,
    Warn,
    Error,
}

/// The settings document and every section, for the registry (describe/registry.rs).
pub fn describe_settings_types() -> Vec<TypeDescription> {
    vec![
        describe_settings_document(),
        describe_ui(),
        describe_daemon(),
        describe_gateway(),
        describe_inference(),
        describe_sampling(),
        describe_psychology(),
        describe_training(),
        describe_lora(),
        describe_checkpoint(),
        describe_eval(),
        describe_compute(),
        describe_gpu_assignment(),
        describe_nodes(),
        describe_storage(),
        describe_hf(),
        describe_plugins(),
        describe_privacy(),
        describe_sandbox(),
        describe_sandbox_caps(),
        describe_logging(),
        describe_updates(),
    ]
}

fn describe_settings_document() -> TypeDescription {
    let mut fields = header_fields();
    for (name, type_name, doc) in SECTIONS {
        fields.push(key(name, FieldKind::Type(type_name.to_string()), doc));
    }
    TypeDescription {
        name: SETTINGS_DOCUMENT_NAME.to_string(),
        schema_id: SETTINGS_SCHEMA_ID.to_string(),
        doc: "A settings layer (settings.json): only the keys it sets.".to_string(),
        fields,
    }
}

fn describe_ui() -> TypeDescription {
    type_description(
        UI_NAME,
        SETTINGS_SCHEMA_ID,
        UI_DOC,
        vec![
            key("mode", enum_of(UI_MODE_VALUES), "Basic or Advanced interface."),
            key(
                "theme",
                FieldKind::String,
                "system, light, dark, high_contrast (the built-in high-contrast theme), or a plugin theme id (M8).",
            ),
            key("density", enum_of(DENSITY_VALUES), "Spacing scale."),
            key("locale", FieldKind::String, "Interface language."),
            key("reduced_motion", FieldKind::Boolean, "Turn animations off."),
            key(
                "colorblind_palette",
                FieldKind::Boolean,
                "Use the colorblind-safe chart colors.",
            ),
        ],
    )
}

fn describe_daemon() -> TypeDescription {
    type_description(
        DAEMON_NAME,
        SETTINGS_SCHEMA_ID,
        DAEMON_DOC,
        vec![
            key("port", FieldKind::Integer, "Loopback HTTP port."),
            key("listen", FieldKind::String, "Address to listen on."),
            key("open_browser", FieldKind::Boolean, "Open the browser on start."),
            key(
                "bus_buffer_events",
                FieldKind::Integer,
                "Events kept for clients that reconnect.",
            ),
        ],
    )
}

fn describe_gateway() -> TypeDescription {
    type_description(
        GATEWAY_NAME,
        SETTINGS_SCHEMA_ID,
        GATEWAY_DOC,
        vec![
            key("enabled", FieldKind::Boolean, "Serve the /v1 endpoints."),
            key("retention_days", FieldKind::Integer, "Days the gateway log is kept."),
            key("incognito_default", FieldKind::Boolean, "New chats start incognito."),
            key(
                "redact_pii_on_export",
                FieldKind::Boolean,
                "Redact personal data in exports.",
            ),
            key(
                "default_adapter",
                FieldKind::String,
                "API adapter used when none is named.",
            ),
        ],
    )
}

fn describe_inference() -> TypeDescription {
    type_description(
        INFERENCE_NAME,
        SETTINGS_SCHEMA_ID,
        INFERENCE_DOC,
        vec![
            key("default_runtime", enum_of(RUNTIME_VALUES), "inspectable or fast."),
            key("default_model", FieldKind::String, "Model ref used when none is named."),
            key(
                "sampling",
                FieldKind::Type(SAMPLING_NAME.to_string()),
                "Default sampling settings.",
            ),
            key(
                "pin_attention_kernel_for_replay",
                FieldKind::Boolean,
                "Use one attention kernel so replays match.",
            ),
        ],
    )
}

fn describe_sampling() -> TypeDescription {
    type_description(
        SAMPLING_NAME,
        SETTINGS_SCHEMA_ID,
        SAMPLING_DOC,
        vec![
            key("temperature", FieldKind::Number, "Sampling temperature."),
            key("top_k", FieldKind::Integer, "Keep the k most likely tokens."),
            key("top_p", FieldKind::Number, "Keep tokens up to this probability mass."),
            key("repetition_penalty", FieldKind::Number, "Penalty for repeated tokens."),
            key("max_tokens", FieldKind::Integer, "Most tokens to generate."),
            key(
                "seed",
                FieldKind::Integer,
                "Fixed seed; left out for a new one each time.",
            ),
        ],
    )
}

fn describe_psychology() -> TypeDescription {
    type_description(
        PSYCHOLOGY_NAME,
        SETTINGS_SCHEMA_ID,
        PSYCHOLOGY_DOC,
        vec![
            key("live_panel", FieldKind::Boolean, "Show the live panel while chatting."),
            key(
                "overhead_budget_percent",
                FieldKind::Number,
                "Most extra decode time the live panel may cost.",
            ),
            key(
                "sae_layers_default",
                list_of(FieldKind::Integer),
                "Layers with SAEs by default.",
            ),
            key("top_k", FieldKind::Integer, "Features shown per token."),
            key(
                "summary_layer_top_k",
                FieldKind::Integer,
                "Features kept per token in the summary layer.",
            ),
            key(
                "sae_fit_max_fvu_increase",
                FieldKind::Number,
                "Largest allowed FVU increase in the fit check.",
            ),
        ],
    )
}

fn describe_training() -> TypeDescription {
    type_description(
        TRAINING_NAME,
        SETTINGS_SCHEMA_ID,
        TRAINING_DOC,
        vec![
            key(
                "anchors_default",
                FieldKind::Boolean,
                "Add anchor examples to SFT and DPO.",
            ),
            key(
                "answer_only_loss",
                FieldKind::Boolean,
                "Compute the loss on answers only.",
            ),
            key(
                "gradient_checkpointing",
                FieldKind::Boolean,
                "Trade compute for memory.",
            ),
            key("lora", FieldKind::Type(LORA_NAME.to_string()), "Default LoRA size."),
            key(
                "checkpoint",
                FieldKind::Type(CHECKPOINT_NAME.to_string()),
                "Checkpoint schedule.",
            ),
            key(
                "memory_cap_bytes",
                FieldKind::Integer,
                "Most GPU memory training may use.",
            ),
            key(
                "auto_resume",
                FieldKind::Boolean,
                "Resume preempted runs automatically.",
            ),
            key(
                "rlhf_reward_model_min_accuracy",
                FieldKind::Number,
                "Reward model accuracy RLHF needs to continue.",
            ),
        ],
    )
}

fn describe_lora() -> TypeDescription {
    type_description(
        LORA_NAME,
        SETTINGS_SCHEMA_ID,
        LORA_DOC,
        vec![
            key("rank", FieldKind::Integer, "LoRA rank."),
            key("alpha", FieldKind::Integer, "LoRA alpha."),
        ],
    )
}

fn describe_checkpoint() -> TypeDescription {
    type_description(
        CHECKPOINT_NAME,
        SETTINGS_SCHEMA_ID,
        CHECKPOINT_DOC,
        vec![
            key("keep_last", FieldKind::Integer, "Checkpoints kept per run."),
            key("interval_minutes", FieldKind::Integer, "Minutes between checkpoints."),
            key(
                "preemption_budget_seconds",
                FieldKind::Integer,
                "Time allowed to checkpoint on SIGTERM.",
            ),
        ],
    )
}

fn describe_eval() -> TypeDescription {
    type_description(
        EVAL_NAME,
        SETTINGS_SCHEMA_ID,
        EVAL_DOC,
        vec![
            key(
                "drift_prompt_set",
                FieldKind::String,
                "Prompt set for drift checks; empty for the built-in one.",
            ),
            key(
                "run_on_new_checkpoint",
                FieldKind::Boolean,
                "Run the suites on each new checkpoint.",
            ),
            key(
                "judge_default",
                FieldKind::String,
                "Judge model used when none is named.",
            ),
        ],
    )
}

fn describe_compute() -> TypeDescription {
    type_description(
        COMPUTE_NAME,
        SETTINGS_SCHEMA_ID,
        COMPUTE_DOC,
        vec![
            key("default_node", FieldKind::String, "local or a node id."),
            key(
                "gpu_assignment",
                FieldKind::Type(GPU_ASSIGNMENT_NAME.to_string()),
                "GPUs per use.",
            ),
            key(
                "vram_reserve_bytes",
                FieldKind::Integer,
                "GPU memory left free on every GPU.",
            ),
            key("sandbox_cap", FieldKind::Integer, "Most sandboxes at once."),
        ],
    )
}

fn describe_gpu_assignment() -> TypeDescription {
    type_description(
        GPU_ASSIGNMENT_NAME,
        SETTINGS_SCHEMA_ID,
        GPU_ASSIGNMENT_DOC,
        vec![
            key("chat", list_of(FieldKind::Integer), "GPUs for chat."),
            key("training", list_of(FieldKind::Integer), "GPUs for training."),
        ],
    )
}

fn describe_nodes() -> TypeDescription {
    type_description(
        NODES_NAME,
        SETTINGS_SCHEMA_ID,
        NODES_DOC,
        vec![
            key("heartbeat_seconds", FieldKind::Integer, "Seconds between heartbeats."),
            key(
                "missed_heartbeats_unreachable",
                FieldKind::Integer,
                "Missed heartbeats before a node is unreachable.",
            ),
            key("ssh_binary", FieldKind::String, "The OpenSSH client to run."),
        ],
    )
}

fn describe_storage() -> TypeDescription {
    type_description(
        STORAGE_NAME,
        SETTINGS_SCHEMA_ID,
        STORAGE_DOC,
        vec![
            key("data_dir", FieldKind::String, "Data folder; left out for the default."),
            key(
                "quota_bytes",
                FieldKind::Integer,
                "Most bytes the data folder may hold; left out for none.",
            ),
            key("gc_interval_hours", FieldKind::Integer, "Hours between cleanups."),
            key(
                "free_space_margin_bytes",
                FieldKind::Integer,
                "Space always left free on the disk.",
            ),
        ],
    )
}

fn describe_hf() -> TypeDescription {
    type_description(
        HF_NAME,
        SETTINGS_SCHEMA_ID,
        HF_DOC,
        vec![
            key(
                "cache_dir",
                FieldKind::String,
                "Cache folder; left out for the Hugging Face default.",
            ),
            key("offline", FieldKind::Boolean, "Never contact the Hub."),
        ],
    )
}

fn describe_plugins() -> TypeDescription {
    type_description(
        PLUGINS_NAME,
        SETTINGS_SCHEMA_ID,
        PLUGINS_DOC,
        vec![
            key(
                "dev_paths",
                list_of(FieldKind::String),
                "Unpacked plugin folders to load.",
            ),
            key(
                "experimental_apis",
                FieldKind::Boolean,
                "Allow experimental extension points.",
            ),
        ],
    )
}

fn describe_privacy() -> TypeDescription {
    type_description(
        PRIVACY_NAME,
        SETTINGS_SCHEMA_ID,
        PRIVACY_DOC,
        vec![
            key(
                "data_policy",
                enum_of(DATA_POLICY_VALUES),
                "Where data may go (contracts 11.1).",
            ),
            key("crash_reports", FieldKind::Boolean, "Offer to review a crash report."),
            key(
                "show_egress_badge",
                FieldKind::Boolean,
                "Show the badge while data leaves the machine.",
            ),
        ],
    )
}

fn describe_sandbox() -> TypeDescription {
    type_description(
        SANDBOX_NAME,
        SETTINGS_SCHEMA_ID,
        SANDBOX_DOC,
        vec![
            key(
                "engine_socket",
                FieldKind::String,
                "Container engine socket; empty finds Podman, then Docker.",
            ),
            key(
                "default_caps",
                FieldKind::Type(SANDBOX_CAPS_NAME.to_string()),
                "Default sandbox limits.",
            ),
        ],
    )
}

fn describe_sandbox_caps() -> TypeDescription {
    type_description(
        SANDBOX_CAPS_NAME,
        SETTINGS_SCHEMA_ID,
        SANDBOX_CAPS_DOC,
        vec![
            key("cpu", FieldKind::Integer, "CPU cores."),
            key("ram_bytes", FieldKind::Integer, "Memory."),
            key("time_seconds", FieldKind::Integer, "Wall-clock time."),
            key("pids_limit", FieldKind::Integer, "Most processes."),
            key("disk_bytes", FieldKind::Integer, "Disk space."),
        ],
    )
}

fn describe_logging() -> TypeDescription {
    type_description(
        LOGGING_NAME,
        SETTINGS_SCHEMA_ID,
        LOGGING_DOC,
        vec![
            key("level", enum_of(LOG_LEVEL_VALUES), "Lowest level written."),
            key(
                "max_file_bytes",
                FieldKind::Integer,
                "Size at which a log file rotates.",
            ),
            key("keep_files", FieldKind::Integer, "Rotated files kept."),
        ],
    )
}

fn describe_updates() -> TypeDescription {
    type_description(
        UPDATES_NAME,
        SETTINGS_SCHEMA_ID,
        UPDATES_DOC,
        vec![
            key("channel", FieldKind::String, "Release channel, such as stable."),
            key("auto_check", FieldKind::Boolean, "Check for updates on start."),
        ],
    )
}

/// Every settings key is optional: a layer sets only some.
fn key(name: &str, kind: FieldKind, doc: &str) -> FieldDescription {
    field(name, kind, OPTIONAL, doc)
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

    /// The theme setting names the built-in high-contrast value, so a reader of the schema sees it (contracts 11.2).
    #[test]
    fn the_theme_setting_names_the_built_in_high_contrast_theme() {
        let ui = describe_settings_types()
            .into_iter()
            .find(|item| item.name == UI_NAME)
            .unwrap();
        let theme = ui.fields.iter().find(|item| item.name == "theme").unwrap();
        assert!(theme.doc.contains("high_contrast"), "{}", theme.doc);
    }

    #[test]
    fn enum_values_are_what_serde_writes() {
        assert_eq!(
            written(&[UiModeSetting::Basic, UiModeSetting::Advanced]),
            UI_MODE_VALUES
        );
        assert_eq!(
            written(&[DensitySetting::Comfortable, DensitySetting::Compact]),
            DENSITY_VALUES
        );
        assert_eq!(
            written(&[RuntimeSetting::Inspectable, RuntimeSetting::Fast]),
            RUNTIME_VALUES
        );
        let policies = [
            DataPolicySetting::LocalOnly,
            DataPolicySetting::TrustedRemote,
            DataPolicySetting::Any,
        ];
        assert_eq!(written(&policies), DATA_POLICY_VALUES);
        let levels = [
            LogLevelSetting::Debug,
            LogLevelSetting::Info,
            LogLevelSetting::Warn,
            LogLevelSetting::Error,
        ];
        assert_eq!(written(&levels), LOG_LEVEL_VALUES);
    }

    #[test]
    fn a_layer_writes_only_the_keys_it_sets() {
        let ui = SettingsUi {
            theme: Some("dark".to_string()),
            density: Some(DensitySetting::Compact),
            ..SettingsUi::default()
        };
        let written = serde_json::to_value(&ui).unwrap();
        assert_eq!(written, serde_json::json!({"theme": "dark", "density": "compact"}));
    }

    #[test]
    fn every_section_type_is_described() {
        let names: Vec<String> = describe_settings_types().into_iter().map(|item| item.name).collect();
        for (_, type_name, _) in SECTIONS {
            assert!(names.contains(&type_name.to_string()), "{type_name}");
        }
    }
}
