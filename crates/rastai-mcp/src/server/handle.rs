//! `handle_message`: one MCP message in, one step out (layout: rust-control-plane.md 7.2, 7.3). The step is plain
//! data, so this crate never executes anything and needs no callback into the daemon: a tool call comes back as
//! `ExecuteTool`, and `rastai-server` runs it through `execute_command` (gate, scope, arguments, journal) and turns
//! the outcome into the reply with `tool_call_reply`.
//!
//! Methods: `initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`. Any other request is
//! JSON-RPC `-32601`. `resources/*` for `ui://` panels comes with two-way MCP Apps (§33).

use rastai_core::commands::naming::command_id_from_mcp_tool_name;
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::diagnostic;
use serde_json::{Value, json};

use crate::error::McpError;
use crate::jsonrpc::{INVALID_PARAMS, JsonRpcId, JsonRpcMessage, METHOD_NOT_FOUND, diagnostic_reply, error_reply};
use crate::server::session::{McpSession, initialize_session, mark_initialized, require_initialize};
use crate::server::tools::{McpTool, tools_list_result};

/// A tool name the session doesn't list (contracts 10.3).
const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";

/// What the caller must do next. Plain data out, so rastai-mcp needs no callback into the daemon.
#[derive(Debug, Clone, PartialEq)]
pub enum McpStep {
    /// Send this reply.
    Reply(JsonRpcMessage),
    /// Run this command as the session's principal, then reply with `tool_call_reply(request_id, &outcome)`.
    ExecuteTool {
        request_id: JsonRpcId,
        command_id: String,
        arguments_json: String,
    },
    /// Nothing to send: the message was a notification, or a reply to nothing this server asked.
    NoReply,
}

/// Handles one message of this session. `tools` are the session principal's (`tools_for_principal`): a tool call
/// asks to execute only a tool listed there, so an MCP client can never reach a command its token can't see.
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
        "tools/call" => require_initialize(session).and_then(|()| call_tool(&id, params, tools)),
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

/// A listed tool becomes `ExecuteTool` with its arguments as JSON text. A name the session doesn't list is refused
/// here (JSON-RPC `-32602`, as MCP says for an unknown tool) and never reaches the daemon.
fn call_tool(id: &JsonRpcId, params: &Value, tools: &[McpTool]) -> Result<McpStep, McpError> {
    let Some(Value::String(name)) = params.get("name") else {
        return Err(invalid_params("tools/call needs \"name\" as a string"));
    };
    let arguments = match params.get("arguments") {
        None => json!({}),
        Some(Value::Object(members)) => Value::Object(members.clone()),
        Some(_) => return Err(invalid_params("tools/call \"arguments\" must be an object")),
    };
    let Some(tool) = tools.iter().find(|tool| tool.name == *name) else {
        let command_id = command_id_from_mcp_tool_name(name);
        let unknown = diagnostic(CODE_UNKNOWN_COMMAND, &[("command_id", &command_id)]);
        return Ok(McpStep::Reply(diagnostic_reply(
            Some(id.clone()),
            INVALID_PARAMS,
            &unknown,
        )));
    };
    Ok(McpStep::ExecuteTool {
        request_id: id.clone(),
        command_id: tool.command_id.clone(),
        arguments_json: arguments.to_string(),
    })
}

fn reply(id: &JsonRpcId, result: Value) -> McpStep {
    McpStep::Reply(JsonRpcMessage::Response { id: id.clone(), result })
}

fn unknown_method(method: &str) -> McpError {
    McpError::MalformedMessage {
        reason: format!("the method {method:?} isn't supported"),
        json_rpc_code: METHOD_NOT_FOUND,
        id: None,
    }
}

fn invalid_params(reason: &str) -> McpError {
    McpError::MalformedMessage {
        reason: reason.to_string(),
        json_rpc_code: INVALID_PARAMS,
        id: None,
    }
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
