//! `run_stdio_bridge`, behind `rastai mcp` (layout: rust-control-plane.md 7.2; flows/platform.md 4.3): an MCP client
//! starts it and speaks newline-delimited JSON-RPC on its stdin and stdout; the bridge forwards each message as
//! `POST /mcp` over daemon.sock with the scoped token, and writes each reply back as one line.
//!
//! - **The token only.** The bridge gets its token as an argument (`rastai-cli` reads `RASTAI_TOKEN`). Without one
//!   it answers every request with `RRS-C3002` and never contacts the daemon: it never falls back to the owner, so an
//!   agent gets scopes only when the owner grants them.
//! - **Checked before it is sent.** Every line goes through `parse_message`; a malformed one is answered here with
//!   `RRS-C3001`, and the daemon only ever sees whole, re-written messages.
//! - **A lost session is restored once.** After a daemon restart `/mcp` answers 404 to the old `Mcp-Session-Id`. The
//!   bridge then replays the client's `initialize` (and `notifications/initialized`, if the client sent it), takes the
//!   new session id, and retries the message once (MCP Streamable HTTP, "Session Management").
//! - **One message at a time, in order.** Every daemon reply is one JSON body (layout 7.2), so a reply is complete
//!   when the next line is read.

use std::path::PathBuf;

use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{HttpRequest, HttpResponse, HttpTarget, send_request};
use serde_json::{Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::{McpError, malformed_message};
use crate::jsonrpc::{
    INVALID_REQUEST, JsonRpcId, JsonRpcMessage, MAX_MESSAGE_BYTES, RASTAI_ERROR, diagnostic_reply, error_reply,
    message_json, parse_message,
};

const MCP_PATH: &str = "/mcp";
/// Every message goes to `/mcp` as a POST (MCP Streamable HTTP).
const POST_METHOD: &str = "POST";
/// A sync command runs inside the call; a job command returns its job_id at once. Five minutes covers the slowest
/// sync command with room to spare, and still ends a request the daemon lost.
const REQUEST_TIMEOUT_MS: u64 = 300_000;
// HTTP header names, as MCP's Streamable HTTP transport spells them.
const AUTHORIZATION_HEADER: &str = "Authorization";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const ACCEPT_HEADER: &str = "Accept";
const SESSION_ID_HEADER: &str = "Mcp-Session-Id";
const PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const JSON_CONTENT_TYPE: &str = "application/json";
/// MCP asks a client to accept both; the daemon answers with JSON only.
const ACCEPTED_CONTENT_TYPES: &str = "application/json, text/event-stream";
/// What `/mcp` answers to a session id it doesn't know: the daemon restarted, or the session idled out.
const HTTP_NOT_FOUND: u16 = 404;
const INITIALIZE_METHOD: &str = "initialize";
const INITIALIZED_NOTIFICATION: &str = "notifications/initialized";
const CODE_CONNECTION_FAILED: &str = "RRS-N2008";

/// Where the daemon listens: daemon.sock (`rastai_core::paths::daemon_socket_path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub socket_path: PathBuf,
}

/// What the bridge remembers about its daemon session.
#[derive(Debug, Default)]
struct BridgeSession {
    /// From the `initialize` reply's `Mcp-Session-Id`; empty before it.
    session_id: String,
    /// From the `initialize` reply's `protocolVersion`; sent as `MCP-Protocol-Version` after it.
    protocol_version: String,
    /// The client's `initialize`, replayed when the daemon lost the session.
    initialize_message: Option<JsonRpcMessage>,
    client_sent_initialized: bool,
}

/// One line of input, as `read_capped_line` found it.
enum LineRead {
    Line(Vec<u8>),
    /// Longer than `MAX_MESSAGE_BYTES`; skipped up to its newline without keeping it.
    TooLong,
    End,
}

/// `rastai mcp`: the bridge on this process's stdin and stdout. Returns when the client closes stdin.
pub async fn run_stdio_bridge(daemon: &DaemonEndpoint, token: &str) {
    let input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    run_bridge(daemon, token, input, &mut output).await;
}

/// The bridge over any line stream: `run_stdio_bridge` gives it stdin and stdout, the tests an in-memory pair.
/// Returns at the end of the input, or when the output can't be written (the client is gone).
pub async fn run_bridge(
    daemon: &DaemonEndpoint,
    token: &str,
    mut input: impl AsyncBufRead + Unpin,
    output: &mut (impl AsyncWrite + Unpin),
) {
    let mut session = BridgeSession::default();
    loop {
        let reply = match read_capped_line(&mut input).await {
            Ok(LineRead::Line(line)) => answer_line(daemon, token, &mut session, &line).await,
            Ok(LineRead::TooLong) => Some(error_reply(
                None,
                &malformed_message(&too_long_reason(), INVALID_REQUEST, None),
            )),
            Ok(LineRead::End) => return,
            Err(error) => {
                // Nothing can be answered on input that can't be read: the client is gone or broken.
                log_warn("rastai mcp: reading stdin failed", &[("reason", &error.to_string())]);
                return;
            }
        };
        // A failed write means the client closed its end: nobody is left to answer.
        if let Some(reply) = reply
            && write_line(output, &message_json(&reply)).await.is_err()
        {
            return;
        }
    }
}

/// The reply to one input line, if it gets one: a request always does; a notification or a reply never does.
async fn answer_line(
    daemon: &DaemonEndpoint,
    token: &str,
    session: &mut BridgeSession,
    line: &[u8],
) -> Option<JsonRpcMessage> {
    let Ok(text) = std::str::from_utf8(line) else {
        return Some(error_reply(
            None,
            &malformed_message("the line isn't UTF-8", INVALID_REQUEST, None),
        ));
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let message = match parse_message(text) {
        Ok(message) => message,
        Err(error) => return Some(error_reply(None, &error)),
    };
    if token.trim().is_empty() {
        return request_id(&message).map(|id| error_reply(Some(id), &McpError::NoToken));
    }
    forward_message(daemon, token, session, message).await
}

/// Sends a message to `/mcp`, restoring a lost session once, and turns the answer into the client's reply.
async fn forward_message(
    daemon: &DaemonEndpoint,
    token: &str,
    session: &mut BridgeSession,
    message: JsonRpcMessage,
) -> Option<JsonRpcMessage> {
    let is_initialize = is_call_to(&message, INITIALIZE_METHOD);
    if is_initialize {
        start_new_session(session, &message);
    }
    if is_call_to(&message, INITIALIZED_NOTIFICATION) {
        session.client_sent_initialized = true;
    }
    let had_session = !session.session_id.is_empty();
    let mut answer = post_message(daemon, token, session, &message).await;
    if had_session && is_status(&answer, HTTP_NOT_FOUND) && restore_session(daemon, token, session).await {
        answer = post_message(daemon, token, session, &message).await;
    }
    if is_initialize {
        remember_session(session, &answer);
    }
    let id = request_id(&message)?;
    Some(reply_from_answer(daemon, id, answer))
}

/// A client that initializes again starts a new session, as it would against the daemon directly: nothing of the
/// old one is kept, so a later restore replays only what the client sent for the new one.
fn start_new_session(session: &mut BridgeSession, initialize: &JsonRpcMessage) {
    *session = BridgeSession {
        initialize_message: Some(initialize.clone()),
        ..BridgeSession::default()
    };
}

/// Replays the client's `initialize` on a new session after the daemon lost the old one. True when the daemon
/// accepted it, so the failed message can be sent once more.
async fn restore_session(daemon: &DaemonEndpoint, token: &str, session: &mut BridgeSession) -> bool {
    session.session_id.clear();
    let Some(initialize) = session.initialize_message.clone() else {
        return false;
    };
    let answer = post_message(daemon, token, session, &initialize).await;
    if !remember_session(session, &answer) {
        return false;
    }
    if session.client_sent_initialized {
        let initialized = JsonRpcMessage::Notification {
            method: INITIALIZED_NOTIFICATION.to_string(),
            params: Value::Null,
        };
        // Its answer is an empty 202; if it fails, the retried message gets the daemon's own refusal.
        let _ = post_message(daemon, token, session, &initialized).await;
    }
    true
}

/// Keeps the session id and revision of an `initialize` the daemon accepted. False when it didn't.
fn remember_session(session: &mut BridgeSession, answer: &Result<HttpResponse, TransportError>) -> bool {
    let Ok(response) = answer else {
        return false;
    };
    let Ok(JsonRpcMessage::Response { result, .. }) = parse_body(&response.body) else {
        return false;
    };
    if !is_success(response.status) {
        return false;
    }
    session.session_id = header_value(response, SESSION_ID_HEADER).unwrap_or_default();
    if let Some(Value::String(version)) = result.get("protocolVersion") {
        session.protocol_version = version.clone();
    }
    true
}

async fn post_message(
    daemon: &DaemonEndpoint,
    token: &str,
    session: &BridgeSession,
    message: &JsonRpcMessage,
) -> Result<HttpResponse, TransportError> {
    let request = HttpRequest {
        method: POST_METHOD.to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: daemon.socket_path.clone(),
        },
        path: MCP_PATH.to_string(),
        headers: request_headers(token, session, message),
        body: message_json(message).into_bytes(),
        timeout_ms: REQUEST_TIMEOUT_MS,
        // The daemon's socket is local, so no egress tag applies (only HTTPS requests carry one).
        egress: None,
    };
    send_request(request).await
}

/// The bearer token on every request; the session id and revision on every message after `initialize`.
fn request_headers(token: &str, session: &BridgeSession, message: &JsonRpcMessage) -> Vec<(String, String)> {
    let mut headers = vec![
        (AUTHORIZATION_HEADER.to_string(), format!("Bearer {}", token.trim())),
        (CONTENT_TYPE_HEADER.to_string(), JSON_CONTENT_TYPE.to_string()),
        (ACCEPT_HEADER.to_string(), ACCEPTED_CONTENT_TYPES.to_string()),
    ];
    if is_call_to(message, INITIALIZE_METHOD) {
        return headers;
    }
    if !session.session_id.is_empty() {
        headers.push((SESSION_ID_HEADER.to_string(), session.session_id.clone()));
    }
    if !session.protocol_version.is_empty() {
        headers.push((PROTOCOL_VERSION_HEADER.to_string(), session.protocol_version.clone()));
    }
    headers
}

/// The client's reply to request `id`: the daemon's JSON-RPC reply when it sent one, else an error carrying why
/// there is none (the daemon unreachable, a refusal before MCP, or an answer that isn't JSON-RPC).
fn reply_from_answer(
    daemon: &DaemonEndpoint,
    id: JsonRpcId,
    answer: Result<HttpResponse, TransportError>,
) -> JsonRpcMessage {
    let response = match answer {
        Ok(response) => response,
        Err(error) => return diagnostic_reply(Some(id), RASTAI_ERROR, &transport_diagnostic(daemon, &error)),
    };
    if let Ok(reply) = parse_body(&response.body) {
        return reply;
    }
    let refusal = match daemon_diagnostic(&response.body) {
        Some(refusal) => refusal,
        None => {
            let reason = format!("the daemon answered HTTP {} without a JSON-RPC reply", response.status);
            connection_failed(daemon, &reason)
        }
    };
    diagnostic_reply(Some(id), RASTAI_ERROR, &refusal)
}

fn parse_body(body: &[u8]) -> Result<JsonRpcMessage, McpError> {
    match std::str::from_utf8(body) {
        Ok(text) => parse_message(text),
        Err(_) => Err(malformed_message(
            "the daemon's reply isn't UTF-8",
            INVALID_REQUEST,
            None,
        )),
    }
}

/// The daemon's request gate refuses before MCP (401 `RRS-C1002`, a revoked token `RRS-C1006`) with a diagnostic as
/// JSON (`render_json`). Its code, message and fix are passed on as they are.
fn daemon_diagnostic(body: &[u8]) -> Option<Diagnostic> {
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(body) else {
        return None;
    };
    let code = text_member(&object, "code");
    if code.is_empty() {
        return None;
    }
    Some(Diagnostic {
        code,
        severity: Severity::Error,
        message: text_member(&object, "message"),
        path: text_member(&object, "path"),
        fix_text: text_member(&object, "fix_text"),
        fix_command_id: text_member(&object, "fix_command_id"),
        fix_arguments_json: text_member(&object, "fix_arguments_json"),
    })
}

/// A string member of a JSON object; empty when it is missing or not a string.
fn text_member(object: &Map<String, Value>, key: &str) -> String {
    match object.get(key) {
        Some(Value::String(value)) => value.clone(),
        _ => String::new(),
    }
}

/// The transport's error as its catalog diagnostic, with the daemon's socket as the path.
fn transport_diagnostic(daemon: &DaemonEndpoint, error: &TransportError) -> Diagnostic {
    let socket_path = daemon.socket_path.display().to_string();
    match error {
        TransportError::SocketNotReachable { .. } | TransportError::PeerIsAnotherUser { .. } => {
            diagnostic(error.code(), &[("path", &socket_path)])
        }
        TransportError::SocketFolderInsecure { folder, .. } => {
            diagnostic(error.code(), &[("path", &folder.display().to_string())])
        }
        TransportError::SocketNotCreated { reason, .. } | TransportError::ConnectionFailed { reason } => {
            diagnostic(error.code(), &[("path", &socket_path), ("reason", reason)])
        }
        TransportError::TimedOut { timeout_ms } => {
            connection_failed(daemon, &format!("no answer within {timeout_ms} ms"))
        }
        TransportError::NotSupportedOnThisPlatform { .. } => {
            diagnostic(error.code(), &[("platform", std::env::consts::OS)])
        }
        TransportError::EgressTagMissing { host } => diagnostic(error.code(), &[("host", host)]),
    }
}

fn connection_failed(daemon: &DaemonEndpoint, reason: &str) -> Diagnostic {
    let socket_path = daemon.socket_path.display().to_string();
    diagnostic(CODE_CONNECTION_FAILED, &[("path", &socket_path), ("reason", reason)])
}

/// Reads up to the next newline, keeping at most `MAX_MESSAGE_BYTES`: a longer line is skipped, never held whole.
/// The last line may end without a newline.
async fn read_capped_line(input: &mut (impl AsyncBufRead + Unpin)) -> std::io::Result<LineRead> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = input.fill_buf().await?;
        if available.is_empty() {
            return Ok(finished_line(line, too_long, true));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let taken = newline.unwrap_or(available.len());
        if !too_long {
            line.extend_from_slice(&available[..taken]);
            too_long = line.len() > MAX_MESSAGE_BYTES;
            if too_long {
                line = Vec::new();
            }
        }
        match newline {
            Some(position) => {
                input.consume(position + 1);
                return Ok(finished_line(line, too_long, false));
            }
            None => input.consume(taken),
        }
    }
}

fn finished_line(line: Vec<u8>, too_long: bool, at_end: bool) -> LineRead {
    if too_long {
        LineRead::TooLong
    } else if at_end && line.is_empty() {
        LineRead::End
    } else {
        LineRead::Line(line)
    }
}

fn too_long_reason() -> String {
    format!("the message is over the {MAX_MESSAGE_BYTES}-byte limit")
}

async fn write_line(output: &mut (impl AsyncWrite + Unpin), line: &str) -> std::io::Result<()> {
    output.write_all(line.as_bytes()).await?;
    output.write_all(b"\n").await?;
    output.flush().await
}

fn request_id(message: &JsonRpcMessage) -> Option<JsonRpcId> {
    match message {
        JsonRpcMessage::Request { id, .. } => Some(id.clone()),
        _ => None,
    }
}

fn is_call_to(message: &JsonRpcMessage, wanted: &str) -> bool {
    match message {
        JsonRpcMessage::Request { method, .. } | JsonRpcMessage::Notification { method, .. } => method == wanted,
        _ => false,
    }
}

fn is_status(answer: &Result<HttpResponse, TransportError>, status: u16) -> bool {
    matches!(answer, Ok(response) if response.status == status)
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// A response header's value; HTTP header names are case-insensitive.
fn header_value(response: &HttpResponse, name: &str) -> Option<String> {
    response
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn lines_of(input: &[u8]) -> Vec<String> {
        let mut reader = input;
        let mut lines = Vec::new();
        loop {
            match read_capped_line(&mut reader).await.unwrap() {
                LineRead::Line(line) => lines.push(String::from_utf8(line).unwrap()),
                LineRead::TooLong => lines.push("<too long>".to_string()),
                LineRead::End => return lines,
            }
        }
    }

    #[tokio::test]
    async fn lines_are_split_at_newlines_and_the_last_may_lack_one() {
        assert_eq!(lines_of(b"a\nbc\n\nd").await, ["a", "bc", "", "d"]);
        assert!(lines_of(b"").await.is_empty());
    }

    #[tokio::test]
    async fn a_line_over_the_limit_is_skipped_and_the_next_one_read() {
        let mut input = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        input.extend_from_slice(b"\nnext\n");
        assert_eq!(lines_of(&input).await, ["<too long>", "next"]);
        let exact = vec![b'y'; MAX_MESSAGE_BYTES];
        assert_eq!(lines_of(&exact).await[0].len(), MAX_MESSAGE_BYTES);
    }

    #[test]
    fn initialize_carries_no_session_and_later_messages_do() {
        let session = BridgeSession {
            session_id: "S1".to_string(),
            protocol_version: "2025-06-18".to_string(),
            ..BridgeSession::default()
        };
        let initialize = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        let names =
            |headers: Vec<(String, String)>| -> Vec<String> { headers.into_iter().map(|(name, _)| name).collect() };
        assert_eq!(
            names(request_headers(" tok_secret ", &session, &initialize)),
            ["Authorization", "Content-Type", "Accept"]
        );
        let list = parse_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let headers = request_headers("tok_secret", &session, &list);
        assert_eq!(headers[0].1, "Bearer tok_secret");
        assert!(headers.contains(&("Mcp-Session-Id".to_string(), "S1".to_string())));
        assert!(headers.contains(&("MCP-Protocol-Version".to_string(), "2025-06-18".to_string())));
    }

    #[test]
    fn a_second_initialize_forgets_the_old_session() {
        let first = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        let second = parse_message(r#"{"jsonrpc":"2.0","id":9,"method":"initialize","params":{}}"#).unwrap();
        let mut session = BridgeSession {
            session_id: "S1".to_string(),
            protocol_version: "2025-06-18".to_string(),
            initialize_message: Some(first),
            client_sent_initialized: true,
        };
        start_new_session(&mut session, &second);
        assert!(session.session_id.is_empty());
        assert!(session.protocol_version.is_empty());
        assert!(!session.client_sent_initialized);
        assert_eq!(session.initialize_message, Some(second));
    }

    #[test]
    fn a_gate_refusal_keeps_its_code() {
        let body = br#"{"code":"RRS-C1006","severity":"error","message":"This token was revoked.","path":"","fix_text":"Create a new token.","fix_command_id":"","fix_arguments_json":""}"#;
        let refusal = daemon_diagnostic(body).unwrap();
        assert_eq!(refusal.code, "RRS-C1006");
        assert_eq!(refusal.fix_text, "Create a new token.");
        assert!(daemon_diagnostic(b"<html>").is_none());
        assert!(daemon_diagnostic(br#"{"message":"no code"}"#).is_none());
    }
}
