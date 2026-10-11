//! Every description lists exactly the fields its struct's serde writes, with matching kinds (layout 4.11). A
//! description that drifts from its struct would let layer 2 pass a file that serde then refuses, or refuse a file
//! serde would read; this test catches both in CI.
//!
//! Each type is checked with a filled value: every optional field set, so serde writes every key. A document type's
//! PR adds one test here with its own filled value, for the document and for every part of it.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use rastai_formats::describe::registry::{describe_all_types, find_description};
use rastai_formats::describe::structure_check::{StructureCheck, find_mismatches};
use rastai_formats::documents::lock::{LockEnvironment, LockFile, LockedModel, LockedPlugin};
use rastai_formats::documents::plugin_manifest::{
    CommandContribution, EventHandlerContribution, PanelContribution, PanelPlacement, PluginCommandKind,
    PluginContributions, PluginEngines, PluginManifest, PluginProvider, PluginPython, PluginUi, PluginUiKind,
    SectionContribution, SettingsPageContribution, ThemeContribution,
};
use rastai_formats::documents::project::Project;
use rastai_formats::documents::resume::{ResumeDataloader, ResumeState};
use rastai_formats::documents::run_record::{
    RunBaseModel, RunChain, RunChainStep, RunKind, RunRecipe, RunRecord, RunSource,
};
use rastai_formats::documents::settings::{
    DataPolicySetting, DensitySetting, LogLevelSetting, RuntimeSetting, SettingsCheckpoint, SettingsCompute,
    SettingsDaemon, SettingsDocument, SettingsEval, SettingsGateway, SettingsGpuAssignment, SettingsHf,
    SettingsInference, SettingsLogging, SettingsLora, SettingsNodes, SettingsPlugins, SettingsPrivacy,
    SettingsPsychology, SettingsSampling, SettingsSandbox, SettingsSandboxCaps, SettingsStorage, SettingsTraining,
    SettingsUi, SettingsUpdates, UiModeSetting,
};
use rastai_formats::documents::theme::{
    COLOR_ROLES, ThemeBase, ThemeColors, ThemeDensity, ThemeDocument, ThemeElevation, ThemeFocusRing, ThemeFontSizes,
    ThemeFontWeights, ThemeLineHeights, ThemeMotion, ThemeRadius, ThemeSpacing, ThemeTypography,
};
use rastai_formats::documents::{DOCUMENT_TYPES, DocumentType};
use rastai_formats::header::{SHARED_HEADER_TYPE_NAME, SharedHeader};
use rastai_formats::json::pointer::join_pointer;
use rastai_formats::json::positions::position_of;
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const HASH: &str = "blake3:0000000000000000000000000000000000000000000000000000000000000000";

/// The checks every type gets: same keys, every value of its described kind, the written order is the description
/// order, and the written value reads back into an equal struct.
fn assert_description_matches<T: Serialize + DeserializeOwned + PartialEq + Debug>(filled: &T, type_name: &str) {
    let all_types = describe_all_types();
    let description = find_description(&all_types, type_name).unwrap();
    let written = serde_json::to_value(filled).unwrap();

    let mut written_keys: Vec<&str> = written.as_object().unwrap().keys().map(String::as_str).collect();
    let mut described_keys: Vec<&str> = description.fields.iter().map(|field| field.name.as_str()).collect();
    written_keys.sort_unstable();
    described_keys.sort_unstable();
    assert_eq!(
        written_keys, described_keys,
        "{type_name}: serde keys and described fields differ"
    );

    let check = StructureCheck {
        all_types: &all_types,
        report_unknown_fields: true,
    };
    assert_eq!(
        find_mismatches(&written, description, &check),
        vec![],
        "{type_name}: kinds differ"
    );

    assert_written_in_description_order(filled, type_name);
    let read_back: T = serde_json::from_value(written).unwrap();
    assert_eq!(&read_back, filled);
}

/// Files list fields in the struct's order (json/pretty.rs) and the codegen in the description's; they must agree,
/// so each described field must start on a later line of the written file than the one before it.
fn assert_written_in_description_order<T: Serialize>(filled: &T, type_name: &str) {
    let all_types = describe_all_types();
    let description = find_description(&all_types, type_name).unwrap();
    let text = pretty_json(filled).unwrap();
    let positions = read_json(&text, &ImportLimits::default()).unwrap().positions;
    let lines: Vec<u32> = description
        .fields
        .iter()
        .map(|field| position_of(&positions, &join_pointer("", &field.name)))
        .map(|position| position.unwrap().line)
        .collect();
    let mut sorted_lines = lines.clone();
    sorted_lines.sort_unstable();
    assert_eq!(
        lines, sorted_lines,
        "{type_name}: written order differs from description order"
    );
}

fn filled_header() -> SharedHeader {
    header_of_type("rastai.model")
}

/// A header with every field set, for a document of this type.
fn header_of_type(type_name: &str) -> SharedHeader {
    let mut requires = BTreeMap::new();
    requires.insert("psychology".to_string(), "^1.2".to_string());
    SharedHeader {
        schema_url: format!("https://schemas.example/{type_name}/1.json"),
        document_type: type_name.to_string(),
        schema_version: 1,
        created_with: "rastai 0.1.0".to_string(),
        id: "model_01J".to_string(),
        content_hash: HASH.to_string(),
        derived_from: vec![format!("model_00A@blake3:{}", "1".repeat(64))],
        authors: vec!["Ada".to_string()],
        license: "Apache-2.0".to_string(),
        source_url: "https://example.org/model".to_string(),
        requires,
    }
}

#[test]
fn the_shared_header_matches_its_description() {
    assert_description_matches(&filled_header(), SHARED_HEADER_TYPE_NAME);
}

#[test]
fn a_header_with_only_required_fields_still_writes_every_key() {
    let minimal: SharedHeader = serde_json::from_value(serde_json::json!({
        "$schema": "s", "type": "rastai.model", "schema_version": 1, "id": "model_1"
    }))
    .unwrap();
    let written = serde_json::to_value(&minimal).unwrap();
    let all_types = describe_all_types();
    let description = find_description(&all_types, SHARED_HEADER_TYPE_NAME).unwrap();
    for field in &description.fields {
        assert!(written.get(&field.name).is_some(), "{} is not written", field.name);
    }
    assert_eq!(written["license"], Value::String(String::new()));
}

#[test]
fn every_registered_description_has_fields() {
    for description in describe_all_types() {
        assert!(!description.fields.is_empty(), "{} has no fields", description.name);
        assert!(!description.doc.is_empty(), "{} has no doc", description.name);
    }
}

fn text(value: &str) -> String {
    value.to_string()
}

fn map_of(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries.iter().map(|(key, value)| (text(key), text(value))).collect()
}

fn json_map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

// Run record.

fn filled_chain_step() -> RunChainStep {
    RunChainStep {
        step_name: text("train"),
        child_run_id: text("run_01JA2B3C4D5E6F7G8H9J0K1M2R"),
        command_id: text("train.start"),
        arguments_json: text("{}"),
        optional: true,
    }
}

fn filled_run_record() -> RunRecord {
    RunRecord {
        header: header_of_type("rastai.run_record"),
        run_id: text("run_01JA2B3C4D5E6F7G8H9J0K1M2N"),
        kind: RunKind::Teach,
        job_spec: Some(json!({"job_id": "job_1"})),
        recipe: Some(RunRecipe {
            name: text("sft"),
            settings: json_map(json!({"epochs": 3})),
        }),
        dataset_hashes: vec![text(HASH)],
        base_model: Some(RunBaseModel {
            model_ref: text("hf:google/gemma-3-1b-it@main"),
            model_hash: text(HASH),
        }),
        plugin_versions: map_of(&[("eval", "1.0.0")]),
        environment_lock_hash: text(HASH),
        seed: Some(7),
        hardware_fingerprint: Some(json!({"gpu_names": []})),
        git_commit: text("0123abc"),
        app_version: text("0.1.0"),
        parent_run_id: text("run_01JA2B3C4D5E6F7G8H9J0K1M2P"),
        chain: Some(RunChain {
            steps: vec![filled_chain_step()],
        }),
        source: Some(RunSource {
            kind: text("tensorboard"),
            location: text("/runs/old"),
            original_run_id: text("old-1"),
        }),
        created_at: text("2026-10-11T12:00:00Z"),
    }
}

#[test]
fn the_run_record_and_its_parts_match_their_descriptions() {
    let run_record = filled_run_record();
    assert_description_matches(&run_record, "RunRecord");
    assert_description_matches(run_record.recipe.as_ref().unwrap(), "RunRecipe");
    assert_description_matches(run_record.base_model.as_ref().unwrap(), "RunBaseModel");
    assert_description_matches(run_record.chain.as_ref().unwrap(), "RunChain");
    assert_description_matches(&filled_chain_step(), "RunChainStep");
    assert_description_matches(run_record.source.as_ref().unwrap(), "RunSource");
}

// Resume state.

fn filled_resume_state() -> ResumeState {
    ResumeState {
        header: header_of_type("rastai.resume"),
        run_id: text("run_01JA2B3C4D5E6F7G8H9J0K1M2N"),
        step: 1500,
        epoch: 1,
        samples_seen: 48000,
        dataloader: ResumeDataloader {
            dataset_hash: text(HASH),
            split: text("train"),
            shuffle_seed: 1234,
            epoch: 1,
            position: 16000,
        },
        rng_file: text("rng.safetensors"),
        weights_file: text("adapter.safetensors"),
        optimizer_file: text("optimizer.safetensors"),
        scheduler: json_map(json!({"last_step": 1500})),
        recipe_state: json_map(json!({"anchor_index": 3})),
        created_at: text("2026-10-11T13:00:00Z"),
    }
}

#[test]
fn the_resume_state_and_its_part_match_their_descriptions() {
    let resume = filled_resume_state();
    assert_description_matches(&resume, "ResumeState");
    assert_description_matches(&resume.dataloader, "ResumeDataloader");
}

// Settings: every section and every key set.

fn filled_settings() -> SettingsDocument {
    SettingsDocument {
        header: header_of_type("rastai.settings"),
        ui: Some(filled_ui()),
        daemon: Some(filled_daemon()),
        gateway: Some(filled_gateway()),
        inference: Some(filled_inference()),
        psychology: Some(filled_psychology()),
        training: Some(filled_training()),
        eval: Some(filled_eval()),
        compute: Some(filled_compute()),
        nodes: Some(filled_nodes()),
        storage: Some(filled_storage()),
        hf: Some(filled_hf()),
        plugins: Some(filled_plugins()),
        privacy: Some(filled_privacy()),
        sandbox: Some(filled_sandbox()),
        logging: Some(filled_logging()),
        updates: Some(filled_updates()),
    }
}

fn filled_ui() -> SettingsUi {
    SettingsUi {
        mode: Some(UiModeSetting::Basic),
        theme: Some(text("dark")),
        density: Some(DensitySetting::Compact),
        locale: Some(text("en")),
        reduced_motion: Some(true),
        colorblind_palette: Some(false),
    }
}

fn filled_daemon() -> SettingsDaemon {
    SettingsDaemon {
        port: Some(8899),
        listen: Some(text("127.0.0.1")),
        open_browser: Some(false),
        bus_buffer_events: Some(10000),
    }
}

fn filled_gateway() -> SettingsGateway {
    SettingsGateway {
        enabled: Some(true),
        retention_days: Some(30),
        incognito_default: Some(false),
        redact_pii_on_export: Some(true),
        default_adapter: Some(text("openai")),
    }
}

fn filled_psychology() -> SettingsPsychology {
    SettingsPsychology {
        live_panel: Some(true),
        overhead_budget_percent: Some(35.0),
        sae_layers_default: Some(vec![4, 8]),
        top_k: Some(16),
        summary_layer_top_k: Some(8),
        sae_fit_max_fvu_increase: Some(0.05),
    }
}

fn filled_eval() -> SettingsEval {
    SettingsEval {
        drift_prompt_set: Some(text("default")),
        run_on_new_checkpoint: Some(true),
        judge_default: Some(text("api:openai/judge")),
    }
}

fn filled_compute() -> SettingsCompute {
    SettingsCompute {
        default_node: Some(text("local")),
        gpu_assignment: Some(SettingsGpuAssignment {
            chat: Some(vec![0]),
            training: Some(vec![1, 2]),
        }),
        vram_reserve_bytes: Some(1 << 30),
        sandbox_cap: Some(8),
    }
}

fn filled_nodes() -> SettingsNodes {
    SettingsNodes {
        heartbeat_seconds: Some(5),
        missed_heartbeats_unreachable: Some(3),
        ssh_binary: Some(text("ssh")),
    }
}

fn filled_storage() -> SettingsStorage {
    SettingsStorage {
        data_dir: Some(text("/data/rastai")),
        quota_bytes: Some(1 << 40),
        gc_interval_hours: Some(24),
        free_space_margin_bytes: Some(5 << 30),
    }
}

fn filled_hf() -> SettingsHf {
    SettingsHf {
        cache_dir: Some(text("/data/hf")),
        offline: Some(false),
    }
}

fn filled_plugins() -> SettingsPlugins {
    SettingsPlugins {
        dev_paths: Some(vec![text("plugins/psychology")]),
        experimental_apis: Some(false),
    }
}

fn filled_privacy() -> SettingsPrivacy {
    SettingsPrivacy {
        data_policy: Some(DataPolicySetting::TrustedRemote),
        crash_reports: Some(false),
        show_egress_badge: Some(true),
    }
}

fn filled_sandbox() -> SettingsSandbox {
    SettingsSandbox {
        engine_socket: Some(text("/run/podman/podman.sock")),
        default_caps: Some(SettingsSandboxCaps {
            cpu: Some(2),
            ram_bytes: Some(4 << 30),
            time_seconds: Some(600),
            pids_limit: Some(512),
            disk_bytes: Some(10 << 30),
        }),
    }
}

fn filled_logging() -> SettingsLogging {
    SettingsLogging {
        level: Some(LogLevelSetting::Debug),
        max_file_bytes: Some(10 << 20),
        keep_files: Some(5),
    }
}

fn filled_updates() -> SettingsUpdates {
    SettingsUpdates {
        channel: Some(text("stable")),
        auto_check: Some(true),
    }
}

fn filled_inference() -> SettingsInference {
    SettingsInference {
        default_runtime: Some(RuntimeSetting::Fast),
        default_model: Some(text("hf:google/gemma-3-1b-it@main")),
        sampling: Some(SettingsSampling {
            temperature: Some(0.7),
            top_k: Some(64),
            top_p: Some(0.95),
            repetition_penalty: Some(1.1),
            max_tokens: Some(1024),
            seed: Some(7),
        }),
        pin_attention_kernel_for_replay: Some(true),
    }
}

fn filled_training() -> SettingsTraining {
    SettingsTraining {
        anchors_default: Some(true),
        answer_only_loss: Some(true),
        gradient_checkpointing: Some(true),
        lora: Some(SettingsLora {
            rank: Some(16),
            alpha: Some(32),
        }),
        checkpoint: Some(SettingsCheckpoint {
            keep_last: Some(3),
            interval_minutes: Some(15),
            preemption_budget_seconds: Some(25),
        }),
        memory_cap_bytes: Some(20 << 30),
        auto_resume: Some(true),
        rlhf_reward_model_min_accuracy: Some(0.65),
    }
}

#[test]
fn the_settings_document_and_every_section_match_their_descriptions() {
    let settings = filled_settings();
    assert_description_matches(&settings, "SettingsDocument");
    assert_description_matches(settings.ui.as_ref().unwrap(), "SettingsUi");
    assert_description_matches(settings.daemon.as_ref().unwrap(), "SettingsDaemon");
    assert_description_matches(settings.gateway.as_ref().unwrap(), "SettingsGateway");
    let inference = settings.inference.as_ref().unwrap();
    assert_description_matches(inference, "SettingsInference");
    assert_description_matches(inference.sampling.as_ref().unwrap(), "SettingsSampling");
    assert_description_matches(settings.psychology.as_ref().unwrap(), "SettingsPsychology");
    let training = settings.training.as_ref().unwrap();
    assert_description_matches(training, "SettingsTraining");
    assert_description_matches(training.lora.as_ref().unwrap(), "SettingsLora");
    assert_description_matches(training.checkpoint.as_ref().unwrap(), "SettingsCheckpoint");
    assert_description_matches(settings.eval.as_ref().unwrap(), "SettingsEval");
    let compute = settings.compute.as_ref().unwrap();
    assert_description_matches(compute, "SettingsCompute");
    assert_description_matches(compute.gpu_assignment.as_ref().unwrap(), "SettingsGpuAssignment");
    assert_description_matches(settings.nodes.as_ref().unwrap(), "SettingsNodes");
    assert_description_matches(settings.storage.as_ref().unwrap(), "SettingsStorage");
    assert_description_matches(settings.hf.as_ref().unwrap(), "SettingsHf");
    assert_description_matches(settings.plugins.as_ref().unwrap(), "SettingsPlugins");
    assert_description_matches(settings.privacy.as_ref().unwrap(), "SettingsPrivacy");
    let sandbox = settings.sandbox.as_ref().unwrap();
    assert_description_matches(sandbox, "SettingsSandbox");
    assert_description_matches(sandbox.default_caps.as_ref().unwrap(), "SettingsSandboxCaps");
    assert_description_matches(settings.logging.as_ref().unwrap(), "SettingsLogging");
    assert_description_matches(settings.updates.as_ref().unwrap(), "SettingsUpdates");
}

#[test]
fn ui_theme_and_density_are_settings_keys() {
    let all_types = describe_all_types();
    let ui = find_description(&all_types, "SettingsUi").unwrap();
    let keys: Vec<&str> = ui.fields.iter().map(|field| field.name.as_str()).collect();
    assert!(keys.contains(&"theme") && keys.contains(&"density"), "{keys:?}");
}

// Project and lock file.

#[test]
fn the_project_matches_its_description() {
    let project = Project {
        header: header_of_type("rastai.project"),
        name: text("TextCNN sentiment"),
        default_node: text("local"),
    };
    assert_description_matches(&project, "Project");
}

#[test]
fn the_lock_file_and_its_parts_match_their_descriptions() {
    let plugin = LockedPlugin {
        version: text("1.0.0"),
        content_hash: text(HASH),
    };
    let model = LockedModel {
        revision: text("main"),
        model_hash: text(HASH),
    };
    let lock = LockFile {
        header: header_of_type("rastai.lock"),
        environment: LockEnvironment {
            environment_id: text("cuda13"),
            lock_hash: text(HASH),
        },
        plugins: [(text("eval"), plugin.clone())].into_iter().collect(),
        models: [(text("hf:google/gemma-3-1b-it"), model.clone())].into_iter().collect(),
        datasets: map_of(&[("doc_1", HASH)]),
    };
    assert_description_matches(&lock, "LockFile");
    assert_description_matches(&lock.environment, "LockEnvironment");
    assert_description_matches(&plugin, "LockedPlugin");
    assert_description_matches(&model, "LockedModel");
}

// Plugin manifest.

fn filled_command() -> CommandContribution {
    CommandContribution {
        command_id: text("acme_tools.refresh"),
        title: text("Refresh"),
        summary: text("Reload the report."),
        scope: text("plugin:acme_tools"),
        kind: PluginCommandKind::Sync,
        arguments_type: text("RefreshArgs"),
        result_type: text("RefreshResult"),
        basic: true,
        default_shortcut: text("Ctrl+Shift+R"),
        undoable: true,
    }
}

fn filled_contributions() -> PluginContributions {
    let untyped = vec![json_map(json!({"lens_id": "acme_tools.norms"}))];
    PluginContributions {
        commands: vec![filled_command()],
        sections: vec![SectionContribution {
            section_id: text("acme_tools"),
            title: text("Acme"),
            icon: text("flask"),
            order: 40,
            default_layout: text("layouts/acme.json"),
        }],
        panels: vec![PanelContribution {
            panel_id: text("acme_tools.report"),
            title: text("Report"),
            dock_section: text("acme_tools"),
            placement: PanelPlacement::Bottom,
            basic: true,
        }],
        settings_pages: vec![SettingsPageContribution {
            page_id: text("acme_tools.settings"),
            title: text("Acme"),
        }],
        themes: vec![ThemeContribution {
            theme_id: text("acme_tools.night"),
            title: text("Night"),
            tokens_file: text("themes/night.theme.json"),
        }],
        icon_sets: untyped.clone(),
        locales: untyped.clone(),
        macros: untyped.clone(),
        layouts: untyped.clone(),
        keybindings: untyped.clone(),
        templates: untyped.clone(),
        file_types: untyped.clone(),
        importers: untyped.clone(),
        exporters: untyped.clone(),
        converters: untyped.clone(),
        data_sources: untyped.clone(),
        recipes: untyped.clone(),
        reward_functions: untyped.clone(),
        node_types: untyped.clone(),
        event_handlers: vec![EventHandlerContribution {
            event_type: text("checkpoint.saved"),
            command_id: text("acme_tools.refresh"),
        }],
        chat_templates: untyped.clone(),
        tokenizers: untyped.clone(),
        samplers: untyped.clone(),
        sandbox_images: untyped.clone(),
        lenses: untyped.clone(),
        sae_loaders: untyped.clone(),
        metrics: untyped.clone(),
        benchmark_loaders: untyped.clone(),
        adapters: untyped.clone(),
        assistant_skills: untyped.clone(),
        compute_targets: untyped,
    }
}

fn filled_manifest() -> PluginManifest {
    PluginManifest {
        header: header_of_type("rastai.plugin"),
        plugin_id: text("acme_tools"),
        display_name: text("Acme tools"),
        version: text("1.2.0"),
        engines: PluginEngines { rastai: text("^1.0") },
        requires_capabilities: vec![text("commands@1")],
        contributes: filled_contributions(),
        permissions: vec![text("commands:[plugin:acme_tools]")],
        ui: Some(PluginUi {
            entry: text("index.html"),
            kind: PluginUiKind::InProcess,
        }),
        python: Some(PluginPython {
            module: text("acme_tools"),
            lockfile: text("uv.lock"),
        }),
        provider: Some(PluginProvider {
            binaries: map_of(&[("linux-x86_64", "bin/acme")]),
        }),
        dependencies: map_of(&[("psychology", "^1.0")]),
        catalog: text("catalog"),
        experimental: true,
    }
}

#[test]
fn the_plugin_manifest_and_its_parts_match_their_descriptions() {
    let manifest = filled_manifest();
    assert_description_matches(&manifest, "PluginManifest");
    assert_description_matches(&manifest.engines, "PluginEngines");
    let contributes = &manifest.contributes;
    assert_description_matches(contributes, "PluginContributions");
    assert_description_matches(&contributes.commands[0], "CommandContribution");
    assert_description_matches(&contributes.sections[0], "SectionContribution");
    assert_description_matches(&contributes.panels[0], "PanelContribution");
    assert_description_matches(&contributes.settings_pages[0], "SettingsPageContribution");
    assert_description_matches(&contributes.themes[0], "ThemeContribution");
    assert_description_matches(&contributes.event_handlers[0], "EventHandlerContribution");
    assert_description_matches(manifest.ui.as_ref().unwrap(), "PluginUi");
    assert_description_matches(manifest.python.as_ref().unwrap(), "PluginPython");
    assert_description_matches(manifest.provider.as_ref().unwrap(), "PluginProvider");
}

// Theme: every color role set.

fn every_color_role() -> ThemeColors {
    let mut members = serde_json::Map::new();
    for (role, _) in COLOR_ROLES {
        members.insert(text(role), json!("#1f6feb"));
    }
    serde_json::from_value(Value::Object(members)).unwrap()
}

fn filled_theme() -> ThemeDocument {
    let spacing = |steps: &[u32]| ThemeSpacing {
        spacing_px: steps.to_vec(),
    };
    ThemeDocument {
        header: header_of_type("rastai.theme"),
        name: text("Night"),
        base: ThemeBase::Dark,
        colors: every_color_role(),
        typography: ThemeTypography {
            font_family_ui: text("Inter, sans-serif"),
            font_family_mono: text("monospace"),
            font_weights: ThemeFontWeights {
                regular: 400,
                medium: 500,
                bold: 700,
            },
            font_sizes_px: ThemeFontSizes {
                xs: 11,
                sm: 12,
                md: 14,
                lg: 16,
                xl: 20,
            },
            line_heights: ThemeLineHeights {
                body: 1.5,
                heading: 1.25,
                code: 1.4,
            },
        },
        density: ThemeDensity {
            comfortable: spacing(&[0, 4, 8, 16]),
            compact: spacing(&[0, 2, 4, 8]),
        },
        radius_px: ThemeRadius {
            small: 2,
            medium: 4,
            large: 8,
        },
        motion: ThemeMotion {
            fast_ms: 120,
            slow_ms: 240,
            easing: text("ease-out"),
        },
        focus_ring: ThemeFocusRing {
            width_px: 2,
            offset_px: 2,
        },
        elevation: ThemeElevation {
            level_1: text("0 1px 2px black"),
            level_2: text("0 2px 8px black"),
            level_3: text("0 8px 24px black"),
        },
    }
}

#[test]
fn the_theme_and_every_token_group_match_their_descriptions() {
    let theme = filled_theme();
    assert_description_matches(&theme, "ThemeDocument");
    assert_description_matches(&theme.colors, "ThemeColors");
    assert_description_matches(&theme.typography, "ThemeTypography");
    assert_description_matches(&theme.typography.font_weights, "ThemeFontWeights");
    assert_description_matches(&theme.typography.font_sizes_px, "ThemeFontSizes");
    assert_description_matches(&theme.typography.line_heights, "ThemeLineHeights");
    assert_description_matches(&theme.density, "ThemeDensity");
    assert_description_matches(&theme.density.comfortable, "ThemeSpacing");
    assert_description_matches(&theme.radius_px, "ThemeRadius");
    assert_description_matches(&theme.motion, "ThemeMotion");
    assert_description_matches(&theme.focus_ring, "ThemeFocusRing");
    assert_description_matches(&theme.elevation, "ThemeElevation");
}

#[test]
fn every_color_role_is_set_in_the_filled_theme() {
    // A role missing from ThemeColors would fail to read back above; one missing from COLOR_ROLES would be left
    // unset here and dropped from the written value.
    let written = serde_json::to_value(every_color_role()).unwrap();
    assert_eq!(written.as_object().unwrap().len(), COLOR_ROLES.len());
}

// Every registered document type is covered above.

#[test]
fn every_document_type_has_a_case_here() {
    let covered = [
        "RunRecord",
        "ResumeState",
        "SettingsDocument",
        "Project",
        "LockFile",
        "PluginManifest",
        "ThemeDocument",
    ];
    let registered: Vec<&str> = DOCUMENT_TYPES
        .iter()
        .map(|row: &DocumentType| row.description_name)
        .collect();
    assert_eq!(registered, covered);
}
