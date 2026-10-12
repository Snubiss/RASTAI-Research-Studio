//! `POST /mcp`: MCP over Streamable HTTP (layout: rust-control-plane.md 7.2, 7.3; flows/platform.md 4.3). The axum
//! handler is the only async glue: rastai-mcp turns each message into a plain `McpStep`, and a tool call runs through
//! `execute_command`, where every caller ends (gate, scope, arguments, journal).
//!
//! - **MCP runs as a bearer token.** The gate has already let the request in; MCP then takes its principal from the
//!   `Authorization: Bearer` token alone, on both listeners. daemon.sock admits its own user as the owner, but the
//!   `rastai mcp` bridge comes in over it carrying a scoped token, and the tool list and every call must be that
//!   token's (`RRS-C1005` outside its scopes), never the owner's. No bearer token is `RRS-C1002`, a revoked one
//!   `RRS-C1006`: the gate's own bearer check, so the two can't differ.
//! - **Sessions.** An `initialize` request opens a new session, whose id goes back in `Mcp-Session-Id`. Every other
//!   message names it in that header: without one it is 400, and with one this daemon doesn't hold (it restarted,
//!   the session idled out for 24 hours, or another token opened it) it is 404, after which a client initializes
//!   again (the bridge does so by itself). Sessions live in memory only, at most `MAX_MCP_SESSIONS` of them.
//! - **Answers.** A request gets HTTP 200 with one JSON-RPC reply as `application/json` (no command needs
//!   server-pushed progress yet); a notification or a client's reply gets 202 with no body. A body that isn't a
//!   JSON-RPC message, or an `MCP-Protocol-Version` this server doesn't speak, is 400 with a JSON-RPC error carrying
//!   `RRS-C3001`. `GET /mcp` is 405 (the router's method rule): there is no server-to-client stream.
//! - **Tools.** `tools/list` shows the commands the principal's scopes allow (`tools_for_principal`), experimental
//!   ones only while `plugins.experimental_apis` is on. A `tools/call` isn't checked against that list:
//!   `execute_command` decides, so a missing scope is `RRS-C1005` and an unknown command `RRS-C0001`. The one
//!   exception is an experimental command while the setting is off, which stays unknown to MCP (`RRS-C0001`).

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::{Arc, Mutex, MutexGuard, RwLockReadGuard};

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rastai_core::commands::registry::{CommandRegistry, find_command};
use rastai_core::principal::Principal;
use rastai_core::time::now_unix_ms;
use rastai_core::wire::diagnostic_from_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_mcp::error::McpError;
use rastai_mcp::jsonrpc::{
    INVALID_REQUEST, JsonRpcId, JsonRpcMessage, PARSE_ERROR, error_reply, message_json, parse_message,
};
use rastai_mcp::protocol_versions::SUPPORTED_PROTOCOL_VERSIONS;
use rastai_mcp::server::handle::{McpStep, handle_message};
use rastai_mcp::server::session::{McpSession, new_session};
use rastai_mcp::server::tools::{McpTool, ToolOutcome, tool_call_reply, tools_for_principal};
use rastai_proto::commands::v1::ExecuteRequest;

use crate::commands::execute::execute_command;
use crate::http::command_endpoints::experimental_apis_on;
use crate::http::gate::{RequestHead, check_bearer};
use crate::http::router::refusal_response;
use crate::server_state::Server;

pub(crate) const MCP_PATH: &str = "/mcp";
// MCP's Streamable HTTP headers, lowercase as axum hands them over.
const SESSION_ID_HEADER: &str = "mcp-session-id";
const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const JSON_CONTENT_TYPE: &str = "application/json";
const INITIALIZE_METHOD: &str = "initialize";
/// Sessions idle this long are dropped (layout 7.2): 24 hours, like the UI's sign-in sessions.
const SESSION_IDLE_EXPIRY_MS: i64 = 24 * 60 * 60 * 1000;
/// A token holder could otherwise grow the list without end by initializing again and again. The least recently
/// used session makes room; its client gets 404 on its next message and initializes again.
const MAX_MCP_SESSIONS: usize = 256;
const CODE_NO_CREDENTIAL: &str = "RRS-C1002";
const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";

const HTTP_OK: u16 = 200;
const HTTP_ACCEPTED: u16 = 202;
const HTTP_BAD_REQUEST: u16 = 400;
const HTTP_NOT_FOUND: u16 = 404;

/// What `/mcp` answers, as plain data; `mcp_response` turns it into the HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpReply {
    pub status: u16,
    /// Sent as `Mcp-Session-Id` on the answer to the `initialize` that opened it; empty otherwise.
    pub session_id: String,
    /// One JSON-RPC message; empty for 202.
    pub body: String,
}

/// The axum handler. The gate has run (router.rs); the principal comes from the bearer token (see the header).
pub(crate) async fn mcp_endpoint(
    State(server): State<Arc<Server>>,
    Extension(head): Extension<RequestHead>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let principal = match mcp_principal(&head, &server) {
        Ok(principal) => principal,
        Err(refusal) => return refusal_response(&refusal),
    };
    let session_id = header_text(&headers, SESSION_ID_HEADER);
    let protocol_version = header_text(&headers, PROTOCOL_VERSION_HEADER);
    let reply = handle_mcp_post(&server, &principal, &session_id, &protocol_version, &body).await;
    mcp_response(&reply)
}

/// One MCP message from `principal`. `session_id` and `protocol_version` are the request's headers, empty when absent.
pub async fn handle_mcp_post(
    server: &Server,
    principal: &Principal,
    session_id: &str,
    protocol_version: &str,
    body: &[u8],
) -> McpHttpReply {
    let message = match read_message(body) {
        Ok(message) => message,
        Err(error) => return error_http_reply(HTTP_BAD_REQUEST, None, &error),
    };
    if !protocol_version.is_empty() && !SUPPORTED_PROTOCOL_VERSIONS.contains(&protocol_version) {
        let reason = format!("MCP-Protocol-Version {protocol_version:?} isn't one this server speaks");
        return error_http_reply(HTTP_BAD_REQUEST, request_id(&message), &malformed(&reason));
    }
    let tools = principal_tools(server, principal);
    let (step, new_session_id) = match step_in_session(server, principal, session_id, message, &tools) {
        Ok(found) => found,
        Err(reply) => return reply,
    };
    match step {
        McpStep::Reply(reply) => json_http_reply(&reply, new_session_id),
        McpStep::NoReply => accepted_reply(new_session_id),
        McpStep::ExecuteTool {
            request_id,
            command_id,
            arguments_json,
        } => {
            let outcome = execute_tool(server, principal, &command_id, &arguments_json).await;
            json_http_reply(&tool_call_reply(request_id, &outcome), new_session_id)
        }
    }
}

/// The bearer token's principal, through the gate's own check. A cookie or the daemon.sock peer alone isn't enough.
fn mcp_principal(head: &RequestHead, server: &Server) -> Result<Principal, Diagnostic> {
    let Some(token) = &head.bearer_token else {
        return Err(diagnostic(CODE_NO_CREDENTIAL, &[]));
    };
    match check_bearer(token, &server.auth)? {
        Some(principal) => Ok(principal),
        None => Err(diagnostic(CODE_NO_CREDENTIAL, &[])),
    }
}

fn read_message(body: &[u8]) -> Result<JsonRpcMessage, McpError> {
    match std::str::from_utf8(body) {
        Ok(text) => parse_message(text),
        Err(_) => Err(McpError::MalformedMessage {
            reason: "the body isn't UTF-8".to_string(),
            json_rpc_code: PARSE_ERROR,
            id: None,
        }),
    }
}

/// Runs the message in its session: a new one for `initialize`, else the one the header names. The second value is
/// the id of a session this message opened, for the `Mcp-Session-Id` header; empty otherwise.
fn step_in_session(
    server: &Server,
    principal: &Principal,
    session_id: &str,
    message: JsonRpcMessage,
    tools: &[McpTool],
) -> Result<(McpStep, String), McpHttpReply> {
    if is_initialize_request(&message) {
        return start_session(server, principal, message, tools);
    }
    if session_id.is_empty() {
        let refusal = malformed("the Mcp-Session-Id header is missing; send initialize first");
        return Err(error_http_reply(HTTP_BAD_REQUEST, request_id(&message), &refusal));
    }
    let mut sessions = lock_sessions(&server.mcp_sessions);
    drop_idle_sessions(&mut sessions, now_unix_ms());
    // A session belongs to the principal that opened it: another token's request can't use it, even with its id.
    let found = sessions
        .iter_mut()
        .find(|session| session.session_id == session_id && session.principal == *principal);
    let Some(session) = found else {
        let refusal =
            malformed("this MCP session isn't known (the daemon restarted or it expired); send initialize again");
        return Err(error_http_reply(HTTP_NOT_FOUND, request_id(&message), &refusal));
    };
    // handle_message only updates the session and returns plain data: the lock is never held across an .await.
    Ok((handle_message(session, message, tools), String::new()))
}

/// A new session for this `initialize`. It is kept only when `initialize` succeeded (it then has a revision).
fn start_session(
    server: &Server,
    principal: &Principal,
    message: JsonRpcMessage,
    tools: &[McpTool],
) -> Result<(McpStep, String), McpHttpReply> {
    let mut session = match new_session(principal.clone()) {
        Ok(session) => session,
        Err(error) => return Err(error_http_reply(HTTP_OK, request_id(&message), &error)),
    };
    let step = handle_message(&mut session, message, tools);
    if session.protocol_version.is_empty() {
        return Ok((step, String::new()));
    }
    let session_id = session.session_id.clone();
    let mut sessions = lock_sessions(&server.mcp_sessions);
    drop_idle_sessions(&mut sessions, now_unix_ms());
    make_room_for_one(&mut sessions);
    sessions.push(session);
    Ok((step, session_id))
}

/// The tools this principal may call, read under the registry's lock and the settings' lock in turn.
fn principal_tools(server: &Server, principal: &Principal) -> Vec<McpTool> {
    let include_experimental = experimental_apis_on(server);
    let registry = read_registry(server);
    tools_for_principal(&registry, principal, include_experimental)
}

/// Runs the tool's command as `principal` through `execute_command`, and hands back what MCP shows of it.
async fn execute_tool(server: &Server, principal: &Principal, command_id: &str, arguments_json: &str) -> ToolOutcome {
    if is_hidden_experimental(server, command_id) {
        let refusal = diagnostic(CODE_UNKNOWN_COMMAND, &[("command_id", command_id)]);
        return ToolOutcome {
            ok: false,
            result_json: String::new(),
            job_id: String::new(),
            diagnostics: vec![refusal],
        };
    }
    let request = ExecuteRequest {
        command_id: command_id.to_string(),
        arguments_json: arguments_json.to_string(),
        ..ExecuteRequest::default()
    };
    let response = execute_command(server, principal, request).await;
    ToolOutcome {
        ok: response.ok,
        result_json: response.result_json,
        job_id: response.job_id,
        diagnostics: response.diagnostics.iter().map(diagnostic_from_proto).collect(),
    }
}

/// An experimental command isn't listed while `plugins.experimental_apis` is off, so MCP treats it as unknown
/// rather than running it (layout 7.3).
fn is_hidden_experimental(server: &Server, command_id: &str) -> bool {
    if experimental_apis_on(server) {
        return false;
    }
    let registry = read_registry(server);
    find_command(&registry, command_id).is_some_and(|command| command.experimental)
}

fn read_registry(server: &Server) -> RwLockReadGuard<'_, CommandRegistry> {
    match server.registry.read() {
        Ok(registry) => registry,
        // Each registry change is a push or a retain, so a panic mid-change leaves it whole: reads carry on.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Each change to the list is a push, a retain or a remove, so a panic mid-change leaves it whole.
fn lock_sessions(sessions: &Mutex<Vec<McpSession>>) -> MutexGuard<'_, Vec<McpSession>> {
    match sessions.lock() {
        Ok(sessions) => sessions,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn drop_idle_sessions(sessions: &mut Vec<McpSession>, now_unix_ms: i64) {
    sessions.retain(|session| now_unix_ms - session.last_used_unix_ms < SESSION_IDLE_EXPIRY_MS);
}

/// Removes the least recently used session while the list is full.
fn make_room_for_one(sessions: &mut Vec<McpSession>) {
    while sessions.len() >= MAX_MCP_SESSIONS {
        let mut oldest = 0;
        for (index, session) in sessions.iter().enumerate() {
            if session.last_used_unix_ms < sessions[oldest].last_used_unix_ms {
                oldest = index;
            }
        }
        sessions.remove(oldest);
    }
}

fn is_initialize_request(message: &JsonRpcMessage) -> bool {
    matches!(message, JsonRpcMessage::Request { method, .. } if method == INITIALIZE_METHOD)
}

fn request_id(message: &JsonRpcMessage) -> Option<JsonRpcId> {
    match message {
        JsonRpcMessage::Request { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// `RRS-C3001`, which every MCP refusal of a message carries.
fn malformed(reason: &str) -> McpError {
    McpError::MalformedMessage {
        reason: reason.to_string(),
        json_rpc_code: INVALID_REQUEST,
        id: None,
    }
}

fn error_http_reply(status: u16, id: Option<JsonRpcId>, error: &McpError) -> McpHttpReply {
    McpHttpReply {
        status,
        session_id: String::new(),
        body: message_json(&error_reply(id, error)),
    }
}

fn json_http_reply(reply: &JsonRpcMessage, session_id: String) -> McpHttpReply {
    McpHttpReply {
        status: HTTP_OK,
        session_id,
        body: message_json(reply),
    }
}

/// 202 with no body: a notification, or a client's reply to nothing this server asked.
fn accepted_reply(session_id: String) -> McpHttpReply {
    McpHttpReply {
        status: HTTP_ACCEPTED,
        session_id,
        body: String::new(),
    }
}

/// A header's value as text; empty when it is missing or isn't visible ASCII.
fn header_text(headers: &HeaderMap, name: &str) -> String {
    match headers.get(name).map(|value| value.to_str()) {
        Some(Ok(text)) => text.trim().to_string(),
        _ => String::new(),
    }
}

fn mcp_response(reply: &McpHttpReply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, reply.body.clone()).into_response();
    let headers = response.headers_mut();
    if !reply.body.is_empty() {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    }
    if reply.session_id.is_empty() {
        return response;
    }
    // A session id is 26 base32 characters, always a valid header value.
    if let Ok(session_id) = HeaderValue::from_str(&reply.session_id) {
        headers.insert(SESSION_ID_HEADER, session_id);
    }
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::principal::owner_principal;

    fn session_used_at(last_used_unix_ms: i64) -> McpSession {
        let mut session = new_session(owner_principal()).unwrap();
        session.last_used_unix_ms = last_used_unix_ms;
        session
    }

    #[test]
    fn sessions_idle_for_a_day_are_dropped() {
        let now = 10 * SESSION_IDLE_EXPIRY_MS;
        let mut sessions = vec![
            session_used_at(now - SESSION_IDLE_EXPIRY_MS),
            session_used_at(now - SESSION_IDLE_EXPIRY_MS + 1),
        ];
        let kept = sessions[1].session_id.clone();
        drop_idle_sessions(&mut sessions, now);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, kept);
    }

    #[test]
    fn a_full_list_drops_its_least_recently_used_session() {
        let mut sessions: Vec<McpSession> = (0..MAX_MCP_SESSIONS)
            .map(|number| session_used_at(1000 + number as i64))
            .collect();
        sessions.swap(0, 7);
        let oldest = sessions
            .iter()
            .min_by_key(|session| session.last_used_unix_ms)
            .unwrap()
            .session_id
            .clone();
        make_room_for_one(&mut sessions);
        assert_eq!(sessions.len(), MAX_MCP_SESSIONS - 1);
        assert!(sessions.iter().all(|session| session.session_id != oldest));
    }

    #[test]
    fn only_an_initialize_request_opens_a_session() {
        let initialize = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        let notification = parse_message(r#"{"jsonrpc":"2.0","method":"initialize"}"#).unwrap();
        let list = parse_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert!(is_initialize_request(&initialize));
        assert!(!is_initialize_request(&notification));
        assert!(!is_initialize_request(&list));
    }

    #[test]
    fn a_body_that_is_not_utf8_is_a_parse_error() {
        let refused = read_message(&[0xff, 0xfe]).unwrap_err();
        assert_eq!(refused.code(), "RRS-C3001");
        assert!(matches!(
            refused,
            McpError::MalformedMessage {
                json_rpc_code: PARSE_ERROR,
                ..
            }
        ));
    }
}
