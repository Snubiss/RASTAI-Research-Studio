//! Server transcripts (layout: rust-control-plane.md 7.6): each file of `fixtures/mcp/server/` is one MCP session,
//! replayed through `parse_message` and `handle_message` the way the daemon's `/mcp` handler runs them, and every
//! reply must equal the expected one exactly. The fixture README describes the format.
//!
//! When a step is `ExecuteTool`, the transcript says which command and arguments it must name, and gives the outcome
//! the daemon would hand back; the reply is then `tool_call_reply` of that outcome. Running the command is the
//! daemon's job (M1-30), so this crate is checked for what it asks to run, never for running it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_core::commands::registry::CommandRegistry;
use rastai_core::principal::{Principal, PrincipalKind};
use rastai_core::scopes::{Scope, parse_scope, token_preset_scopes};
use rastai_core::versions::APP_VERSION;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_mcp::jsonrpc::{JsonRpcMessage, error_reply, message_json, parse_message};
use rastai_mcp::server::handle::{McpStep, handle_message};
use rastai_mcp::server::session::{McpSession, new_session};
use rastai_mcp::server::tools::{McpTool, ToolOutcome, tool_call_reply, tools_for_principal};
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind};
use serde_json::Value;

/// From this crate's folder.
const FIXTURE_FOLDER: &str = "../../fixtures/mcp/server";
const COMMANDS_FILE: &str = "commands.json";
/// The release bumps the app version, so the expected `serverInfo.version` says this instead of a number.
const APP_VERSION_PLACEHOLDER: &str = "<app_version>";
/// The token every transcript's session runs as.
const TOKEN_ID: &str = "tok_01JA2B3C4D5E6F7G8H9J0K1M2N";
/// The transcripts, one test each below; `every_transcript_has_a_test` keeps this list and the folder equal.
const TRANSCRIPTS: &[&str] = &[
    "initialize",
    "initialize_unknown_revision",
    "malformed",
    "tool_call",
    "tool_call_job",
    "tool_call_outside_scopes",
    "tools_list_mcp_default",
];

#[test]
fn initialize_answers_with_the_client_revision() {
    replay_transcript("initialize");
}

#[test]
fn initialize_with_an_unknown_revision_gets_ours() {
    replay_transcript("initialize_unknown_revision");
}

#[test]
fn tools_list_shows_what_mcp_default_may_run() {
    replay_transcript("tools_list_mcp_default");
}

#[test]
fn a_call_is_handed_to_the_daemon_and_its_outcome_replied() {
    replay_transcript("tool_call");
}

#[test]
fn a_job_call_replies_with_its_job_id() {
    replay_transcript("tool_call_job");
}

#[test]
fn a_call_outside_the_scopes_is_never_executed() {
    replay_transcript("tool_call_outside_scopes");
}

#[test]
fn malformed_lines_get_rrs_c3001() {
    replay_transcript("malformed");
}

#[test]
fn every_transcript_has_a_test() {
    let mut found = Vec::new();
    for entry in fs::read_dir(fixture_folder()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if let Some(stem) = name.strip_suffix(".jsonl") {
            found.push(stem.to_string());
        }
    }
    found.sort();
    assert_eq!(found, TRANSCRIPTS);
}

/// Replays one transcript: the first line sets up the session, every other line is one exchange.
fn replay_transcript(name: &str) {
    let path = fixture_folder().join(format!("{name}.jsonl"));
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace(APP_VERSION_PLACEHOLDER, APP_VERSION);
    let mut lines = text.lines();
    let setup: Value = serde_json::from_str(lines.next().expect("a setup line")).unwrap();
    let principal = token_principal(&setup["session"]);
    let include_experimental = setup["session"]["include_experimental"].as_bool().unwrap_or(false);
    let tools = tools_for_principal(&load_registry(), &principal, include_experimental);
    let mut session = new_session(principal).unwrap();
    for (index, line) in lines.enumerate() {
        let exchange: Value = serde_json::from_str(line).unwrap();
        // Line numbers as an editor shows them: the setup is line 1.
        let place = format!("{}:{}", path.display(), index + 2);
        replay_exchange(&mut session, &tools, &exchange, &place);
    }
}

/// Sends the client's line, checks what the step asks to execute, and compares the reply.
fn replay_exchange(session: &mut McpSession, tools: &[McpTool], exchange: &Value, place: &str) {
    let client_text = match exchange.get("client_text") {
        Some(Value::String(text)) => text.clone(),
        _ => exchange["client"].to_string(),
    };
    let step = match parse_message(&client_text) {
        Ok(message) => handle_message(session, message, tools),
        Err(error) => McpStep::Reply(error_reply(None, &error)),
    };
    let reply = reply_of_step(step, exchange, place);
    compare_reply(reply, &exchange["server"], place);
}

fn reply_of_step(step: McpStep, exchange: &Value, place: &str) -> Option<JsonRpcMessage> {
    let expected_execute = exchange.get("execute");
    match step {
        McpStep::Reply(reply) => {
            assert!(
                expected_execute.is_none(),
                "{place}: expected an ExecuteTool step, got a reply"
            );
            Some(reply)
        }
        McpStep::NoReply => {
            assert!(
                expected_execute.is_none(),
                "{place}: expected an ExecuteTool step, got no reply"
            );
            None
        }
        McpStep::ExecuteTool {
            request_id,
            command_id,
            arguments_json,
        } => {
            let expected = expected_execute.unwrap_or_else(|| panic!("{place}: unexpected ExecuteTool {command_id}"));
            assert_eq!(command_id, expected["command_id"].as_str().unwrap(), "{place}");
            assert_eq!(arguments_json, expected["arguments_json"].as_str().unwrap(), "{place}");
            let outcome = outcome_from(&exchange["outcome"]);
            Some(tool_call_reply(request_id, &outcome))
        }
    }
}

/// The reply is one line, and the same JSON as the expected one; `null` means no reply.
fn compare_reply(reply: Option<JsonRpcMessage>, expected: &Value, place: &str) {
    let Some(reply) = reply else {
        assert!(expected.is_null(), "{place}: expected a reply, got none");
        return;
    };
    let line = message_json(&reply);
    assert!(!line.contains('\n'), "{place}: a reply must be one line");
    let found: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        &found, expected,
        "{place}: the reply differs\n found: {found}\n expected: {expected}"
    );
}

/// The daemon's outcome as the transcript gives it: `{ok, result}`, `{ok, job_id}` or `{ok, diagnostics}`.
fn outcome_from(outcome: &Value) -> ToolOutcome {
    let result_json = match outcome.get("result") {
        Some(result) => result.to_string(),
        None => String::new(),
    };
    let diagnostics = match outcome.get("diagnostics") {
        Some(Value::Array(items)) => items.iter().map(diagnostic_from).collect(),
        _ => Vec::new(),
    };
    ToolOutcome {
        ok: outcome["ok"].as_bool().unwrap(),
        result_json,
        job_id: outcome["job_id"].as_str().unwrap_or_default().to_string(),
        diagnostics,
    }
}

/// `{code, values}`: the catalog fills in the rest, as it does for the daemon.
fn diagnostic_from(item: &Value) -> Diagnostic {
    let mut values = Vec::new();
    if let Some(Value::Object(members)) = item.get("values") {
        for (name, value) in members {
            values.push((name.as_str(), value.as_str().unwrap()));
        }
    }
    diagnostic(item["code"].as_str().unwrap(), &values)
}

/// `{token_preset}` or `{scopes}`: the session runs as a scoped token, as `rastai mcp` does.
fn token_principal(session: &Value) -> Principal {
    let scopes = match session.get("token_preset") {
        Some(Value::String(preset)) => token_preset_scopes(preset).unwrap(),
        _ => scope_list(&session["scopes"]),
    };
    Principal {
        kind: PrincipalKind::Token,
        id: TOKEN_ID.to_string(),
        scopes,
    }
}

fn scope_list(names: &Value) -> Vec<Scope> {
    let names = names.as_array().expect("\"scopes\" or \"token_preset\"");
    names
        .iter()
        .map(|name| parse_scope(name.as_str().unwrap()).unwrap())
        .collect()
}

/// The registry the transcripts run against: `commands.json`, one descriptor per row, with the schemas as text the
/// way the registry holds them.
fn load_registry() -> CommandRegistry {
    let text = fs::read_to_string(fixture_folder().join(COMMANDS_FILE)).unwrap();
    let rows: Vec<Value> = serde_json::from_str(&text).unwrap();
    CommandRegistry {
        commands: rows.iter().map(descriptor_from).collect(),
    }
}

fn descriptor_from(row: &Value) -> CommandDescriptor {
    let text = |key: &str| row[key].as_str().unwrap().to_string();
    let kind = match row["kind"].as_str().unwrap() {
        "job" => CommandKind::Job,
        "stream" => CommandKind::Stream,
        _ => CommandKind::Sync,
    };
    CommandDescriptor {
        command_id: text("command_id"),
        title: text("title"),
        summary: text("summary"),
        section: text("section"),
        scope: text("scope"),
        kind: kind as i32,
        experimental: row["experimental"].as_bool().unwrap(),
        arguments_schema_json: schema_text(&row["arguments_schema"]),
        result_schema_json: schema_text(&row["result_schema"]),
        ..CommandDescriptor::default()
    }
}

/// `null` is a schema the registry doesn't have yet: empty text.
fn schema_text(schema: &Value) -> String {
    if schema.is_null() {
        String::new()
    } else {
        schema.to_string()
    }
}

fn fixture_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_FOLDER)
}
