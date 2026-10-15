//! `handle_message`: one MCP message in, one step out (layout: rust-control-plane.md 7.2, 7.3). The step is plain
//! data, so this crate never executes anything and needs no callback into the daemon: a tool call comes back as
//! `ExecuteTool`, and `rastai-server` runs it through `execute_command` (gate, scope, arguments, journal) and turns
//! the outcome into the reply with `tool_call_reply`.
//!
//! Methods: `initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`. Any other request is
//! JSON-RPC `-32601`. `resources/*` for `ui://` panels comes with two-way MCP Apps (§33).

use rastai_core::commands::naming::command_id_from_mcp_tool_name;
use rastai_core::time::now_unix_ms;
use serde_json::{Value, json};

use crate::error::{McpError, malformed_message};
use crate::jsonrpc::{INVALID_PARAMS, JsonRpcId, JsonRpcMessage, METHOD_NOT_FOUND, error_reply};
use crate::server::session::{McpSession, initialize_session, mark_initialized, require_initialize};
use crate::server::tools::{McpTool, tools_list_result};

/// The MCP member that carries the retry key of a `tools/call` (contracts 4.7). MCP keeps `_meta` for protocol
/// metadata; the name is unprefixed, as #268 names it.
const IDEMPOTENCY_KEY_MEMBER: &str = "idempotencyKey";
const META_MEMBER: &str = "_meta";

/// What the caller must do next. Plain data out, so rastai-mcp needs no callback into the daemon.
#[derive(Debug, Clone, PartialEq)]
pub enum McpStep {
    /// Send this reply.
    Reply(JsonRpcMessage),
    /// Run this command as the session's principal, then reply with `tool_call_reply(request_id, &outcome)`.
    /// `idempotency_key` is `params._meta.idempotencyKey` (contracts 4.7), empty when the call sent none.
    ExecuteTool {
        request_id: JsonRpcId,
        command_id: String,
        arguments_json: String,
        idempotency_key: String,
    },
    /// Nothing to send: the message was a notification, or a reply to nothing this server asked.
    NoReply,
}

/// Handles one message of this session. `tools` are the session principal's (`tools_for_principal`), which
/// `tools/list` shows. A `tools/call` isn't checked against them: `execute_command` decides (layout 7.5).
pub fn handle_message(session: &mut McpSession, message: JsonRpcMessage, tools: &[McpTool]) -> McpStep {
    session.last_used_unix_ms = now_unix_ms();
    match message {
        JsonRpcMessage::Request { id, method, params } => handle_request(session, id, &method, &params, tools),
        JsonRpcMessage::Notification { method, .. } => {
            handle_notification(session, &method);
            McpStep::NoReply
        }
        // This server sends no requests, so no response or error is waited for.
        JsonRpcMessage::Response { .. } | JsonRpcMessage::Error { .. } => McpStep::NoReply,
    }
}

fn handle_request(session: &mut McpSession, id: JsonRpcId, method: &str, params: &Value, tools: &[McpTool]) -> McpStep {
    let step = match method {
        "initialize" => initialize_session(session, params).map(|result| reply(&id, result)),
        "ping" => Ok(reply(&id, json!({}))),
        "tools/list" => require_initialize(session).map(|()| reply(&id, tools_list_result(tools))),
        "tools/call" => require_initialize(session).and_then(|()| call_tool(&id, params)),
        _ => Err(unknown_method(method)),
    };
    match step {
        Ok(step) => step,
        Err(error) => McpStep::Reply(error_reply(Some(id), &error)),
    }
}

/// `notifications/initialized` marks the session ready. Other notifications (`notifications/cancelled`,
/// `notifications/roots/list_changed`) need nothing from a server whose replies are single and immediate.
fn handle_notification(session: &mut McpSession, method: &str) {
    if method == "notifications/initialized" {
        mark_initialized(session);
    }
}

/// Any tool name becomes `ExecuteTool` with its command id and its arguments as JSON text. Whether the command
/// exists, the principal holds its scope and its plugin is on is `execute_command`'s decision (flows/platform.md
/// 3.2: `RRS-C0001`, `RRS-C1005`, `RRS-C0003`), so an MCP client gets the same answer the CLI would. Refusing
/// unlisted names here would answer a missing scope as an unknown command.
fn call_tool(id: &JsonRpcId, params: &Value) -> Result<McpStep, McpError> {
    let Some(Value::String(name)) = params.get("name") else {
        return Err(malformed_message(
            "tools/call needs \"name\" as a string",
            INVALID_PARAMS,
            None,
        ));
    };
    let arguments = match params.get("arguments") {
        None => json!({}),
        Some(Value::Object(members)) => Value::Object(members.clone()),
        Some(_) => {
            return Err(malformed_message(
                "tools/call \"arguments\" must be an object",
                INVALID_PARAMS,
                None,
            ));
        }
    };
    let idempotency_key = idempotency_key_of(params)?;
    Ok(McpStep::ExecuteTool {
        request_id: id.clone(),
        command_id: command_id_from_mcp_tool_name(name),
        arguments_json: arguments.to_string(),
        idempotency_key,
    })
}

/// `params._meta.idempotencyKey`: absent or empty means no key. A value that isn't a string is refused here, before
/// any command runs, with `RRS-C0002` (contracts 4.7).
fn idempotency_key_of(params: &Value) -> Result<String, McpError> {
    match params
        .get(META_MEMBER)
        .and_then(|meta| meta.get(IDEMPOTENCY_KEY_MEMBER))
    {
        None => Ok(String::new()),
        Some(Value::String(key)) => Ok(key.clone()),
        Some(other) => Err(McpError::InvalidIdempotencyKey {
            found: json_kind(other).to_string(),
        }),
    }
}

/// What a JSON value is, in words a refusal can use.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn reply(id: &JsonRpcId, result: Value) -> McpStep {
    McpStep::Reply(JsonRpcMessage::Response { id: id.clone(), result })
}

fn unknown_method(method: &str) -> McpError {
    let reason = format!("the method {method:?} isn't supported");
    malformed_message(&reason, METHOD_NOT_FOUND, None)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::server::session::new_session;
    use rastai_core::principal::owner_principal;

    fn request(id: i64, method: &str, params: Value) -> JsonRpcMessage {
        JsonRpcMessage::Request {
            id: JsonRpcId::Number(id),
            method: method.to_string(),
            params,
        }
    }

    fn initialized_session() -> McpSession {
        let mut session = new_session(owner_principal()).unwrap();
        let step = handle_message(
            &mut session,
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
            &[],
        );
        assert!(matches!(step, McpStep::Reply(JsonRpcMessage::Response { .. })));
        session
    }

    #[test]
    fn notifications_get_no_reply_and_initialized_marks_the_session() {
        let mut session = initialized_session();
        let initialized = JsonRpcMessage::Notification {
            method: "notifications/initialized".to_string(),
            params: Value::Null,
        };
        assert_eq!(handle_message(&mut session, initialized, &[]), McpStep::NoReply);
        assert!(session.initialized);
        let cancelled = JsonRpcMessage::Notification {
            method: "notifications/cancelled".to_string(),
            params: json!({"requestId": 4}),
        };
        assert_eq!(handle_message(&mut session, cancelled, &[]), McpStep::NoReply);
    }

    #[test]
    fn responses_from_the_client_get_no_reply() {
        let mut session = initialized_session();
        let response = JsonRpcMessage::Response {
            id: JsonRpcId::Number(1),
            result: json!({}),
        };
        assert_eq!(handle_message(&mut session, response, &[]), McpStep::NoReply);
    }

    #[test]
    fn an_unknown_method_is_method_not_found() {
        let mut session = initialized_session();
        let McpStep::Reply(JsonRpcMessage::Error { id, code, .. }) =
            handle_message(&mut session, request(5, "resources/list", json!({})), &[])
        else {
            panic!("an unknown method is answered with an error");
        };
        assert_eq!(id, Some(JsonRpcId::Number(5)));
        assert_eq!(code, METHOD_NOT_FOUND);
    }

    #[test]
    fn ping_works_before_initialize_and_tools_do_not() {
        let mut session = new_session(owner_principal()).unwrap();
        let pong = handle_message(&mut session, request(1, "ping", Value::Null), &[]);
        assert!(matches!(pong, McpStep::Reply(JsonRpcMessage::Response { .. })));
        let early = handle_message(&mut session, request(2, "tools/list", Value::Null), &[]);
        assert!(matches!(early, McpStep::Reply(JsonRpcMessage::Error { .. })));
    }

    #[test]
    fn a_call_to_an_unlisted_tool_is_still_handed_to_the_daemon() {
        let mut session = initialized_session();
        let params = json!({"name": "train__start", "arguments": {"recipe": "textcnn"}});
        let step = handle_message(&mut session, request(4, "tools/call", params), &[]);
        assert_eq!(
            step,
            McpStep::ExecuteTool {
                request_id: JsonRpcId::Number(4),
                command_id: "train.start".to_string(),
                arguments_json: r#"{"recipe":"textcnn"}"#.to_string(),
                idempotency_key: String::new(),
            }
        );
    }

    fn tool_call_with_meta(meta: Value) -> JsonRpcMessage {
        let params = json!({"name": "train__start", "arguments": {"recipe": "textcnn"}, "_meta": meta});
        request(6, "tools/call", params)
    }

    fn key_sent_with(meta: Value) -> String {
        let mut session = initialized_session();
        match handle_message(&mut session, tool_call_with_meta(meta), &[]) {
            McpStep::ExecuteTool { idempotency_key, .. } => idempotency_key,
            other => panic!("expected ExecuteTool, got {other:?}"),
        }
    }

    #[test]
    fn meta_idempotency_key_is_the_key_the_command_runs_with() {
        assert_eq!(key_sent_with(json!({"idempotencyKey": "retry-7"})), "retry-7");
    }

    #[test]
    fn a_missing_or_empty_idempotency_key_means_no_key() {
        assert_eq!(key_sent_with(json!({})), "");
        assert_eq!(key_sent_with(json!({"idempotencyKey": ""})), "");
        let mut session = initialized_session();
        let params = json!({"name": "system__status", "arguments": {}});
        let step = handle_message(&mut session, request(7, "tools/call", params), &[]);
        assert!(matches!(step, McpStep::ExecuteTool { ref idempotency_key, .. } if idempotency_key.is_empty()));
    }

    #[test]
    fn a_non_string_idempotency_key_is_refused_before_the_command_runs() {
        for (meta, kind) in [
            (json!({"idempotencyKey": 7}), "a number"),
            (json!({"idempotencyKey": null}), "null"),
            (json!({"idempotencyKey": ["k"]}), "an array"),
            (json!({"idempotencyKey": {"key": "k"}}), "an object"),
        ] {
            let mut session = initialized_session();
            let McpStep::Reply(JsonRpcMessage::Error { id, code, data, .. }) =
                handle_message(&mut session, tool_call_with_meta(meta), &[])
            else {
                panic!("a non-string key is answered with an error, not run");
            };
            assert_eq!(id, Some(JsonRpcId::Number(6)));
            assert_eq!(code, INVALID_PARAMS);
            assert_eq!(data["code"], "RRS-C0002");
            assert!(data["message"].as_str().unwrap().contains(kind), "{data}");
        }
    }

    #[test]
    fn call_arguments_must_be_an_object() {
        let mut session = initialized_session();
        let params = json!({"name": "system__status", "arguments": [1]});
        let McpStep::Reply(JsonRpcMessage::Error { code, .. }) =
            handle_message(&mut session, request(3, "tools/call", params), &[])
        else {
            panic!("bad arguments are answered with an error");
        };
        assert_eq!(code, INVALID_PARAMS);
    }
}
