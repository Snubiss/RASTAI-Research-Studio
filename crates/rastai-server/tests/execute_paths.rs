//! The `Execute` path (flows/platform.md 3.1 and 3.2): one test per branch of the decision chart with its code, then
//! the journal (secret redaction, idempotent replay, `RRS-C0004`) and the handlers built so far (`settings.*`,
//! `token.*`, `system.*`). Each test starts a daemon on its own temporary folders and runs commands in-process
//! through `execute_command`, where every caller ends; `connect_protocol.rs` runs the same path over HTTP.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use rastai_core::commands::registry::add_plugin_commands;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::paths::{rastai_db_path, user_settings_path};
use rastai_core::principal::owner_principal;
use rastai_core::scopes::Scope;
use rastai_core::time::now_unix_ms;
use rastai_formats::hashing::hash_bytes;
use rastai_proto::commands::v1::{CommandDescriptor, ExecuteRequest, ExecuteResponse};
use rastai_server::daemon::RunningDaemon;
use rastai_server::http::sessions::find_scoped_token;
use rastai_store::index::journal::{JournalRow, JournalStartedRow, get_journal};
use rastai_store::index::kept_tables::ProjectRow;
use rastai_store::index::reads::{list_table_columns, list_table_rows, open_index_reader};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use serde_json::{Value, json};

use common::{
    execute_request, ok_result, remove_test_folder, response_codes, run_command, start_test_daemon, stop_test_daemon,
    test_paths, token_principal,
};

const REDACTED: &str = "<redacted>";

/// Runs one command as the owner.
async fn run_as_owner(running: &RunningDaemon, command_id: &str, arguments: Value) -> ExecuteResponse {
    let request = execute_request(command_id, &arguments.to_string());
    run_command(&running.server, &owner_principal(), request).await
}

/// Runs one command as the owner with an idempotency key.
async fn run_with_key(running: &RunningDaemon, command_id: &str, arguments: Value, key: &str) -> ExecuteResponse {
    let request = ExecuteRequest {
        idempotency_key: key.to_string(),
        ..execute_request(command_id, &arguments.to_string())
    };
    run_command(&running.server, &owner_principal(), request).await
}

fn journal_row(running: &RunningDaemon, journal_id: &str) -> JournalRow {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_journal(&reader, journal_id).unwrap().unwrap()
}

/// Every row of every table of rastai.db, as text.
fn every_stored_value(running: &RunningDaemon) -> String {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    let mut text = String::new();
    for table in list_table_columns(&reader).unwrap() {
        for row in list_table_rows(&reader, &table.table_name).unwrap() {
            text.push_str(&row.join("\t"));
            text.push('\n');
        }
    }
    text
}

/// The bytes of rastai.db and its write-ahead log, where an unredacted value would sit before a checkpoint.
fn database_file_bytes(paths: &rastai_core::paths::DataPaths) -> Vec<u8> {
    let database = rastai_db_path(paths);
    let mut bytes = std::fs::read(&database).unwrap();
    let wal = database.with_extension("db-wal");
    bytes.extend(std::fs::read(wal).unwrap_or_default());
    bytes
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

// ---- The decision chart, one branch each ----

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_command_is_c0001() {
    let paths = test_paths("x-unknown");
    let running = start_test_daemon(&paths).await;
    let response = run_as_owner(&running, "system.make_coffee", json!({})).await;
    assert!(!response.ok);
    assert_eq!(response_codes(&response), ["RRS-C0001"]);
    assert!(response.journal_id.is_empty());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_scope_is_c1005_before_the_arguments_are_read() {
    let paths = test_paths("x-scope");
    let running = start_test_daemon(&paths).await;
    let chat_only = token_principal(vec![Scope::Chat]);
    // Bad arguments too: the caller without the scope must not learn their shape.
    let request = execute_request("token.create", r#"{"colour": 1}"#);
    let response = run_command(&running.server, &chat_only, request).await;
    assert_eq!(response_codes(&response), ["RRS-C1005"]);
    assert!(response.diagnostics[0].message.contains("system.admin"));
    // The same token may run what its scopes allow.
    let read_only = token_principal(vec![Scope::Read]);
    let status = run_command(&running.server, &read_only, execute_request("system.status", "")).await;
    assert!(status.ok, "{:?}", status.diagnostics);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acting_plugin_narrows_even_the_owner() {
    let paths = test_paths("x-acting");
    let running = start_test_daemon(&paths).await;
    let request = ExecuteRequest {
        acting_plugin_id: "acme".to_string(),
        ..execute_request("system.status", "")
    };
    let response = run_command(&running.server, &owner_principal(), request).await;
    assert_eq!(response_codes(&response), ["RRS-C1005"]);
    assert!(response.diagnostics[0].message.contains("plugin:acme"));
    let malformed = ExecuteRequest {
        acting_plugin_id: "Acme Tools".to_string(),
        ..execute_request("system.status", "")
    };
    let response = run_command(&running.server, &owner_principal(), malformed).await;
    assert_eq!(response_codes(&response), ["RRS-C0002"]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_of_a_plugin_that_is_not_enabled_is_c0003() {
    let paths = test_paths("x-plugin");
    let running = start_test_daemon(&paths).await;
    let command = CommandDescriptor {
        command_id: "acme.tool.run".to_string(),
        scope: "read".to_string(),
        ..CommandDescriptor::default()
    };
    add_plugin_commands(&mut running.server.registry.write().unwrap(), "acme", vec![command]).unwrap();
    let response = run_as_owner(&running, "acme.tool.run", json!({})).await;
    assert_eq!(response_codes(&response), ["RRS-C0003"]);
    assert_eq!(response.diagnostics[0].fix_command_id, "plugin.enable");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_arguments_are_c0002_with_every_problem_and_its_path() {
    let paths = test_paths("x-args");
    let running = start_test_daemon(&paths).await;
    let response = run_as_owner(&running, "token.create", json!({"name": 5, "colour": "red"})).await;
    let found: Vec<(&str, &str)> = response
        .diagnostics
        .iter()
        .map(|item| (item.code.as_str(), item.path.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("RRS-C0002", "#/name"),
            ("RRS-E1002", "#/name"),
            ("RRS-E1003", "#/colour")
        ]
    );
    // Refused before the journal: nothing was run, so nothing was written.
    assert!(response.journal_id.is_empty());
    let not_json = run_command(&running.server, &owner_principal(), execute_request("token.list", "{")).await;
    assert_eq!(response_codes(&not_json)[0], "RRS-C0002");
    assert!(response_codes(&not_json)[1].starts_with("RRS-E0"));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_checks_and_writes_nothing() {
    let paths = test_paths("x-dry");
    let running = start_test_daemon(&paths).await;
    let dry = |command_id: &str, arguments: Value| ExecuteRequest {
        dry_run: true,
        ..execute_request(command_id, &arguments.to_string())
    };
    let token = run_command(
        &running.server,
        &owner_principal(),
        dry("token.create", json!({"name": "ci", "preset": "chat-only"})),
    )
    .await;
    assert!(token.journal_id.is_empty());
    // The answer has the result's shape: the scopes it would have, and no id or token, since none was made.
    let planned = ok_result(&token);
    assert_eq!(planned["scopes"], json!(["chat"]));
    assert_eq!(
        (planned["token_id"].as_str(), planned["token"].as_str()),
        (Some(""), Some(""))
    );
    let code = run_command(
        &running.server,
        &owner_principal(),
        dry("token.login_code.create", json!({})),
    )
    .await;
    let planned_code = ok_result(&code);
    assert_eq!(planned_code["code"], "");
    assert_eq!(planned_code["http_port"], u32::from(running.http_port));
    let setting = run_command(
        &running.server,
        &owner_principal(),
        dry("settings.set", json!({"key": "ui.theme", "value": "dark"})),
    )
    .await;
    assert_eq!(ok_result(&setting)["value"], "dark");
    assert!(!user_settings_path(&paths).exists());
    let stop = run_command(&running.server, &owner_principal(), dry("system.shutdown", json!({}))).await;
    assert!(stop.ok);
    assert!(!*running.server.shutdown.borrow());
    // A dry run still checks: a bad scope name is refused.
    let refused = run_command(
        &running.server,
        &owner_principal(),
        dry("token.create", json!({"name": "ci", "scopes": ["root"]})),
    )
    .await;
    assert_eq!(response_codes(&refused), ["RRS-C0002", "RRS-E1004"]);
    let listed = ok_result(&run_as_owner(&running, "token.list", json!({})).await);
    assert_eq!(listed["tokens"], json!([]));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- The journal ----

#[tokio::test(flavor = "multi_thread")]
async fn read_commands_are_not_journaled_and_the_others_are() {
    let paths = test_paths("x-journal");
    let running = start_test_daemon(&paths).await;
    let status = run_as_owner(&running, "system.status", json!({})).await;
    assert!(status.ok);
    assert!(status.journal_id.is_empty());
    let started = run_as_owner(
        &running,
        "train.start",
        json!({"recipe": "sft", "design_path": "d", "dataset_path": "x"}),
    )
    .await;
    // A command not built yet fails with its area's code and is journaled like any other.
    assert_eq!(response_codes(&started), ["RRS-T0006"]);
    let row = journal_row(&running, &started.journal_id);
    assert_eq!(
        (row.ok, row.code.as_str(), row.principal.as_str()),
        (Some(false), "RRS-T0006", "owner")
    );
    assert!(row.finished_unix_ms.is_some());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_token_never_reaches_rastai_db() {
    let paths = test_paths("x-redact");
    let running = start_test_daemon(&paths).await;
    let response = run_as_owner(&running, "token.create", json!({"name": "ci", "preset": "mcp-default"})).await;
    let result = ok_result(&response);
    let token = result["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    assert_eq!(result["scopes"], json!(["read", "chat", "project"]));

    let row = journal_row(&running, &response.journal_id);
    let stored: Value = serde_json::from_str(&row.result_json).unwrap();
    assert_eq!(stored["token"], REDACTED);
    assert_eq!(stored["token_id"], result["token_id"]);
    assert_eq!(row.ok, Some(true));
    assert!(!every_stored_value(&running).contains(&token));
    assert!(!contains_bytes(&database_file_bytes(&paths), token.as_bytes()));
    // The token works: the gate's cache knows its hash and its scopes.
    let cached = find_scoped_token(&running.server.auth, &hash_bytes(token.as_bytes())).unwrap();
    assert_eq!(cached.scopes, vec![Scope::Read, Scope::Chat, Scope::Project]);

    let code = run_as_owner(&running, "token.login_code.create", json!({})).await;
    let code_result = ok_result(&code);
    let code_text = code_result["code"].as_str().unwrap().to_string();
    assert_eq!(code_result["http_port"], u32::from(running.http_port));
    assert_eq!(
        serde_json::from_str::<Value>(&journal_row(&running, &code.journal_id).result_json).unwrap()["code"],
        REDACTED
    );
    assert!(!every_stored_value(&running).contains(&code_text));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_key_replays_the_stored_redacted_response() {
    let paths = test_paths("x-replay");
    let running = start_test_daemon(&paths).await;
    let arguments = json!({"name": "ci", "preset": "chat-only"});
    let first = run_with_key(&running, "token.create", arguments.clone(), "key-1").await;
    let replay = run_with_key(&running, "token.create", arguments, "key-1").await;
    let (first_result, replay_result) = (ok_result(&first), ok_result(&replay));
    assert_eq!(replay.journal_id, first.journal_id);
    assert_eq!(replay_result["token_id"], first_result["token_id"]);
    assert_ne!(first_result["token"], REDACTED);
    assert_eq!(replay_result["token"], REDACTED);
    let listed = ok_result(&run_as_owner(&running, "token.list", json!({})).await);
    assert_eq!(listed["tokens"].as_array().unwrap().len(), 1);
    // Another principal's same key is its own. It holds chat, so it may hand chat on.
    let admin_token = token_principal(vec![Scope::SystemAdmin, Scope::Chat]);
    let request = ExecuteRequest {
        idempotency_key: "key-1".to_string(),
        ..execute_request("token.create", r#"{"name": "ci", "preset": "chat-only"}"#)
    };
    let other = run_command(&running.server, &admin_token, request).await;
    assert_ne!(ok_result(&other)["token_id"], first_result["token_id"]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_closes_an_unfinished_row_as_c0005_and_its_key_runs_again() {
    let paths = test_paths("x-interrupted");
    let running = start_test_daemon(&paths).await;
    // The started row of a token.create the last daemon was running when it stopped: no finished row follows.
    let arguments = json!({"name": "ci", "preset": "chat-only"});
    let interrupted_id = new_id(IdPrefix::Journal).unwrap();
    let started = JournalStartedRow {
        journal_id: interrupted_id.clone(),
        command_id: "token.create".to_string(),
        principal: "owner".to_string(),
        arguments_json: arguments.to_string(),
        idempotency_key: "key-i".to_string(),
        started_unix_ms: now_unix_ms(),
    };
    write_index(&running.server.store.index, IndexWrite::JournalStarted(started))
        .await
        .unwrap();
    assert_eq!(journal_row(&running, &interrupted_id).ok, None);
    stop_test_daemon(running).await;

    let restarted = start_test_daemon(&paths).await;
    let row = journal_row(&restarted, &interrupted_id);
    assert_eq!((row.ok, row.code.as_str()), (Some(false), "RRS-C0005"));
    assert!(row.finished_unix_ms.is_some());
    // The interrupted attempt kept nothing, so a retry under its key runs the command again.
    let retry = run_with_key(&restarted, "token.create", arguments, "key-i").await;
    let retried = ok_result(&retry);
    assert_ne!(retry.journal_id, interrupted_id);
    assert_ne!(retried["token"], REDACTED);
    let listed = ok_result(&run_as_owner(&restarted, "token.list", json!({})).await);
    assert_eq!(listed["tokens"].as_array().unwrap().len(), 1);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_key_with_another_request_is_c0004() {
    let paths = test_paths("x-reuse");
    let running = start_test_daemon(&paths).await;
    let first = run_with_key(
        &running,
        "token.create",
        json!({"name": "ci", "preset": "chat-only"}),
        "key-1",
    )
    .await;
    assert!(first.ok);
    let other_arguments = run_with_key(
        &running,
        "token.create",
        json!({"name": "other", "preset": "chat-only"}),
        "key-1",
    )
    .await;
    assert_eq!(response_codes(&other_arguments), ["RRS-C0004"]);
    let other_command = run_with_key(
        &running,
        "settings.set",
        json!({"key": "ui.theme", "value": "dark"}),
        "key-1",
    )
    .await;
    assert_eq!(response_codes(&other_command), ["RRS-C0004"]);
    // Key order and spacing don't make a request different.
    let reordered = run_with_key(
        &running,
        "token.create",
        json!({"preset": "chat-only", "name": "ci"}),
        "key-1",
    )
    .await;
    assert_eq!(reordered.journal_id, first.journal_id);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_attempt_is_run_again_under_its_key() {
    let paths = test_paths("x-retry");
    let running = start_test_daemon(&paths).await;
    let arguments = json!({"token_id": "tok_01JA2B3C4D5E6F7G8H9J0K1M2N"});
    let first = run_with_key(&running, "token.revoke", arguments.clone(), "key-r").await;
    let second = run_with_key(&running, "token.revoke", arguments, "key-r").await;
    assert_eq!(response_codes(&first), ["RRS-C0002"]);
    assert_eq!(response_codes(&second), ["RRS-C0002"]);
    assert_ne!(first.journal_id, second.journal_id);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_sent_while_the_first_runs_waits_for_it() {
    let paths = test_paths("x-dup");
    let running = start_test_daemon(&paths).await;
    let arguments = json!({"name": "ci", "preset": "chat-only"});
    let (one, two) = tokio::join!(
        run_with_key(&running, "token.create", arguments.clone(), "key-d"),
        run_with_key(&running, "token.create", arguments, "key-d"),
    );
    let (one, two) = (ok_result(&one), ok_result(&two));
    assert_eq!(one["token_id"], two["token_id"]);
    let texts = [one["token"].as_str().unwrap(), two["token"].as_str().unwrap()];
    assert_eq!(texts.iter().filter(|text| **text == REDACTED).count(), 1, "{texts:?}");
    let listed = ok_result(&run_as_owner(&running, "token.list", json!({})).await);
    assert_eq!(listed["tokens"].as_array().unwrap().len(), 1);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- Handlers ----

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_token_is_refused_and_listed_only_when_asked() {
    let paths = test_paths("x-revoke");
    let running = start_test_daemon(&paths).await;
    let created = ok_result(&run_as_owner(&running, "token.create", json!({"name": "ci", "scopes": ["read"]})).await);
    let token_hash = hash_bytes(created["token"].as_str().unwrap().as_bytes());
    let revoked = ok_result(&run_as_owner(&running, "token.revoke", json!({"token_id": created["token_id"]})).await);
    assert!(revoked["revoked_unix_ms"].as_u64().unwrap() > 0);
    assert!(find_scoped_token(&running.server.auth, &token_hash).unwrap().revoked);
    let again = ok_result(&run_as_owner(&running, "token.revoke", json!({"token_id": created["token_id"]})).await);
    assert_eq!(again["revoked_unix_ms"], revoked["revoked_unix_ms"]);
    let active = ok_result(&run_as_owner(&running, "token.list", json!({})).await);
    assert_eq!(active["tokens"], json!([]));
    let all = ok_result(&run_as_owner(&running, "token.list", json!({"include_revoked": true})).await);
    assert_eq!(all["tokens"][0]["revoked_unix_ms"], revoked["revoked_unix_ms"]);
    assert!(all["tokens"][0].get("token").is_none());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_hands_out_only_the_scopes_it_holds() {
    let paths = test_paths("x-escalate");
    let running = start_test_daemon(&paths).await;
    let admin_only = token_principal(vec![Scope::SystemAdmin]);
    let wider = execute_request("token.create", r#"{"name": "wider", "scopes": ["chat", "secrets"]}"#);
    let refused = run_command(&running.server, &admin_only, wider.clone()).await;
    assert_eq!(response_codes(&refused), ["RRS-C1005", "RRS-C1005"]);
    assert!(refused.diagnostics[0].message.contains("chat"));
    assert!(refused.diagnostics[1].message.contains("secrets"));
    // A dry run is refused the same way.
    let dry = ExecuteRequest { dry_run: true, ..wider };
    assert_eq!(
        response_codes(&run_command(&running.server, &admin_only, dry).await)[0],
        "RRS-C1005"
    );
    let listed = ok_result(&run_as_owner(&running, "token.list", json!({})).await);
    assert_eq!(listed["tokens"], json!([]));
    // What it holds it may pass on; the owner may grant anything.
    let same = execute_request("token.create", r#"{"name": "same", "scopes": ["system.admin"]}"#);
    assert!(run_command(&running.server, &admin_only, same).await.ok);
    let owner_made = run_as_owner(
        &running,
        "token.create",
        json!({"name": "wide", "scopes": ["chat", "secrets"]}),
    )
    .await;
    assert_eq!(ok_result(&owner_made)["scopes"], json!(["chat", "secrets"]));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_still_work_after_a_restart() {
    let paths = test_paths("x-restart");
    let running = start_test_daemon(&paths).await;
    let created =
        ok_result(&run_as_owner(&running, "token.create", json!({"name": "ci", "preset": "chat-only"})).await);
    stop_test_daemon(running).await;
    let restarted = start_test_daemon(&paths).await;
    let token_hash = hash_bytes(created["token"].as_str().unwrap().as_bytes());
    let cached = find_scoped_token(&restarted.server.auth, &token_hash).unwrap();
    assert_eq!(cached.token_id, created["token_id"].as_str().unwrap());
    assert_eq!(cached.scopes, vec![Scope::Chat]);
    assert!(!cached.revoked);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_set_writes_the_user_file_and_settings_get_reads_it() {
    let paths = test_paths("x-settings");
    let running = start_test_daemon(&paths).await;
    let set = ok_result(&run_as_owner(&running, "settings.set", json!({"key": "ui.theme", "value": "dark"})).await);
    assert_eq!(set, json!({"key": "ui.theme", "value": "dark", "source": "user"}));
    let file: Value = serde_json::from_slice(&std::fs::read(user_settings_path(&paths)).unwrap()).unwrap();
    assert_eq!(file["type"], "rastai.settings");
    assert_eq!(file["ui"], json!({"theme": "dark"}));
    let got = ok_result(&run_as_owner(&running, "settings.get", json!({"key": "ui.theme"})).await);
    assert_eq!(got["values"], json!({"ui": {"theme": "dark"}}));
    assert_eq!(
        got["sources"],
        json!([{"key": "ui.theme", "source": "user", "project_folder": ""}])
    );
    // A second key keeps the first, and a restart reads the file.
    run_as_owner(
        &running,
        "settings.set",
        json!({"key": "ui.density", "value": "compact"}),
    )
    .await;
    stop_test_daemon(running).await;
    let restarted = start_test_daemon(&paths).await;
    let ui = ok_result(&run_as_owner(&restarted, "settings.get", json!({"key": "ui"})).await);
    assert_eq!(
        (
            ui["values"]["ui"]["theme"].as_str(),
            ui["values"]["ui"]["density"].as_str()
        ),
        (Some("dark"), Some("compact"))
    );
    let unknown = run_as_owner(&restarted, "settings.get", json!({"key": "ui.colour"})).await;
    assert_eq!(response_codes(&unknown), ["RRS-C0002"]);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_set_refuses_what_the_file_cannot_hold_and_never_overwrites_a_broken_file() {
    let paths = test_paths("x-badset");
    let running = start_test_daemon(&paths).await;
    let unknown = run_as_owner(&running, "settings.set", json!({"key": "ui.colour", "value": "red"})).await;
    assert_eq!(response_codes(&unknown)[0], "RRS-C0002");
    assert_eq!(unknown.diagnostics[0].path, "#/key");
    assert!(response_codes(&unknown).contains(&"RRS-E1003".to_string()));
    let wrong_kind = run_as_owner(&running, "settings.set", json!({"key": "daemon.port", "value": "high"})).await;
    assert_eq!(wrong_kind.diagnostics[0].path, "#/value");
    assert!(!user_settings_path(&paths).exists());

    std::fs::create_dir_all(&paths.config).unwrap();
    std::fs::write(user_settings_path(&paths), b"{ not json").unwrap();
    let broken = run_as_owner(&running, "settings.set", json!({"key": "ui.theme", "value": "dark"})).await;
    assert_eq!(response_codes(&broken), ["RRS-C0009"]);
    assert_eq!(std::fs::read(user_settings_path(&paths)).unwrap(), b"{ not json");
    let without_project = run_as_owner(
        &running,
        "settings.set",
        json!({"key": "ui.theme", "value": "dark", "level": "project"}),
    )
    .await;
    assert_eq!(response_codes(&without_project), ["RRS-C0002"]);
    assert_eq!(without_project.diagnostics[0].path, "#/level");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_level_setting_goes_to_the_project_file() {
    let paths = test_paths("x-project");
    let running = start_test_daemon(&paths).await;
    let project_folder = paths.data.join("demo.rrsproj");
    let project = ProjectRow {
        project_id: "prj_01JA2B3C4D5E6F7G8H9J0K1M2N".to_string(),
        path: project_folder.display().to_string(),
        name: "demo".to_string(),
        opened_unix_ms: 1,
    };
    write_index(&running.server.store.index, IndexWrite::UpsertProject(project.clone()))
        .await
        .unwrap();
    let in_project = |command_id: &str, arguments: Value| ExecuteRequest {
        project_id: project.project_id.clone(),
        ..execute_request(command_id, &arguments.to_string())
    };
    let set_request = in_project(
        "settings.set",
        json!({"key": "privacy.data_policy", "value": "any", "level": "project"}),
    );
    let set = run_command(&running.server, &owner_principal(), set_request).await;
    assert_eq!(ok_result(&set)["source"], "project");
    assert!(project_folder.join("settings.json").exists());
    assert!(!user_settings_path(&paths).exists());
    let got = run_command(
        &running.server,
        &owner_principal(),
        in_project("settings.get", json!({"key": "privacy.data_policy"})),
    )
    .await;
    let got = ok_result(&got);
    assert_eq!(got["values"]["privacy"]["data_policy"], "any");
    assert_eq!(got["sources"][0]["source"], "project");
    assert_eq!(got["sources"][0]["project_folder"], project.path.as_str());
    // Outside the project the user's value, here the default, is in effect.
    let outside = ok_result(&run_as_owner(&running, "settings.get", json!({"key": "privacy.data_policy"})).await);
    assert_eq!(outside["values"]["privacy"]["data_policy"], "local-only");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn system_status_reports_versions_and_system_shutdown_stops_the_daemon() {
    let paths = test_paths("x-system");
    let running = start_test_daemon(&paths).await;
    let status = ok_result(&run_as_owner(&running, "system.status", json!({})).await);
    assert_eq!(status["app_version"], rastai_core::versions::APP_VERSION);
    assert_eq!(
        status["api_packages"],
        json!(["rastai.commands.v1", "rastai.events.v1"])
    );
    let listed = ok_result(&run_as_owner(&running, "command.list", json!({"section": "settings"})).await);
    let ids: Vec<&str> = listed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["command_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"settings.set") && !ids.contains(&"job.list"), "{ids:?}");
    let stopped = run_as_owner(&running, "system.shutdown", json!({})).await;
    assert_eq!(ok_result(&stopped), json!({"running_job_ids": []}));
    assert!(*running.server.shutdown.borrow());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
