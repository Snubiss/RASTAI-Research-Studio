//! `POST /mcp` (layout: rust-control-plane.md 7.3, 8.6; flows/platform.md 4.3): the tool list is the token's
//! scopes, a tool call runs through `execute_command` as the token and lands in the journal under it, and the MCP
//! sessions behave as the `rastai mcp` bridge expects (a new one per `initialize`, 404 for one the daemon doesn't
//! hold, 202 for notifications).
//!
//! Each test starts a daemon on its own temporary folders. The token tests drive the real bridge (`run_bridge`)
//! over daemon.sock, the way `rastai mcp` reaches the daemon; the session tests send raw requests, over daemon.sock
//! and over the loopback port.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use rastai_core::paths::{DataPaths, daemon_socket_path};
use rastai_core::principal::owner_principal;
use rastai_mcp::bridge::{DaemonEndpoint, run_bridge};
use rastai_server::daemon::RunningDaemon;
use rastai_store::index::journal::{JournalRow, get_journal};
use rastai_store::index::reads::{list_table_rows, open_index_reader};
use rastai_transport::http_client::HttpResponse;
use serde_json::{Value, json};

use common::{
    execute_request, header_value, ok_result, owner_bearer, remove_test_folder, run_command, send_to_port,
    send_to_socket, start_test_daemon, stop_test_daemon, test_paths,
};

const MCP_PATH: &str = "/mcp";
const PROTOCOL_VERSION: &str = "2025-06-18";
const SESSION_ID_HEADER: &str = "mcp-session-id";
/// The scopes of the `mcp-default` preset (contracts 6.3), as the registry spells them.
const MCP_DEFAULT_SCOPES: &[&str] = &["read", "chat", "project"];
/// The first column of a `command_journal` row (migration 0001).
const JOURNAL_ID_COLUMN: usize = 0;
/// `list_table_rows` writes text values in double quotes (journal ids hold no quote to escape).
const TEXT_QUOTE: char = '"';

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

/// A scoped token as `token.create` hands it out.
struct NewToken {
    token_id: String,
    token: String,
}

// ---- The tool list follows the token's scopes ----

#[tokio::test(flavor = "multi_thread")]
async fn the_tool_list_is_the_commands_the_token_scopes_allow() {
    let paths = test_paths("mcp-list");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;

    let replies = run_bridge_on_lines(&paths, &token.token, &[INITIALIZE, INITIALIZED, TOOLS_LIST]).await;
    let listed = tool_names(&replies[1]);
    assert_eq!(listed, commands_with_scopes(&running, MCP_DEFAULT_SCOPES));
    assert!(listed.contains(&"system__status".to_string()), "{listed:?}");
    for unlisted in ["settings__set", "token__create", "train__start", "system__shutdown"] {
        assert!(
            !listed.contains(&unlisted.to_string()),
            "{unlisted} is outside mcp-default: {listed:?}"
        );
    }

    // The owner token, sent as a bearer token, sees every command: the list isn't a fixed one.
    let owner_token = owner_token_text(&paths);
    let owner_replies = run_bridge_on_lines(&paths, &owner_token, &[INITIALIZE, TOOLS_LIST]).await;
    let owner_listed = tool_names(&owner_replies[1]);
    assert!(owner_listed.contains(&"token__create".to_string()), "{owner_listed:?}");
    assert!(owner_listed.contains(&"train__start".to_string()), "{owner_listed:?}");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- A call runs as the token, through execute_command ----

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_call_lands_in_the_journal_with_the_token_principal() {
    let paths = test_paths("mcp-call");
    let running = start_test_daemon(&paths).await;
    let token = create_token(
        &running,
        json!({"name": "agent", "preset": "mcp-default", "scopes": ["system.admin"]}),
    )
    .await;
    let call = tools_call(3, "settings__set", json!({"key": "ui.theme", "value": "dark"}));

    let replies = run_bridge_on_lines(&paths, &token.token, &[INITIALIZE, INITIALIZED, &call]).await;
    let result = &replies[1]["result"];
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["key"], "ui.theme", "{result}");

    // The token.create the test ran as the owner has a row too; the tool call's row is the token's.
    let rows = journal_rows(&running);
    let row = rows.iter().find(|row| row.command_id == "settings.set").unwrap();
    assert_eq!(row.principal, format!("token:{}", token.token_id));
    assert_eq!(row.ok, Some(true));
    assert_eq!(row.arguments_json, r#"{"key":"ui.theme","value":"dark"}"#);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_outside_the_scopes_is_c1005_and_an_unknown_tool_c0001() {
    let paths = test_paths("mcp-refused");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;
    let journal_before = journal_rows(&running).len();
    let outside = tools_call(3, "settings__set", json!({"key": "ui.theme", "value": "dark"}));
    let unknown = tools_call(4, "system__make_coffee", json!({}));

    let replies = run_bridge_on_lines(&paths, &token.token, &[INITIALIZE, &outside, &unknown]).await;
    assert_eq!(tool_error_code(&replies[1]), "RRS-C1005");
    assert_eq!(tool_error_code(&replies[2]), "RRS-C0001");
    // Neither ran, so neither has a journal row.
    assert_eq!(journal_rows(&running).len(), journal_before);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_experimental_command_is_unknown_to_mcp_while_the_setting_is_off() {
    let paths = test_paths("mcp-experimental");
    let running = start_test_daemon(&paths).await;
    mark_experimental(&running, "job.list");
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;
    let call = tools_call(3, "job__list", json!({}));

    let replies = run_bridge_on_lines(&paths, &token.token, &[INITIALIZE, TOOLS_LIST, &call]).await;
    assert!(!tool_names(&replies[1]).contains(&"job__list".to_string()));
    assert_eq!(tool_error_code(&replies[2]), "RRS-C0001");

    // With the setting on it is listed, and the call reaches its handler, which lists no job yet.
    let set = json!({"key": "plugins.experimental_apis", "value": true});
    ok_result(
        &run_command(
            &running.server,
            &owner_principal(),
            execute_request("settings.set", &set.to_string()),
        )
        .await,
    );
    let replies = run_bridge_on_lines(&paths, &token.token, &[INITIALIZE, TOOLS_LIST, &call]).await;
    assert!(tool_names(&replies[1]).contains(&"job__list".to_string()));
    let result = &replies[2]["result"];
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["jobs"], json!([]), "{result}");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- Sessions and the HTTP answers ----

#[tokio::test(flavor = "multi_thread")]
async fn initialize_opens_a_session_that_only_its_token_can_use() {
    let paths = test_paths("mcp-session");
    let running = start_test_daemon(&paths).await;
    let first = create_token(&running, json!({"name": "first", "preset": "mcp-default"})).await;
    let second = create_token(&running, json!({"name": "second", "preset": "mcp-default"})).await;

    let opened = post_mcp(&paths, Some(&first.token), None, INITIALIZE).await;
    assert_eq!(opened.status, 200);
    assert_eq!(header_value(&opened, "content-type"), Some("application/json"));
    assert_eq!(body_json(&opened)["result"]["protocolVersion"], PROTOCOL_VERSION);
    let session_id = header_value(&opened, SESSION_ID_HEADER).unwrap().to_string();

    let initialized = post_mcp(&paths, Some(&first.token), Some(&session_id), INITIALIZED).await;
    assert_eq!(initialized.status, 202);
    assert!(initialized.body.is_empty());
    let listed = post_mcp(&paths, Some(&first.token), Some(&session_id), TOOLS_LIST).await;
    assert_eq!(listed.status, 200);
    assert_eq!(body_json(&listed)["id"], 2);
    assert!(header_value(&listed, SESSION_ID_HEADER).is_none());

    // Another token can't borrow the session, even knowing its id.
    let borrowed = post_mcp(&paths, Some(&second.token), Some(&session_id), TOOLS_LIST).await;
    assert_eq!(borrowed.status, 404);
    assert_eq!(rpc_error_code(&borrowed), "RRS-C3001");
    let unknown = post_mcp(&paths, Some(&first.token), Some("NOT-A-SESSION"), TOOLS_LIST).await;
    assert_eq!(unknown.status, 404);
    let without_session = post_mcp(&paths, Some(&first.token), None, TOOLS_LIST).await;
    assert_eq!(without_session.status, 400);
    assert_eq!(body_json(&without_session)["id"], 2);
    assert_eq!(rpc_error_code(&without_session), "RRS-C3001");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_ends_with_the_daemon() {
    let paths = test_paths("mcp-restart");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;
    let opened = post_mcp(&paths, Some(&token.token), None, INITIALIZE).await;
    let session_id = header_value(&opened, SESSION_ID_HEADER).unwrap().to_string();
    stop_test_daemon(running).await;

    // The token is in rastai.db and still works; the session lived in memory only, so the bridge gets its 404.
    let restarted = start_test_daemon(&paths).await;
    let lost = post_mcp(&paths, Some(&token.token), Some(&session_id), TOOLS_LIST).await;
    assert_eq!(lost.status, 404);
    let reopened = post_mcp(&paths, Some(&token.token), None, INITIALIZE).await;
    assert_eq!(reopened.status, 200);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_needs_a_bearer_token_even_on_daemon_sock() {
    let paths = test_paths("mcp-bearer");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;

    // daemon.sock admits its user as the owner, but MCP never falls back to the owner.
    let without_token = post_mcp(&paths, None, None, INITIALIZE).await;
    assert_eq!(without_token.status, 401);
    assert_eq!(body_json(&without_token)["code"], "RRS-C1002");
    let wrong_token = post_mcp(&paths, Some("not-a-token"), None, INITIALIZE).await;
    assert_eq!(body_json(&wrong_token)["code"], "RRS-C1002");

    let revoke = json!({"token_id": token.token_id});
    ok_result(
        &run_command(
            &running.server,
            &owner_principal(),
            execute_request("token.revoke", &revoke.to_string()),
        )
        .await,
    );
    let revoked = post_mcp(&paths, Some(&token.token), None, INITIALIZE).await;
    assert_eq!(revoked.status, 401);
    assert_eq!(body_json(&revoked)["code"], "RRS-C1006");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_loopback_port_serves_mcp_behind_the_gate() {
    let paths = test_paths("mcp-port");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;
    let port = running.http_port;
    let bearer = format!("Bearer {}", token.token);
    let host = format!("127.0.0.1:{port}");
    let json_headers = [("Authorization", bearer.as_str()), ("Content-Type", "application/json")];

    let opened = send_to_port(port, "POST", MCP_PATH, &json_headers, INITIALIZE.as_bytes()).await;
    assert_eq!(opened.status, 200);
    assert!(header_value(&opened, SESSION_ID_HEADER).is_some());
    // The gate runs first: a rebound Host is refused before MCP sees the message.
    let rebound = [("Host", "attacker.test"), ("Authorization", bearer.as_str())];
    let refused = send_to_port(port, "POST", MCP_PATH, &rebound, INITIALIZE.as_bytes()).await;
    assert_eq!(refused.status, 403);
    assert_eq!(body_json(&refused)["code"], "RRS-C1003");
    // No server-to-client stream: GET is 405.
    let get_headers = [("Host", host.as_str()), ("Authorization", bearer.as_str())];
    let get = send_to_port(port, "GET", MCP_PATH, &get_headers, b"").await;
    assert_eq!(get.status, 405);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_is_not_a_message_or_a_revision_it_does_not_speak_is_400() {
    let paths = test_paths("mcp-malformed");
    let running = start_test_daemon(&paths).await;
    let token = create_token(&running, json!({"name": "claude-desktop", "preset": "mcp-default"})).await;

    for body in ["", "{", "[]", r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#] {
        let refused = post_mcp(&paths, Some(&token.token), None, body).await;
        assert_eq!(refused.status, 400, "{body}");
        assert_eq!(rpc_error_code(&refused), "RRS-C3001", "{body}");
    }
    let opened = post_mcp(&paths, Some(&token.token), None, INITIALIZE).await;
    let session_id = header_value(&opened, SESSION_ID_HEADER).unwrap().to_string();
    let headers = [
        ("Authorization", format!("Bearer {}", token.token)),
        ("Content-Type", "application/json".to_string()),
        ("Mcp-Session-Id", session_id),
        ("MCP-Protocol-Version", "1999-01-01".to_string()),
    ];
    let header_refs: Vec<(&str, &str)> = headers.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let refused = send_to_socket(&paths, "POST", MCP_PATH, &header_refs, TOOLS_LIST.as_bytes()).await;
    assert_eq!(refused.status, 400);
    assert_eq!(rpc_error_code(&refused), "RRS-C3001");

    // A message over axum's default 2 MiB body limit but under MAX_MESSAGE_BYTES still reaches MCP.
    let large = tools_call(5, "system__status", json!({"padding": "x".repeat(3 * 1024 * 1024)}));
    let opened = post_mcp(&paths, Some(&token.token), None, INITIALIZE).await;
    let session_id = header_value(&opened, SESSION_ID_HEADER).unwrap().to_string();
    let answered = post_mcp(&paths, Some(&token.token), Some(&session_id), &large).await;
    assert_eq!(answered.status, 200);
    assert_eq!(body_json(&answered)["result"]["isError"], true);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

// ---- Helpers ----

/// Creates a scoped token as the owner, in-process.
async fn create_token(running: &RunningDaemon, arguments: Value) -> NewToken {
    let request = execute_request("token.create", &arguments.to_string());
    let created = ok_result(&run_command(&running.server, &owner_principal(), request).await);
    NewToken {
        token_id: created["token_id"].as_str().unwrap().to_string(),
        token: created["token"].as_str().unwrap().to_string(),
    }
}

fn owner_token_text(paths: &DataPaths) -> String {
    owner_bearer(paths).trim_start_matches("Bearer ").to_string()
}

/// The `rastai mcp` bridge on these input lines, against the test daemon's daemon.sock. One reply per request.
async fn run_bridge_on_lines(paths: &DataPaths, token: &str, lines: &[&str]) -> Vec<Value> {
    let daemon = DaemonEndpoint {
        socket_path: daemon_socket_path(paths),
    };
    let mut input = Vec::new();
    for line in lines {
        input.extend_from_slice(line.as_bytes());
        input.push(b'\n');
    }
    let mut output = Vec::new();
    run_bridge(&daemon, token, input.as_slice(), &mut output).await;
    let text = String::from_utf8(output).unwrap();
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

/// One raw `POST /mcp` over daemon.sock, with the headers MCP's Streamable HTTP transport uses.
async fn post_mcp(paths: &DataPaths, token: Option<&str>, session_id: Option<&str>, body: &str) -> HttpResponse {
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json, text/event-stream".to_string()),
    ];
    if let Some(token) = token {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    if let Some(session_id) = session_id {
        headers.push(("Mcp-Session-Id".to_string(), session_id.to_string()));
    }
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    send_to_socket(paths, "POST", MCP_PATH, &header_refs, body.as_bytes()).await
}

fn tools_call(id: i64, tool_name: &str, arguments: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool_name, "arguments": arguments}})
        .to_string()
}

fn body_json(response: &HttpResponse) -> Value {
    serde_json::from_slice(&response.body).unwrap()
}

/// The catalog code a JSON-RPC error carries in its `data` (the diagnostic as JSON).
fn rpc_error_code(response: &HttpResponse) -> String {
    let body = body_json(response);
    body["error"]["data"]["code"].as_str().unwrap().to_string()
}

fn tool_names(reply: &Value) -> Vec<String> {
    let tools = reply["result"]["tools"].as_array().unwrap();
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// A failed tool call's first code: its text starts with `RRS-XNNNN:`.
fn tool_error_code(reply: &Value) -> String {
    let result = &reply["result"];
    assert_eq!(result["isError"], true, "{reply}");
    let text = result["content"][0]["text"].as_str().unwrap();
    text.split(':').next().unwrap().to_string()
}

/// The MCP names of the registry's commands whose scope is one of `scopes`, in registry order.
fn commands_with_scopes(running: &RunningDaemon, scopes: &[&str]) -> Vec<String> {
    let registry = running.server.registry.read().unwrap();
    registry
        .commands
        .iter()
        .filter(|command| scopes.contains(&command.scope.as_str()) && !command.experimental)
        .map(|command| command.command_id.replace('.', "__"))
        .collect()
}

fn mark_experimental(running: &RunningDaemon, command_id: &str) {
    let mut registry = running.server.registry.write().unwrap();
    let command = registry
        .commands
        .iter_mut()
        .find(|command| command.command_id == command_id)
        .unwrap();
    command.experimental = true;
}

/// Every row of `command_journal`.
fn journal_rows(running: &RunningDaemon) -> Vec<JournalRow> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    let rows = list_table_rows(&reader, "command_journal").unwrap();
    let mut journal = Vec::new();
    for row in &rows {
        let journal_id = row[JOURNAL_ID_COLUMN].trim_matches(TEXT_QUOTE);
        journal.push(get_journal(&reader, journal_id).unwrap().unwrap());
    }
    journal
}
