//! `build_router`: every route of contracts 3.3 that exists so far, behind the request gate. The same router serves
//! loopback TCP and daemon.sock (listeners.rs); the gate tells the two apart by the peer a daemon.sock connection
//! carries.
//!
//! M1-26 brought `/api/health` and `POST /api/login`, M1-27 `Execute` and `ListCommands`, M1-30 `POST /mcp`.
//! Subscribe (M1-28), blobs (M3), `/v1` (M3), plugin assets and the web app join here as they land.
//!
//! A gate refusal on a Connect path (`/rastai.`) comes back in the Connect error envelope (connect.rs), so a Connect
//! client reads it as an error with its diagnostic attached; on any other path it is the diagnostic as JSON.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Extension, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use rastai_core::principal::Principal;
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::render_json::render_json;
use rastai_mcp::jsonrpc::MAX_MESSAGE_BYTES;
use serde_json::{Value, json};

use crate::http::command_endpoints::{EXECUTE_PATH, LIST_COMMANDS_PATH, execute_endpoint, list_commands_endpoint};
use crate::http::connect::connect_error_response;
use crate::http::gate::{
    HEALTH_PATH, LOGIN_PATH, Listener, MISSING_HEADER_TEXT, RequestHead, check_request_gate, read_request_head,
};
use crate::http::health::health_endpoint;
use crate::http::listeners::SocketPeer;
use crate::http::mcp_endpoint::{MCP_PATH, mcp_endpoint};
use crate::http::sessions::{NewSession, redeem_login_code, session_cookie_header};
use crate::server_state::Server;

const JSON_CONTENT_TYPE: &str = "application/json";
/// Answers about sign-in must never be cached: they carry a cookie, a key or a refusal for this request only.
const NO_STORE: &str = "no-store";
/// `POST /api/login` reads `{"code": "<64 lowercase hex>"}` and answers `{"session_key": "<64 lowercase hex>"}`
/// (contracts 3.3).
const LOGIN_CODE_FIELD: &str = "code";
const SESSION_KEY_FIELD: &str = "session_key";
const LOGIN_BODY_EXPECTED: &str = "{\"code\": \"<64 lowercase hex>\"}";
const LOGIN_BODY_WITHOUT_CODE: &str = "a body without a code";
/// HTTP status codes are used only for credential problems and transport errors (layout section 8.5).
const UNAUTHENTICATED_CODES: &[&str] = &["RRS-C1002", "RRS-C1006", "RRS-C1007"];
const FORBIDDEN_CODES: &[&str] = &["RRS-C1003", "RRS-C1004", "RRS-C1005"];
const BAD_REQUEST_CODES: &[&str] = &["RRS-C0002"];
/// Every Connect service path starts with its protobuf package (contracts 3.3).
const CONNECT_PATH_PREFIX: &str = "/rastai.";

/// The daemon's routes with the gate in front of every one of them, the fallback included, so an unknown path is
/// refused like a known one before anyone learns it doesn't exist.
pub fn build_router(server: Arc<Server>) -> Router {
    let routes: Router<Arc<Server>> = Router::new()
        .route(HEALTH_PATH, get(health_endpoint))
        .route(LOGIN_PATH, post(login_endpoint))
        .route(EXECUTE_PATH, post(execute_endpoint))
        .route(LIST_COMMANDS_PATH, post(list_commands_endpoint))
        // POST only, so GET /mcp is 405: there is no server-to-client stream (layout 7.2). The bridge sends
        // messages up to MAX_MESSAGE_BYTES, over axum's default body limit; the parser refuses anything larger.
        .route(
            MCP_PATH,
            post(mcp_endpoint).layer(DefaultBodyLimit::max(MAX_MESSAGE_BYTES)),
        )
        .fallback(not_found);
    let gated = routes.layer(middleware::from_fn_with_state(server.clone(), gate_middleware));
    gated.with_state(server)
}

/// Runs the gate. A request that passes carries its `RequestHead` and principal on to the handler.
async fn gate_middleware(State(server): State<Arc<Server>>, mut request: Request, next: Next) -> Response {
    let listener = listener_of(&request);
    let is_connect_path = request.uri().path().starts_with(CONNECT_PATH_PREFIX);
    let head = match request_head_of(&request, &server.auth.cookie_name) {
        Ok(head) => head,
        Err(refusal) => return gate_refusal(&refusal, is_connect_path),
    };
    let principal: Option<Principal> = match check_request_gate(&head, &listener, &server.auth) {
        Ok(principal) => principal,
        Err(refusal) => return gate_refusal(&refusal, is_connect_path),
    };
    request.extensions_mut().insert(head);
    request.extensions_mut().insert(principal);
    next.run(request).await
}

fn gate_refusal(refusal: &Diagnostic, is_connect_path: bool) -> Response {
    if is_connect_path {
        return connect_error_response(refusal);
    }
    refusal_response(refusal)
}

/// Only daemon.sock connections carry a `SocketPeer` (listeners.rs), so a request without one came over TCP and
/// gets the full Host, Origin and credential checks.
fn listener_of(request: &Request) -> Listener {
    match request.extensions().get::<ConnectInfo<SocketPeer>>() {
        Some(ConnectInfo(peer)) => Listener::DaemonSocket { peer_uid: peer.user_id },
        None => Listener::LoopbackTcp,
    }
}

fn request_head_of(request: &Request, cookie_name: &str) -> Result<RequestHead, Diagnostic> {
    let headers: Vec<(&str, &[u8])> = request
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_bytes()))
        .collect();
    read_request_head(request.method().as_str(), request.uri().path(), &headers, cookie_name)
}

/// `POST /api/login`: redeems the code and binds the new session to the request's Origin. The gate has already
/// refused a login without an Origin, and an Origin that isn't `http://` plus Host.
async fn login_endpoint(
    State(server): State<Arc<Server>>,
    Extension(head): Extension<RequestHead>,
    body: Bytes,
) -> Response {
    let Some(origin) = head.origin else {
        // Unreachable behind the gate; checked again so a session can never be bound to no origin.
        return refusal_response(&diagnostic("RRS-C1004", &[("origin", MISSING_HEADER_TEXT)]));
    };
    let code = match read_login_code(&body) {
        Ok(code) => code,
        Err(refusal) => return refusal_response(&refusal),
    };
    match redeem_login_code(&server.auth, &code, &origin, now_unix_ms()) {
        Ok(new_session) => login_answer(&server, &new_session),
        Err(refusal) => refusal_response(&refusal),
    }
}

/// The code from `{"code": "..."}`. Anything else is `RRS-C0002`, saying what was expected.
fn read_login_code(body: &[u8]) -> Result<String, Diagnostic> {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| value.get(LOGIN_CODE_FIELD))
        .and_then(Value::as_str);
    match code {
        Some(code) => Ok(code.to_string()),
        None => Err(diagnostic(
            "RRS-C0002",
            &[("found", LOGIN_BODY_WITHOUT_CODE), ("expected", LOGIN_BODY_EXPECTED)],
        )),
    }
}

/// 200 with the session cookie, and the request key in the body; the web app keeps the key in memory only.
fn login_answer(server: &Server, new_session: &NewSession) -> Response {
    let cookie = session_cookie_header(&server.auth, &new_session.session_id);
    let body = json!({ SESSION_KEY_FIELD: new_session.session_key });
    let mut response = json_response(StatusCode::OK, body.to_string());
    match HeaderValue::from_str(&cookie) {
        Ok(cookie_value) => {
            response.headers_mut().insert(header::SET_COOKIE, cookie_value);
        }
        // The cookie is a fixed name and 64 hex digits, always a valid header; failing closed costs only a retry.
        Err(_) => return refusal_response(&diagnostic("RRS-C1002", &[])),
    }
    response
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// A refusal: the diagnostic as JSON (the fields of `rastai.common.v1.Diagnostic`), with 401 for a missing or bad
/// credential, 403 for a refused Host, Origin or scope, 400 for a malformed body.
pub fn refusal_response(refusal: &Diagnostic) -> Response {
    json_response(status_for_code(&refusal.code), render_json(refusal).to_string())
}

fn status_for_code(code: &str) -> StatusCode {
    if UNAUTHENTICATED_CODES.contains(&code) {
        return StatusCode::UNAUTHORIZED;
    }
    if FORBIDDEN_CODES.contains(&code) {
        return StatusCode::FORBIDDEN;
    }
    if BAD_REQUEST_CODES.contains(&code) {
        return StatusCode::BAD_REQUEST;
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

fn json_response(status: StatusCode, body: String) -> Response {
    let headers = [
        (header::CONTENT_TYPE, JSON_CONTENT_TYPE),
        (header::CACHE_CONTROL, NO_STORE),
    ];
    (status, headers, body).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_login_body_carries_the_code() {
        assert_eq!(read_login_code(br#"{"code": "ab12"}"#).unwrap(), "ab12");
        for body in [&b""[..], b"{}", b"[]", br#"{"code": 12}"#, b"code=ab12"] {
            assert_eq!(read_login_code(body).unwrap_err().code, "RRS-C0002");
        }
    }

    #[test]
    fn refusals_map_to_credential_statuses() {
        assert_eq!(status_for_code("RRS-C1002"), StatusCode::UNAUTHORIZED);
        assert_eq!(status_for_code("RRS-C1007"), StatusCode::UNAUTHORIZED);
        assert_eq!(status_for_code("RRS-C1003"), StatusCode::FORBIDDEN);
        assert_eq!(status_for_code("RRS-C1004"), StatusCode::FORBIDDEN);
        assert_eq!(status_for_code("RRS-C0002"), StatusCode::BAD_REQUEST);
        assert_eq!(status_for_code("RRS-C0502"), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
