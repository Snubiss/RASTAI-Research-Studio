//! The plugin host (layout: rust-control-plane.md 12; flows/platform.md 7.2 and 7.3): the capability table against
//! the manifest description, negotiation as a table of manifests with no disk, then the fixture plugins of
//! `fixtures/plugins/` through discovery, negotiation and registration in a running daemon.
//!
//! Also the owner-only settings keys of contracts 11.1 (threat model R27): a `system.admin` token or a plugin is
//! refused with `RRS-C1005`, the owner isn't.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};

use rastai_core::paths::{DataPaths, daemon_events_log_path};
use rastai_core::principal::owner_principal;
use rastai_core::scopes::Scope;
use rastai_core::states::plugin_state_name;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::documents::plugin_manifest::PluginManifest;
use rastai_formats::semver::parse_version;
use rastai_proto::commands::v1::ExecuteRequest;
use rastai_proto::common::v1::PluginState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, PluginStateChanged};
use rastai_server::daemon::{DaemonOptions, RunningDaemon, start_daemon};
use rastai_server::events::bus::subscribe;
use rastai_server::http::connect_json::list_commands_response_from_json;
use rastai_server::plugin_host::capabilities::{
    FIRST_PARTY_CAPABILITIES, HOST_CAPABILITIES, available_capabilities, negotiate_capabilities,
    negotiation_diagnostics, state_after_negotiation,
};
use rastai_server::plugin_host::load::{disable_plugin, load_plugin};
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
use rastai_store::index::kept_tables::{PluginRow, get_plugin};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use serde_json::{Value, json};

use common::{
    ANY_FREE_PORT, execute_request, ok_result, options_on_any_port, owner_bearer, remove_test_folder, response_codes,
    run_command, send_to_port, stop_test_daemon, test_paths, token_principal,
};

const LIST_COMMANDS_PATH: &str = "/rastai.commands.v1.CommandService/ListCommands";
const HEALTH_PATH: &str = "/api/health";
const CONTRIBUTIONS_TYPE: &str = "PluginContributions";
const MANIFEST_FILE_NAME: &str = "rastai-plugin.json";

/// The fixture folders, each with its plugin id, the state it must end in and the codes of its diagnostics.
const FIXTURES: &[(&str, &str, &str, &[&str])] = &[
    ("valid", "fixture_tools", "enabled", &[]),
    (
        "bad-manifest",
        "bad-manifest",
        "failed",
        &["RRS-P0001", "RRS-E1001", "RRS-E1002"],
    ),
    ("missing-capability", "fixture_lens", "incompatible", &["RRS-P1002"]),
    ("reserved-id", "model", "incompatible", &["RRS-P0002"]),
    (
        "command-outside-namespace",
        "fixture_outside",
        "incompatible",
        &["RRS-P0004"],
    ),
    ("theme", "fixture_theme", "incompatible", &["RRS-P1002"]),
    ("scope-beyond-grant", "fixture_reach", "enabled", &["RRS-C1005"]),
];

// ---- The table and negotiation, without disk ----

#[test]
fn every_contribution_key_has_exactly_one_capability_row() {
    let description = describe_all_types()
        .into_iter()
        .find(|item| item.name == CONTRIBUTIONS_TYPE)
        .unwrap();
    let mut manifest_keys: Vec<String> = description.fields.iter().map(|field| field.name.clone()).collect();
    let mut table_keys: Vec<String> = HOST_CAPABILITIES
        .iter()
        .map(|row| row.contribution_key.to_string())
        .chain(
            FIRST_PARTY_CAPABILITIES
                .iter()
                .map(|row| row.contribution_key.to_string()),
        )
        .collect();
    manifest_keys.sort();
    table_keys.sort();
    assert_eq!(table_keys, manifest_keys);
}

/// A manifest that negotiates cleanly; each row of the table changes one thing.
fn base_manifest() -> Value {
    json!({
        "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
        "type": "rastai.plugin",
        "schema_version": 1,
        "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
        "plugin_id": "acme_tools",
        "display_name": "Acme tools",
        "version": "1.0.0",
        "engines": {"rastai": ">=0.1"},
        "requires_capabilities": ["commands@1"],
        "contributes": {"commands": [
            {"command_id": "acme_tools.run", "title": "Run", "kind": "sync", "arguments_type": "A", "result_type": "R"}
        ]},
        "permissions": ["commands:[read]"]
    })
}

struct NegotiationCase {
    name: &'static str,
    /// Top-level fields replaced in the base manifest.
    changes: Value,
    enabled_first_party: &'static [&'static str],
    /// First-party plugins the host knows but that aren't enabled.
    disabled_first_party: &'static [&'static str],
    /// Enabled plugins and their versions.
    enabled_plugins: &'static [(&'static str, &'static str)],
    granted_permissions: &'static [&'static str],
    experimental_allowed: bool,
    state: PluginState,
    codes: &'static [&'static str],
}

fn case(name: &'static str, changes: Value, state: PluginState, codes: &'static [&'static str]) -> NegotiationCase {
    NegotiationCase {
        name,
        changes,
        enabled_first_party: &[],
        disabled_first_party: &[],
        enabled_plugins: &[],
        granted_permissions: &["commands:[read]"],
        experimental_allowed: false,
        state,
        codes,
    }
}

/// Every row of the negotiation table, in the order of flows/platform.md 7.3.
fn negotiation_cases() -> Vec<NegotiationCase> {
    let mut cases = engine_and_capability_cases();
    cases.extend(unprovided_capability_cases());
    cases.extend(experimental_and_first_party_cases());
    cases.extend(permission_and_dependency_cases());
    cases
}

fn engine_and_capability_cases() -> Vec<NegotiationCase> {
    vec![
        case("loads", json!({}), PluginState::Enabled, &[]),
        case(
            "engines too new",
            json!({"engines": {"rastai": ">=99"}}),
            PluginState::Incompatible,
            &["RRS-P1001"],
        ),
        case(
            "engines unreadable",
            json!({"engines": {"rastai": "soon"}}),
            PluginState::Incompatible,
            &["RRS-P1001"],
        ),
        case(
            "a capability nobody provides",
            json!({"requires_capabilities": ["commands@1", "psychology.lens@9"]}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        case(
            "a capability without its major",
            json!({"requires_capabilities": ["commands"]}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
    ]
}

/// Capabilities this build doesn't provide yet, and contributions that don't require theirs.
fn unprovided_capability_cases() -> Vec<NegotiationCase> {
    let theme = json!([{"theme_id": "acme_tools.night", "title": "Night", "tokens_file": "night.theme.json"}]);
    vec![
        case(
            "themes before M8",
            json!({"requires_capabilities": ["ui.theme@1"], "contributes": {"themes": theme}}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        case(
            "icon sets before M8",
            json!({"requires_capabilities": ["ui.icon_set@1"], "contributes": {"icon_sets": [{"set_id": "a"}]}}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        case(
            "assistant skills before Phase 2",
            json!({"requires_capabilities": ["assistant.skill@1"], "contributes": {"assistant_skills": [{"skill_id": "a"}]}}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        case(
            "locales before Phase 2",
            json!({"requires_capabilities": ["ui.locale@1"], "contributes": {"locales": [{"locale": "fr"}]}}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        case(
            "a contribution without its capability",
            json!({"requires_capabilities": [], "contributes": {"themes": theme}}),
            PluginState::Incompatible,
            &["RRS-P0006"],
        ),
        case(
            "commands without commands@1",
            json!({"requires_capabilities": ["ui.panel@1"]}),
            PluginState::Incompatible,
            &["RRS-P0006"],
        ),
    ]
}

fn experimental_and_first_party_cases() -> Vec<NegotiationCase> {
    vec![
        case(
            "an experimental API with the flag off",
            json!({"requires_capabilities": ["commands@1", "data.source@1"]}),
            PluginState::Disabled,
            &["RRS-P1003"],
        ),
        NegotiationCase {
            experimental_allowed: true,
            ..case(
                "an experimental API with the flag on",
                json!({"requires_capabilities": ["commands@1", "data.source@1"]}),
                PluginState::Enabled,
                &[],
            )
        },
        case(
            "a lens without Psychology",
            json!({"requires_capabilities": ["psychology.lens@1"], "contributes": {"lenses": [{"lens_id": "x"}]}}),
            PluginState::Incompatible,
            &["RRS-P1002"],
        ),
        NegotiationCase {
            enabled_first_party: &["psychology"],
            ..case(
                "a lens with Psychology enabled",
                json!({"requires_capabilities": ["psychology.lens@1"], "contributes": {"lenses": [{"lens_id": "x"}]}}),
                PluginState::Enabled,
                &[],
            )
        },
        // Flows/platform.md 7.3: disabling Psychology disables lens plugins; enabling it again is the fix.
        NegotiationCase {
            disabled_first_party: &["psychology"],
            ..case(
                "a lens while Psychology is disabled",
                json!({"requires_capabilities": ["psychology.lens@1"], "contributes": {"lenses": [{"lens_id": "x"}]}}),
                PluginState::Disabled,
                &["RRS-P1002"],
            )
        },
        NegotiationCase {
            disabled_first_party: &["psychology"],
            ..case(
                "a lens major Psychology never provides",
                json!({"requires_capabilities": ["psychology.lens@9"], "contributes": {"lenses": [{"lens_id": "x"}]}}),
                PluginState::Incompatible,
                &["RRS-P1002"],
            )
        },
    ]
}

fn permission_and_dependency_cases() -> Vec<NegotiationCase> {
    vec![
        NegotiationCase {
            granted_permissions: &[],
            ..case("a permission never granted", json!({}), PluginState::AwaitingTrust, &[])
        },
        case(
            "a dependency that isn't there",
            json!({"dependencies": {"acme_base": "^1.0"}}),
            PluginState::Disabled,
            &["RRS-P3001"],
        ),
        NegotiationCase {
            enabled_plugins: &[("acme_base", "1.4.0")],
            ..case(
                "a dependency in range",
                json!({"dependencies": {"acme_base": "^1.0"}}),
                PluginState::Enabled,
                &[],
            )
        },
        NegotiationCase {
            enabled_plugins: &[("acme_base", "2.0.0")],
            ..case(
                "a dependency out of range",
                json!({"dependencies": {"acme_base": "^1.0"}}),
                PluginState::Disabled,
                &["RRS-P3001"],
            )
        },
    ]
}

fn manifest_with(changes: &Value) -> PluginManifest {
    let mut manifest = base_manifest();
    for (key, value) in changes.as_object().unwrap() {
        manifest[key] = value.clone();
    }
    serde_json::from_value(manifest).unwrap()
}

#[test]
fn negotiation_table() {
    for case in negotiation_cases() {
        let manifest = manifest_with(&case.changes);
        let first_party: Vec<String> = case.enabled_first_party.iter().map(|id| id.to_string()).collect();
        let disabled: Vec<String> = case.disabled_first_party.iter().map(|id| id.to_string()).collect();
        let available = available_capabilities(&first_party, &disabled);
        let enabled: Vec<_> = case
            .enabled_plugins
            .iter()
            .map(|(plugin_id, version)| (plugin_id.to_string(), parse_version(version).unwrap()))
            .collect();
        let granted: Vec<String> = case.granted_permissions.iter().map(|text| text.to_string()).collect();
        let result = negotiate_capabilities(&manifest, &available, &enabled, &granted);
        let state = state_after_negotiation(&result, case.experimental_allowed);
        assert_eq!(state, case.state, "{}", case.name);
        let diagnostics = negotiation_diagnostics(&manifest, &result, case.experimental_allowed);
        let codes: Vec<&str> = diagnostics.iter().map(|item| item.code.as_str()).collect();
        assert_eq!(codes, case.codes, "{}", case.name);
    }
}

#[test]
fn a_theme_is_refused_by_name_until_m8() {
    let manifest = manifest_with(&json!({
        "requires_capabilities": ["ui.theme@1"],
        "contributes": {"themes": [{"theme_id": "a.b", "title": "B", "tokens_file": "b.theme.json"}]}
    }));
    let result = negotiate_capabilities(&manifest, &available_capabilities(&[], &[]), &[], &[]);
    assert_eq!(result.missing_capabilities, ["ui.theme@1"]);
    let refusal = &negotiation_diagnostics(&manifest, &result, false)[0];
    assert!(refusal.message.contains("ui.theme@1"), "{}", refusal.message);
}

// ---- The fixtures through a running daemon ----

fn fixture_folder(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/plugins")
        .join(name)
}

/// `--set plugins.dev_paths=[...]` with the named fixture folders.
fn options_with_dev_plugins(names: &[&str]) -> DaemonOptions {
    let folders: Vec<PathBuf> = names.iter().map(|name| fixture_folder(name)).collect();
    options_with_dev_folders(&folders)
}

fn options_with_dev_folders(folders: &[PathBuf]) -> DaemonOptions {
    let folders: Vec<String> = folders.iter().map(|folder| folder.display().to_string()).collect();
    DaemonOptions {
        port: Some(ANY_FREE_PORT),
        set_flags: vec![format!("plugins.dev_paths={}", json!(folders))],
        ..DaemonOptions::default()
    }
}

async fn plugin_list(running: &RunningDaemon) -> Vec<Value> {
    let response = run_command(
        &running.server,
        &owner_principal(),
        execute_request("plugin.list", "{}"),
    )
    .await;
    ok_result(&response)["plugins"].as_array().unwrap().clone()
}

fn find_listed<'a>(plugins: &'a [Value], plugin_id: &str) -> &'a Value {
    plugins
        .iter()
        .find(|plugin| plugin["plugin_id"] == plugin_id)
        .unwrap_or_else(|| panic!("{plugin_id} isn't listed"))
}

fn listed_codes(plugin: &Value) -> Vec<String> {
    let diagnostics = plugin["diagnostics"].as_array().unwrap();
    diagnostics
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_string())
        .collect()
}

/// The command ids `ListCommands` returns for one section, over HTTP with the owner token.
async fn listed_command_ids(
    running: &RunningDaemon,
    paths: &rastai_core::paths::DataPaths,
    section: &str,
) -> Vec<String> {
    let bearer = owner_bearer(paths);
    let headers = [("Authorization", bearer.as_str()), ("Content-Type", "application/json")];
    let body = json!({"section": section}).to_string();
    let response = send_to_port(running.http_port, "POST", LIST_COMMANDS_PATH, &headers, body.as_bytes()).await;
    assert_eq!(response.status, 200);
    let listed = list_commands_response_from_json(&response.body).unwrap();
    listed.commands.into_iter().map(|command| command.command_id).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn each_fixture_ends_in_its_state_and_the_daemon_keeps_running() {
    let paths = test_paths("plugin-fixtures");
    let names: Vec<&str> = FIXTURES.iter().map(|(folder, _, _, _)| *folder).collect();
    let running = start_daemon(paths.clone(), &options_with_dev_plugins(&names))
        .await
        .unwrap();
    let health = send_to_port(running.http_port, "GET", HEALTH_PATH, &[], b"").await;
    assert_eq!(health.status, 200);
    let plugins = plugin_list(&running).await;
    assert_eq!(plugins.len(), FIXTURES.len());
    for (folder, plugin_id, state, codes) in FIXTURES {
        let plugin = find_listed(&plugins, plugin_id);
        assert_eq!(plugin["state"], *state, "{folder}");
        assert_eq!(plugin["source"], "dev", "{folder}");
        let listed = listed_codes(plugin);
        for code in *codes {
            assert!(listed.contains(&code.to_string()), "{folder}: {listed:?}");
        }
        if codes.is_empty() {
            assert!(listed.is_empty(), "{folder}: {listed:?}");
        }
    }
    // The failed plugin's first diagnostic is RRS-P0001; its manifest is listed as an empty object.
    let broken = find_listed(&plugins, "bad-manifest");
    assert_eq!(listed_codes(broken)[0], "RRS-P0001");
    let refusal = broken["diagnostics"][0]["message"].as_str().unwrap();
    assert!(refusal.ends_with('.') && !refusal.ends_with(".."), "{refusal}");
    assert_eq!(broken["manifest"], json!({}));
    // Nothing of a plugin that isn't enabled is registered; a command beyond the grant isn't either.
    for section in ["model", "fixture_outside", "fixture_theme", "fixture_lens"] {
        assert!(
            listed_command_ids(&running, &paths, section).await.is_empty(),
            "{section}"
        );
    }
    assert_eq!(
        listed_command_ids(&running, &paths, "fixture_reach").await,
        ["fixture_reach.peek"]
    );
    // Every state change is in the daemon log, with its reason, and none is left waiting in the host.
    let state_changes = logged_plugin_state_changes(&paths);
    let theme_change = state_changes
        .iter()
        .find(|change| change.plugin_id == "fixture_theme")
        .unwrap();
    assert_eq!(theme_change.to_state, PluginState::Incompatible as i32);
    assert_eq!(theme_change.diagnostic.as_ref().unwrap().code, "RRS-P1002");
    for (_folder, plugin_id, _state, _codes) in FIXTURES {
        assert!(
            state_changes.iter().any(|change| change.plugin_id == *plugin_id),
            "{plugin_id}"
        );
    }
    assert!(running.server.plugins.read().unwrap().state_changes.is_empty());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dev_plugins_commands_come_and_go_with_its_state() {
    let paths = test_paths("plugin-dev");
    let running = start_daemon(paths.clone(), &options_with_dev_plugins(&["valid"]))
        .await
        .unwrap();
    let mut live = subscribe(&running.server.bus);
    let mut listed = listed_command_ids(&running, &paths, "fixture_tools").await;
    listed.sort();
    assert_eq!(listed, ["fixture_tools.refresh", "fixture_tools.report.run"]);
    // The command is registered and its plugin enabled: it reaches the plugin host, whose forwarding is M4.
    let call = execute_request("fixture_tools.refresh", "{}");
    let response = run_command(&running.server, &owner_principal(), call.clone()).await;
    assert_eq!(response_codes(&response), ["RRS-P1004"]);

    assert_eq!(
        disable_plugin(&running.server, "fixture_tools").await,
        PluginState::Disabled
    );
    assert!(listed_command_ids(&running, &paths, "fixture_tools").await.is_empty());
    let response = run_command(&running.server, &owner_principal(), call.clone()).await;
    assert_eq!(response_codes(&response), ["RRS-C0001"]);
    let plugins = plugin_list(&running).await;
    assert_eq!(find_listed(&plugins, "fixture_tools")["state"], "disabled");
    // The disable's own move is logged and published before anything else changes a state: no other enabled
    // plugin is left to check again, so no later call would send it.
    let disabled_move = [(PluginState::Enabled as i32, PluginState::Disabled as i32)];
    let logged: Vec<(i32, i32)> = logged_plugin_state_changes(&paths)
        .iter()
        .skip(1)
        .map(|change| (change.from_state, change.to_state))
        .collect();
    assert_eq!(logged, disabled_move);
    let published: Vec<(i32, i32)> = published_plugin_state_changes(&mut live)
        .iter()
        .map(|change| (change.from_state, change.to_state))
        .collect();
    assert_eq!(published, disabled_move);

    assert_eq!(
        load_plugin(&running.server, "fixture_tools").await,
        PluginState::Enabled
    );
    assert_eq!(listed_command_ids(&running, &paths, "fixture_tools").await.len(), 2);
    // Each move is in the daemon log in order; the two made after the start also reached the bus (the disable's
    // was taken above), as durable records of the daemon.
    let moves: Vec<(i32, i32)> = logged_plugin_state_changes(&paths)
        .iter()
        .map(|change| (change.from_state, change.to_state))
        .collect();
    let expected = [
        (PluginState::Installed as i32, PluginState::Enabled as i32),
        (PluginState::Enabled as i32, PluginState::Disabled as i32),
        (PluginState::Disabled as i32, PluginState::Enabled as i32),
    ];
    assert_eq!(moves, expected);
    let published: Vec<(i32, i32)> = published_plugin_state_changes(&mut live)
        .iter()
        .map(|change| (change.from_state, change.to_state))
        .collect();
    assert_eq!(published, expected[2..]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_acts_only_within_its_grant_and_only_while_enabled() {
    let paths = test_paths("plugin-grant");
    let running = start_daemon(paths.clone(), &options_with_dev_plugins(&["valid"]))
        .await
        .unwrap();
    let as_plugin = |command_id: &str, arguments: &str| ExecuteRequest {
        acting_plugin_id: "fixture_tools".to_string(),
        ..execute_request(command_id, arguments)
    };
    // Granted `read`: the owner's panel for this plugin may read, but not change settings.
    let read = run_command(&running.server, &owner_principal(), as_plugin("system.status", "{}")).await;
    assert!(read.ok, "{:?}", read.diagnostics);
    let set_theme = r#"{"key": "ui.theme", "value": "dark"}"#;
    let refused = run_command(
        &running.server,
        &owner_principal(),
        as_plugin("settings.set", set_theme),
    )
    .await;
    assert_eq!(response_codes(&refused), ["RRS-C1005"]);
    // A token keeps only what it holds itself, even acting as the plugin.
    let chat_token = token_principal(vec![Scope::Chat]);
    let narrowed = run_command(&running.server, &chat_token, as_plugin("system.status", "{}")).await;
    assert_eq!(response_codes(&narrowed), ["RRS-C1005"]);
    // A disabled plugin holds nothing.
    disable_plugin(&running.server, "fixture_tools").await;
    let after = run_command(&running.server, &owner_principal(), as_plugin("system.status", "{}")).await;
    assert_eq!(response_codes(&after), ["RRS-C1005"]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_plugin_loads_from_its_row_and_its_state_is_saved() {
    let paths = test_paths("plugin-installed");
    // A first start makes rastai.db; the row and the folder an install would leave are written while it runs.
    let first = start_daemon(
        paths.clone(),
        &DaemonOptions {
            port: Some(ANY_FREE_PORT),
            ..DaemonOptions::default()
        },
    )
    .await
    .unwrap();
    let row = PluginRow {
        plugin_id: "fixture_tools".to_string(),
        version: "1.0.0".to_string(),
        state: plugin_state_name(PluginState::Installed).to_string(),
        content_hash: "blake3:00".to_string(),
        granted_permissions: vec!["commands:[read, plugin:fixture_tools]".to_string()],
        source: "installed".to_string(),
    };
    write_index(&first.server.store.index, IndexWrite::UpsertPlugin(row))
        .await
        .unwrap();
    stop_test_daemon(first).await;
    let installed_folder = paths.data.join("plugins/fixture_tools/1.0.0");
    std::fs::create_dir_all(&installed_folder).unwrap();
    std::fs::copy(
        fixture_folder("valid").join("rastai-plugin.json"),
        installed_folder.join("rastai-plugin.json"),
    )
    .unwrap();

    let second = start_daemon(
        paths.clone(),
        &DaemonOptions {
            port: Some(ANY_FREE_PORT),
            ..DaemonOptions::default()
        },
    )
    .await
    .unwrap();
    let plugins = plugin_list(&second).await;
    let listed = find_listed(&plugins, "fixture_tools");
    assert_eq!(listed["state"], "enabled");
    assert_eq!(listed["source"], "installed");
    assert_eq!(
        listed["granted_permissions"],
        json!(["commands:[read, plugin:fixture_tools]"])
    );
    let saved = {
        let reader = open_index_reader(&second.server.store.index_readers).unwrap();
        get_plugin(&reader, "fixture_tools").unwrap().unwrap()
    };
    assert_eq!(saved.state, "enabled");
    assert_eq!(saved.content_hash, "blake3:00");
    stop_test_daemon(second).await;
    assert_a_restart_logs_no_second_move(&paths).await;
    remove_test_folder(&paths);
}

/// A restart starts `fixture_tools` from its saved `enabled`: the load changes nothing, so the daemon log still has
/// only the move of its first load.
async fn assert_a_restart_logs_no_second_move(paths: &DataPaths) {
    let restarted = start_daemon(paths.clone(), &options_on_any_port()).await.unwrap();
    assert_eq!(listed_state(&restarted, "fixture_tools").await.0, "enabled");
    let moves: Vec<(i32, i32)> = logged_plugin_state_changes(paths)
        .iter()
        .filter(|change| change.plugin_id == "fixture_tools")
        .map(|change| (change.from_state, change.to_state))
        .collect();
    assert_eq!(moves, [(PluginState::Installed as i32, PluginState::Enabled as i32)]);
    stop_test_daemon(restarted).await;
}

/// The `plugin.state_changed` records in `<data>/daemon/events.log`, in log order.
fn logged_plugin_state_changes(paths: &DataPaths) -> Vec<PluginStateChanged> {
    let mut reader = open_event_log(&daemon_events_log_path(paths)).unwrap();
    let mut changes = Vec::new();
    while let FrameRead::Record(record) = read_next_frame(&mut reader).unwrap() {
        if let Some(change) = plugin_state_change_of(record.payload.and_then(|payload| payload.payload)) {
            changes.push(change);
        }
    }
    changes
}

/// The `plugin.state_changed` events the bus has published to `live` so far, from the daemon.
fn published_plugin_state_changes(live: &mut tokio::sync::broadcast::Receiver<BusEvent>) -> Vec<PluginStateChanged> {
    let mut changes = Vec::new();
    while let Ok(event) = live.try_recv() {
        assert_eq!(event.source, "daemon");
        if let Some(change) = plugin_state_change_of(event.payload.and_then(|payload| payload.payload)) {
            changes.push(change);
        }
    }
    changes
}

fn plugin_state_change_of(payload: Option<Payload>) -> Option<PluginStateChanged> {
    match payload {
        Some(Payload::PluginStateChanged(change)) => Some(change),
        _ => None,
    }
}

// ---- Plugins written for one test: dependency chains, a lost first-party capability, saved states ----

/// A small plugin with one command, `<plugin_id>.run`; `changes` replaces top-level fields.
fn small_manifest(plugin_id: &str, changes: Value) -> Value {
    let mut manifest = json!({
        "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
        "type": "rastai.plugin",
        "schema_version": 1,
        "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
        "plugin_id": plugin_id,
        "display_name": plugin_id,
        "version": "1.0.0",
        "engines": {"rastai": ">=0.1"},
        "requires_capabilities": ["commands@1"],
        "contributes": {"commands": [run_command_contribution(plugin_id)]}
    });
    for (key, value) in changes.as_object().unwrap() {
        manifest[key] = value.clone();
    }
    manifest
}

fn run_command_contribution(plugin_id: &str) -> Value {
    json!({
        "command_id": format!("{plugin_id}.run"),
        "title": "Run",
        "kind": "sync",
        "arguments_type": "RunArgs",
        "result_type": "RunResult"
    })
}

/// Writes the manifest into `<test root>/dev-plugins/<plugin_id>/` and returns the folder.
fn write_dev_plugin(paths: &DataPaths, manifest: &Value) -> PathBuf {
    let plugin_id = manifest["plugin_id"].as_str().unwrap();
    let folder = paths.data.parent().unwrap().join("dev-plugins").join(plugin_id);
    write_manifest(&folder, manifest);
    folder
}

fn write_manifest(folder: &Path, manifest: &Value) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join(MANIFEST_FILE_NAME), manifest.to_string()).unwrap();
}

async fn listed_state(running: &RunningDaemon, plugin_id: &str) -> (String, Vec<String>) {
    let plugins = plugin_list(running).await;
    let plugin = find_listed(&plugins, plugin_id);
    (plugin["state"].as_str().unwrap().to_string(), listed_codes(plugin))
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_the_end_of_a_dependency_chain_disables_the_whole_chain() {
    let paths = test_paths("plugin-chain");
    // acme_a needs acme_b, which needs acme_c. Listed first to last, so discovery's order is the wrong way round.
    let folders = [
        write_dev_plugin(
            &paths,
            &small_manifest("acme_a", json!({"dependencies": {"acme_b": "^1.0"}})),
        ),
        write_dev_plugin(
            &paths,
            &small_manifest("acme_b", json!({"dependencies": {"acme_c": "^1.0"}})),
        ),
        write_dev_plugin(&paths, &small_manifest("acme_c", json!({}))),
    ];
    let running = start_daemon(paths.clone(), &options_with_dev_folders(&folders))
        .await
        .unwrap();
    for plugin_id in ["acme_a", "acme_b", "acme_c"] {
        assert_eq!(listed_state(&running, plugin_id).await.0, "enabled", "{plugin_id}");
    }

    assert_eq!(disable_plugin(&running.server, "acme_c").await, PluginState::Disabled);
    for plugin_id in ["acme_a", "acme_b"] {
        let (state, codes) = listed_state(&running, plugin_id).await;
        assert_eq!(
            (state.as_str(), codes),
            ("disabled", vec!["RRS-P3001".to_string()]),
            "{plugin_id}"
        );
        assert!(
            listed_command_ids(&running, &paths, plugin_id).await.is_empty(),
            "{plugin_id}"
        );
    }
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_cycle_names_the_dependency_held_back() {
    let paths = test_paths("plugin-cycle");
    let folders = [
        write_dev_plugin(
            &paths,
            &small_manifest("acme_a", json!({"dependencies": {"acme_b": "*"}})),
        ),
        write_dev_plugin(
            &paths,
            &small_manifest("acme_b", json!({"dependencies": {"acme_a": "*"}})),
        ),
    ];
    let running = start_daemon(paths.clone(), &options_with_dev_folders(&folders))
        .await
        .unwrap();
    let plugins = plugin_list(&running).await;
    let refusal = find_listed(&plugins, "acme_a")["diagnostics"][0].clone();
    assert_eq!(refusal["code"], "RRS-P3001");
    let message = refusal["message"].as_str().unwrap();
    assert!(message.contains("The dependency acme_b of acme_a"), "{message}");
    // Every discovered plugin is held back, so no load follows the refusals: each sends its own record.
    let mut logged: Vec<(String, i32, String)> = logged_plugin_state_changes(&paths)
        .into_iter()
        .map(|change| {
            let code = change.diagnostic.map(|item| item.code).unwrap_or_default();
            (change.plugin_id, change.to_state, code)
        })
        .collect();
    logged.sort();
    let disabled = PluginState::Disabled as i32;
    let expected = [
        ("acme_a".to_string(), disabled, "RRS-P3001".to_string()),
        ("acme_b".to_string(), disabled, "RRS-P3001".to_string()),
    ];
    assert_eq!(logged, expected);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_psychology_disables_a_lens_plugin_until_it_is_back() {
    let paths = test_paths("plugin-lens");
    let lens = small_manifest(
        "acme_lens",
        json!({
            "requires_capabilities": ["commands@1", "psychology.lens@1"],
            "contributes": {
                "commands": [run_command_contribution("acme_lens")],
                "lenses": [{"lens_id": "acme_lens.norms", "title": "Norms"}]
            }
        }),
    );
    let folders = [
        write_dev_plugin(&paths, &lens),
        write_dev_plugin(&paths, &small_manifest("psychology", json!({}))),
    ];
    let running = start_daemon(paths.clone(), &options_with_dev_folders(&folders))
        .await
        .unwrap();
    assert_eq!(listed_state(&running, "acme_lens").await.0, "enabled");
    assert_eq!(
        listed_command_ids(&running, &paths, "acme_lens").await,
        ["acme_lens.run"]
    );

    // Flows/platform.md 7.3: disabling Psychology disables lens plugins with RRS-P1002, and their contributions go.
    disable_plugin(&running.server, "psychology").await;
    let (state, codes) = listed_state(&running, "acme_lens").await;
    assert_eq!((state.as_str(), codes), ("disabled", vec!["RRS-P1002".to_string()]));
    assert!(listed_command_ids(&running, &paths, "acme_lens").await.is_empty());

    // Disabled, not incompatible: once Psychology is back, the lens plugin loads again.
    assert_eq!(load_plugin(&running.server, "psychology").await, PluginState::Enabled);
    assert_eq!(load_plugin(&running.server, "acme_lens").await, PluginState::Enabled);
    assert_eq!(
        listed_command_ids(&running, &paths, "acme_lens").await,
        ["acme_lens.run"]
    );
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

fn saved_plugin_state(running: &RunningDaemon, plugin_id: &str) -> String {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_plugin(&reader, plugin_id).unwrap().unwrap().state
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_the_host_disabled_loads_after_its_fix_and_a_restart() {
    let paths = test_paths("plugin-host-decided");
    let first = start_daemon(paths.clone(), &options_on_any_port()).await.unwrap();
    let row = PluginRow {
        plugin_id: "acme_stream".to_string(),
        version: "1.0.0".to_string(),
        state: plugin_state_name(PluginState::Installed).to_string(),
        content_hash: "blake3:00".to_string(),
        granted_permissions: Vec::new(),
        source: "installed".to_string(),
    };
    write_index(&first.server.store.index, IndexWrite::UpsertPlugin(row))
        .await
        .unwrap();
    stop_test_daemon(first).await;
    let experimental = small_manifest(
        "acme_stream",
        json!({"requires_capabilities": ["commands@1", "data.source@1"]}),
    );
    write_manifest(&paths.data.join("plugins/acme_stream/1.0.0"), &experimental);

    // With experimental APIs off, the host disables it; that is its decision, so the row isn't changed.
    let second = start_daemon(paths.clone(), &options_on_any_port()).await.unwrap();
    let (state, codes) = listed_state(&second, "acme_stream").await;
    assert_eq!((state.as_str(), codes), ("disabled", vec!["RRS-P1003".to_string()]));
    assert_eq!(saved_plugin_state(&second, "acme_stream"), "installed");
    stop_test_daemon(second).await;

    // The fix the catalog gives, then a restart: the plugin loads.
    let with_flag = DaemonOptions {
        set_flags: vec!["plugins.experimental_apis=true".to_string()],
        ..options_on_any_port()
    };
    let third = start_daemon(paths.clone(), &with_flag).await.unwrap();
    assert_eq!(listed_state(&third, "acme_stream").await.0, "enabled");
    assert_eq!(saved_plugin_state(&third, "acme_stream"), "enabled");
    stop_test_daemon(third).await;
    remove_test_folder(&paths);
}

// ---- Owner-only settings keys (contracts 11.1, threat model R27) ----

#[tokio::test(flavor = "multi_thread")]
async fn owner_only_settings_keys_refuse_a_system_admin_token_and_a_plugin() {
    let paths = test_paths("plugin-owner-keys");
    let running = start_daemon(paths.clone(), &options_with_dev_plugins(&["valid"]))
        .await
        .unwrap();
    let admin = token_principal(vec![Scope::SystemAdmin]);
    let dev_paths = r#"{"key": "plugins.dev_paths", "value": ["/tmp/elsewhere"]}"#;
    let ssh_binary = r#"{"key": "nodes.ssh_binary", "value": "/tmp/not-ssh"}"#;
    let listen = r#"{"key": "daemon.listen", "value": "0.0.0.0"}"#;
    for arguments in [dev_paths, ssh_binary, listen] {
        let refused = run_command(&running.server, &admin, execute_request("settings.set", arguments)).await;
        assert_eq!(response_codes(&refused), ["RRS-C1005"], "{arguments}");
        let dry_run = ExecuteRequest {
            dry_run: true,
            ..execute_request("settings.set", arguments)
        };
        let refused = run_command(&running.server, &admin, dry_run).await;
        assert_eq!(response_codes(&refused), ["RRS-C1005"], "{arguments}");
    }
    // An ordinary key is still the admin token's to set.
    let theme = run_command(
        &running.server,
        &admin,
        execute_request("settings.set", r#"{"key": "ui.theme", "value": "dark"}"#),
    )
    .await;
    assert!(theme.ok, "{:?}", theme.diagnostics);
    // The owner sets an owner-only key; nothing was written by the refused calls before it.
    let user_file = rastai_core::paths::user_settings_path(&paths);
    let before: Value = serde_json::from_slice(&std::fs::read(&user_file).unwrap()).unwrap();
    assert!(before.get("plugins").is_none(), "{before}");
    let owner = run_command(
        &running.server,
        &owner_principal(),
        execute_request("settings.set", dev_paths),
    )
    .await;
    assert!(owner.ok, "{:?}", owner.diagnostics);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
