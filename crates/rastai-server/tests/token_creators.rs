//! Token creators and revocation (contracts 8.2 "Token creators and revocation", threat model R28): every token
//! records who made it, revoking a token revokes what it made, disabling a plugin or starting without it revokes what
//! the plugin made, a create never outlives its creator's revoke, and only the owner may act as a plugin.
//!
//! Each test starts a daemon on its own temporary folders. Tokens are made and used the way a script does, with a
//! bearer over the loopback port; a revoked one is checked there and on `/mcp`, where an agent would use it.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rastai_core::paths::DataPaths;
use rastai_core::principal::{Principal, PrincipalKind, owner_principal};
use rastai_core::scopes::Scope;
use rastai_core::states::plugin_state_name;
use rastai_proto::commands::v1::ExecuteResponse;
use rastai_proto::common::v1::PluginState;
use rastai_server::daemon::{DaemonOptions, RunningDaemon, start_daemon};
use rastai_server::http::connect_json::execute_response_from_json;
use rastai_server::plugin_host::load::disable_plugin;
use rastai_server::server_state::Server;
use rastai_store::index::kept_tables::{PluginRow, get_plugin, list_tokens};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use rastai_transport::http_client::HttpResponse;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use common::{
    ANY_FREE_PORT, created_token_principal, execute_request, ok_result, owner_bearer, refusal_code, remove_test_folder,
    response_codes, run_command, send_to_port, start_test_daemon, stop_test_daemon, test_paths,
};

const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
const MCP_PATH: &str = "/mcp";
const JSON: &str = "application/json";
const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client"}}}"#;
const CODE_TOKEN_REVOKED: &str = "RRS-C1006";
const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
const UNAUTHORIZED: u16 = 401;
/// The scopes every token here is made with: `system.admin` lets it make tokens, `read` lets it list.
const ADMIN_SCOPES: &[&str] = &["system.admin", "read"];
/// A dev plugin granted `system.admin`, so its UI (the owner narrowed to it) can make tokens.
const ADMIN_PLUGIN_ID: &str = "acme_admin";
const ADMIN_PLUGIN_PERMISSION: &str = "commands:[system.admin, read, plugin:acme_admin]";
const MANIFEST_FILE_NAME: &str = "rastai-plugin.json";
/// How many creates race one revoke of their creator.
const RACING_CREATES: usize = 40;

/// A token as `token.create` hands it out.
struct NewToken {
    token_id: String,
    token: String,
}

// ---- Helpers ----

/// One Connect JSON `Execute` over the loopback port with `Authorization: Bearer <token>`, as a script sends it.
async fn execute_as(
    port: u16,
    token: &str,
    command_id: &str,
    arguments: Value,
    acting_plugin_id: &str,
) -> HttpResponse {
    execute_request_as(port, token, command_id, arguments, acting_plugin_id, false).await
}

async fn execute_request_as(
    port: u16,
    token: &str,
    command_id: &str,
    arguments: Value,
    acting_plugin_id: &str,
    dry_run: bool,
) -> HttpResponse {
    let bearer = format!("Bearer {token}");
    let body = json!({
        "command_id": command_id,
        "arguments_json": arguments.to_string(),
        "acting_plugin_id": acting_plugin_id,
        "dry_run": dry_run,
    });
    let headers = [("Authorization", bearer.as_str()), ("Content-Type", JSON)];
    send_to_port(port, "POST", EXECUTE_PATH, &headers, body.to_string().as_bytes()).await
}

/// The `ExecuteResponse` of a request the gate let through.
fn admitted(response: &HttpResponse) -> ExecuteResponse {
    assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
    execute_response_from_json(&response.body).unwrap()
}

fn owner_token(paths: &DataPaths) -> String {
    owner_bearer(paths).trim_start_matches("Bearer ").to_string()
}

/// `token.create` over HTTP with `ADMIN_SCOPES`, as `token` (narrowed to `acting_plugin_id` when it isn't empty).
async fn create_token_as(port: u16, token: &str, name: &str, acting_plugin_id: &str) -> NewToken {
    let arguments = json!({"name": name, "scopes": ADMIN_SCOPES});
    let response = admitted(&execute_as(port, token, "token.create", arguments, acting_plugin_id).await);
    let created = ok_result(&response);
    NewToken {
        token_id: created["token_id"].as_str().unwrap().to_string(),
        token: created["token"].as_str().unwrap().to_string(),
    }
}

async fn revoke_as_owner(paths: &DataPaths, port: u16, token_id: &str) {
    let arguments = json!({"token_id": token_id});
    let response = admitted(&execute_as(port, &owner_token(paths), "token.revoke", arguments, "").await);
    ok_result(&response);
}

/// Every token's `TokenSummary`, revoked ones included, by id.
async fn listed_tokens(paths: &DataPaths, port: u16) -> Vec<Value> {
    let arguments = json!({"include_revoked": true});
    let response = admitted(&execute_as(port, &owner_token(paths), "token.list", arguments, "").await);
    ok_result(&response)["tokens"].as_array().unwrap().clone()
}

fn summary_of<'a>(tokens: &'a [Value], token_id: &str) -> &'a Value {
    tokens
        .iter()
        .find(|summary| summary["token_id"] == token_id)
        .unwrap_or_else(|| panic!("{token_id} isn't listed"))
}

/// The token still works: over HTTP its `token.list` runs, and `/mcp` opens a session for it.
async fn assert_live(port: u16, token: &NewToken) {
    let response = admitted(&execute_as(port, &token.token, "token.list", json!({}), "").await);
    assert!(response.ok, "{} {:?}", token.token_id, response.diagnostics);
    let opened = post_mcp(port, &token.token).await;
    assert_eq!(opened.status, 200, "{}", token.token_id);
}

/// The token is refused with `RRS-C1006` on its next request, over HTTP and over `/mcp`.
async fn assert_revoked(port: u16, token: &NewToken) {
    let refused = execute_as(port, &token.token, "token.list", json!({}), "").await;
    assert_eq!(refused.status, UNAUTHORIZED, "{}", token.token_id);
    assert_eq!(refusal_code(&refused), CODE_TOKEN_REVOKED, "{}", token.token_id);
    let mcp_refused = post_mcp(port, &token.token).await;
    assert_eq!(mcp_refused.status, UNAUTHORIZED, "{}", token.token_id);
    assert_eq!(refusal_code(&mcp_refused), CODE_TOKEN_REVOKED, "{}", token.token_id);
}

/// An MCP `initialize` over the loopback port, as an agent's first request.
async fn post_mcp(port: u16, token: &str) -> HttpResponse {
    let bearer = format!("Bearer {token}");
    let headers = [
        ("Authorization", bearer.as_str()),
        ("Content-Type", JSON),
        ("Accept", "application/json, text/event-stream"),
    ];
    send_to_port(port, "POST", MCP_PATH, &headers, INITIALIZE.as_bytes()).await
}

/// A dev plugin under the test's root, granted `system.admin` by its manifest (a dev folder runs with what it asks
/// for, contracts 11.1).
fn write_admin_plugin(paths: &DataPaths) -> PathBuf {
    let folder = paths.data.parent().unwrap().join("dev-plugins").join(ADMIN_PLUGIN_ID);
    write_admin_manifest(&folder);
    folder
}

fn write_admin_manifest(folder: &Path) {
    let manifest = json!({
        "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
        "type": "rastai.plugin",
        "schema_version": 1,
        "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
        "plugin_id": ADMIN_PLUGIN_ID,
        "display_name": "Acme admin",
        "version": "1.0.0",
        "engines": {"rastai": ">=0.1"},
        "requires_capabilities": ["commands@1"],
        "contributes": {"commands": [{
            "command_id": format!("{ADMIN_PLUGIN_ID}.run"),
            "title": "Run",
            "kind": "sync",
            "arguments_type": "RunArgs",
            "result_type": "RunResult"
        }]},
        "permissions": [ADMIN_PLUGIN_PERMISSION]
    });
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join(MANIFEST_FILE_NAME), manifest.to_string()).unwrap();
}

fn options_with_dev_folders(folders: &[&Path]) -> DaemonOptions {
    let folders: Vec<String> = folders.iter().map(|folder| folder.display().to_string()).collect();
    DaemonOptions {
        port: Some(ANY_FREE_PORT),
        set_flags: vec![format!("plugins.dev_paths={}", json!(folders))],
        ..DaemonOptions::default()
    }
}

async fn start_with_dev_folders(paths: &DataPaths, folders: &[&Path]) -> RunningDaemon {
    start_daemon(paths.clone(), &options_with_dev_folders(folders))
        .await
        .unwrap()
}

/// What `plugin.list` says the plugin's state is.
async fn plugin_state(running: &RunningDaemon, plugin_id: &str) -> String {
    let response = run_command(
        &running.server,
        &owner_principal(),
        execute_request("plugin.list", "{}"),
    )
    .await;
    let plugins = ok_result(&response)["plugins"].as_array().unwrap().clone();
    let plugin = plugins.iter().find(|plugin| plugin["plugin_id"] == plugin_id).unwrap();
    plugin["state"].as_str().unwrap().to_string()
}

/// The owner's panel for the plugin makes P, and P makes Q: one token recorded as the plugin's, one under it.
async fn plugin_made_tokens(paths: &DataPaths, port: u16) -> (NewToken, NewToken) {
    let plugin_made = create_token_as(port, &owner_token(paths), "plugin-made", ADMIN_PLUGIN_ID).await;
    let under_it = create_token_as(port, &plugin_made.token, "under-it", "").await;
    let tokens = listed_tokens(paths, port).await;
    let creator = format!("plugin:{ADMIN_PLUGIN_ID}");
    assert_eq!(
        summary_of(&tokens, &plugin_made.token_id)["created_by"],
        creator.as_str()
    );
    let made_by_p = format!("token:{}", plugin_made.token_id);
    assert_eq!(
        summary_of(&tokens, &under_it.token_id)["created_by"],
        made_by_p.as_str()
    );
    (plugin_made, under_it)
}

// ---- Revoking a token ----

/// A (the owner's) makes B, B makes C. Revoking A revokes B and C in the same write, with A's time, and each is
/// refused on its next request over HTTP and MCP.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_token_revokes_the_tokens_it_made_and_theirs() {
    let paths = test_paths("tc-chain");
    let running = start_test_daemon(&paths).await;
    let port = running.http_port;
    let token_a = create_token_as(port, &owner_token(&paths), "a", "").await;
    let token_b = create_token_as(port, &token_a.token, "b", "").await;
    let token_c = create_token_as(port, &token_b.token, "c", "").await;
    let tokens = listed_tokens(&paths, port).await;
    assert_eq!(summary_of(&tokens, &token_a.token_id)["created_by"], "owner");
    let made_by_a = format!("token:{}", token_a.token_id);
    let made_by_b = format!("token:{}", token_b.token_id);
    assert_eq!(summary_of(&tokens, &token_b.token_id)["created_by"], made_by_a.as_str());
    assert_eq!(summary_of(&tokens, &token_c.token_id)["created_by"], made_by_b.as_str());
    for token in [&token_a, &token_b, &token_c] {
        assert_live(port, token).await;
    }

    revoke_as_owner(&paths, port, &token_a.token_id).await;

    for token in [&token_a, &token_b, &token_c] {
        assert_revoked(port, token).await;
    }
    let tokens = listed_tokens(&paths, port).await;
    let revoked_at = summary_of(&tokens, &token_a.token_id)["revoked_unix_ms"]
        .as_u64()
        .unwrap();
    assert!(revoked_at > 0);
    for token in [&token_b, &token_c] {
        assert_eq!(summary_of(&tokens, &token.token_id)["revoked_unix_ms"], revoked_at);
    }
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// Revoking B reaches down its chain only: A, which made it, and D, which A made beside it, stay live.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_made_token_leaves_its_creator_and_siblings_live() {
    let paths = test_paths("tc-sibling");
    let running = start_test_daemon(&paths).await;
    let port = running.http_port;
    let token_a = create_token_as(port, &owner_token(&paths), "a", "").await;
    let token_b = create_token_as(port, &token_a.token, "b", "").await;
    let token_c = create_token_as(port, &token_b.token, "c", "").await;
    let token_d = create_token_as(port, &token_a.token, "d", "").await;

    revoke_as_owner(&paths, port, &token_b.token_id).await;

    assert_revoked(port, &token_b).await;
    assert_revoked(port, &token_c).await;
    assert_live(port, &token_a).await;
    assert_live(port, &token_d).await;
    // The listing without revoked tokens shows the survivors only.
    let response = admitted(&execute_as(port, &owner_token(&paths), "token.list", json!({}), "").await);
    let active: Vec<String> = ok_result(&response)["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|summary| summary["token_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(active, [token_a.token_id.clone(), token_d.token_id.clone()]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// A request that passed the gate before its token was revoked still can't make a token after: the writer's
/// transaction refuses it with `RRS-C1006` and nothing is written or cached.
#[tokio::test(flavor = "multi_thread")]
async fn a_token_created_after_its_creator_is_revoked_is_refused() {
    let paths = test_paths("tc-late");
    let running = start_test_daemon(&paths).await;
    let creator = created_token_principal(&running.server, vec![Scope::SystemAdmin]).await;
    let revoke = json!({"token_id": creator.id});
    let request = execute_request("token.revoke", &revoke.to_string());
    ok_result(&run_command(&running.server, &owner_principal(), request).await);

    let late = execute_request("token.create", r#"{"name": "late", "scopes": ["system.admin"]}"#);
    let refused = run_command(&running.server, &creator, late).await;

    assert_eq!(response_codes(&refused), [CODE_TOKEN_REVOKED]);
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    let rows = list_tokens(&reader).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(running.server.auth.scoped_tokens.lock().unwrap().len(), 1);
    drop(reader);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// Creates racing their creator's revoke: each one either commits before the revoke and is swept with it, or is
/// refused. No token made by the revoked creator is left live, in `rastai.db` or in the gate's cache.
#[tokio::test(flavor = "multi_thread")]
async fn a_create_racing_its_creators_revoke_never_leaves_a_live_token() {
    let paths = test_paths("tc-race");
    let running = start_test_daemon(&paths).await;
    let creator = created_token_principal(&running.server, vec![Scope::SystemAdmin]).await;
    // Half the creates are sent before the revoke and half after, all on the runtime's threads at once.
    let mut creates = Vec::new();
    for index in 0..RACING_CREATES / 2 {
        creates.push(spawn_create(&running.server, &creator, index));
    }
    let revoke = spawn_revoke(&running.server, &creator.id);
    for index in RACING_CREATES / 2..RACING_CREATES {
        creates.push(spawn_create(&running.server, &creator, index));
    }
    ok_result(&revoke.await.unwrap());
    for create in creates {
        let response = create.await.unwrap();
        assert!(
            response.ok || response_codes(&response) == [CODE_TOKEN_REVOKED],
            "{response:?}"
        );
    }

    let server = &running.server;
    let reader = open_index_reader(&server.store.index_readers).unwrap();
    let rows = list_tokens(&reader).unwrap();
    drop(reader);
    let created_by_creator = format!("token:{}", creator.id);
    let cached = server.auth.scoped_tokens.lock().unwrap().clone();
    assert_eq!(cached.len(), rows.len(), "the cache holds exactly the rows");
    for row in &rows {
        assert!(row.revoked_unix_ms.is_some(), "{} is live", row.token_id);
        assert_eq!(row.created_by == created_by_creator, row.token_id != creator.id);
        let entry = cached.iter().find(|entry| entry.token_id == row.token_id).unwrap();
        assert!(entry.revoked, "{} is live in the cache", row.token_id);
    }
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

fn spawn_create(server: &Arc<Server>, creator: &Principal, index: usize) -> JoinHandle<ExecuteResponse> {
    let server = Arc::clone(server);
    let creator = creator.clone();
    let arguments = json!({"name": format!("racer-{index}"), "scopes": ["system.admin"]});
    tokio::spawn(async move {
        let request = execute_request("token.create", &arguments.to_string());
        run_command(&server, &creator, request).await
    })
}

fn spawn_revoke(server: &Arc<Server>, token_id: &str) -> JoinHandle<ExecuteResponse> {
    let server = Arc::clone(server);
    let arguments = json!({"token_id": token_id});
    tokio::spawn(async move {
        let request = execute_request("token.revoke", &arguments.to_string());
        run_command(&server, &owner_principal(), request).await
    })
}

// ---- Only the owner acts as a plugin ----

/// A leaked `system.admin` token that names an enabled plugin granted `system.admin` would make a token recorded as
/// the plugin's, out of reach of its own revoke. `execute_command` refuses it first, in a dry run too, and nothing is
/// made.
#[tokio::test(flavor = "multi_thread")]
async fn a_token_that_sets_acting_plugin_id_is_refused_and_makes_no_token() {
    let paths = test_paths("tc-narrow");
    let plugin_folder = write_admin_plugin(&paths);
    let running = start_with_dev_folders(&paths, &[&plugin_folder]).await;
    assert_eq!(plugin_state(&running, ADMIN_PLUGIN_ID).await, "enabled");
    let port = running.http_port;
    let leaked = create_token_as(port, &owner_token(&paths), "leaked", "").await;
    let arguments = json!({"name": "successor", "scopes": ADMIN_SCOPES});
    for dry_run in [false, true] {
        let sent = execute_request_as(
            port,
            &leaked.token,
            "token.create",
            arguments.clone(),
            ADMIN_PLUGIN_ID,
            dry_run,
        );
        let refused = admitted(&sent.await);
        assert_eq!(response_codes(&refused), [CODE_SCOPE_NOT_GRANTED], "dry_run {dry_run}");
        let message = &refused.diagnostics[0].message;
        assert!(message.contains("owner"), "{message}");
        assert!(message.contains(&format!("token:{}", leaked.token_id)), "{message}");
        assert!(refused.journal_id.is_empty());
    }
    // A plugin can't act as another plugin either.
    let plugin = Principal {
        kind: PrincipalKind::Plugin,
        id: "acme_other".to_string(),
        scopes: vec![Scope::SystemAdmin, Scope::Read],
    };
    let mut as_other = execute_request("token.create", &arguments.to_string());
    as_other.acting_plugin_id = ADMIN_PLUGIN_ID.to_string();
    let refused = run_command(&running.server, &plugin, as_other).await;
    assert_eq!(response_codes(&refused), [CODE_SCOPE_NOT_GRANTED]);
    let tokens = listed_tokens(&paths, port).await;
    assert_eq!(tokens.len(), 1, "{tokens:?}");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- What a plugin made ----

/// A dev plugin's tokens, and the tokens they made, die when it is disabled; the plugin then makes no new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_dev_plugins_tokens_die_when_it_is_disabled() {
    let paths = test_paths("tc-dev-off");
    let plugin_folder = write_admin_plugin(&paths);
    let running = start_with_dev_folders(&paths, &[&plugin_folder]).await;
    let port = running.http_port;
    let (plugin_made, under_it) = plugin_made_tokens(&paths, port).await;
    let owner_made = create_token_as(port, &owner_token(&paths), "owner-made", "").await;
    assert_live(port, &plugin_made).await;

    disable_plugin(&running.server, ADMIN_PLUGIN_ID).await;

    assert_eq!(plugin_state(&running, ADMIN_PLUGIN_ID).await, "disabled");
    assert_revoked(port, &plugin_made).await;
    assert_revoked(port, &under_it).await;
    assert_live(port, &owner_made).await;
    assert_disabled_plugin_makes_no_token(&running).await;
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// The plugin's own principal (its module or job) is refused a token once it is disabled, even holding the scopes:
/// the check under the plugin-host lock sees the state.
async fn assert_disabled_plugin_makes_no_token(running: &RunningDaemon) {
    let plugin = Principal {
        kind: PrincipalKind::Plugin,
        id: ADMIN_PLUGIN_ID.to_string(),
        scopes: vec![Scope::SystemAdmin, Scope::Read],
    };
    let request = execute_request("token.create", r#"{"name": "after", "scopes": ["read"]}"#);
    let refused = run_command(&running.server, &plugin, request).await;
    assert_eq!(response_codes(&refused), [CODE_SCOPE_NOT_GRANTED]);
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    let rows = list_tokens(&reader).unwrap();
    assert!(rows.iter().all(|row| row.name != "after"));
}

/// An installed plugin's tokens die when it is disabled, and its row says `disabled`: both are one write.
#[tokio::test(flavor = "multi_thread")]
async fn an_installed_plugins_tokens_die_when_it_is_disabled() {
    let paths = test_paths("tc-inst-off");
    let first = start_test_daemon(&paths).await;
    let row = PluginRow {
        plugin_id: ADMIN_PLUGIN_ID.to_string(),
        version: "1.0.0".to_string(),
        state: plugin_state_name(PluginState::Installed).to_string(),
        content_hash: "blake3:00".to_string(),
        granted_permissions: vec![ADMIN_PLUGIN_PERMISSION.to_string()],
        source: "installed".to_string(),
    };
    write_index(&first.server.store.index, IndexWrite::UpsertPlugin(row))
        .await
        .unwrap();
    stop_test_daemon(first).await;
    write_admin_manifest(&paths.data.join("plugins").join(ADMIN_PLUGIN_ID).join("1.0.0"));
    let running = start_test_daemon(&paths).await;
    assert_eq!(plugin_state(&running, ADMIN_PLUGIN_ID).await, "enabled");
    let port = running.http_port;
    let (plugin_made, under_it) = plugin_made_tokens(&paths, port).await;

    disable_plugin(&running.server, ADMIN_PLUGIN_ID).await;

    assert_revoked(port, &plugin_made).await;
    assert_revoked(port, &under_it).await;
    let saved = {
        let reader = open_index_reader(&running.server.store.index_readers).unwrap();
        get_plugin(&reader, ADMIN_PLUGIN_ID).unwrap().unwrap()
    };
    assert_eq!(saved.state, "disabled");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// Dropping a dev path (owner-only, read at start) removes its plugin at the next start, and the start sweep revokes
/// what it made before the listener opens. A restart that still has the plugin keeps them.
#[tokio::test(flavor = "multi_thread")]
async fn starting_without_a_dev_plugin_revokes_the_tokens_it_made() {
    let paths = test_paths("tc-dev-gone");
    let plugin_folder = write_admin_plugin(&paths);
    let running = start_with_dev_folders(&paths, &[&plugin_folder]).await;
    let (plugin_made, under_it) = plugin_made_tokens(&paths, running.http_port).await;
    let owner_made = create_token_as(running.http_port, &owner_token(&paths), "owner-made", "").await;
    stop_test_daemon(running).await;

    let still_there = start_with_dev_folders(&paths, &[&plugin_folder]).await;
    assert_live(still_there.http_port, &plugin_made).await;
    stop_test_daemon(still_there).await;

    let without_it = start_test_daemon(&paths).await;
    let port = without_it.http_port;
    assert_revoked(port, &plugin_made).await;
    assert_revoked(port, &under_it).await;
    assert_live(port, &owner_made).await;
    stop_test_daemon(without_it).await;
    remove_test_folder(&paths);
}
