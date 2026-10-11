//! One MCP session (layout: rust-control-plane.md 7.2): who it runs as, the revision `initialize` chose, and the
//! `initialize` and `notifications/initialized` handling. The daemon keeps the sessions (`Mcp-Session-Id`) and drops
//! one idle for 24 hours; this file only holds and updates one.

use rastai_core::crockford::encode_base32;
use rastai_core::error::CoreError;
use rastai_core::principal::Principal;
use rastai_core::random::random_bytes;
use rastai_core::time::now_unix_ms;
use rastai_core::versions::APP_VERSION;
use serde_json::{Value, json};

use crate::error::McpError;
use crate::jsonrpc::{INVALID_PARAMS, INVALID_REQUEST};
use crate::protocol_versions::pick_protocol_version;

/// 128 random bits: MCP asks for a session id nobody can guess, and it travels in a header (26 base32 characters).
const SESSION_ID_RANDOM_BYTES: usize = 16;
/// How the server names itself in `serverInfo`.
const SERVER_NAME: &str = "rastai";
const SERVER_TITLE: &str = "Rastai Research Studio";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSession {
    /// Sent back in the `Mcp-Session-Id` header.
    pub session_id: String,
    /// The MCP revision `initialize` chose; empty until then.
    pub protocol_version: String,
    /// Who the session runs as: the token behind the request, never the owner by default.
    pub principal: Principal,
    /// `clientInfo.name` from `initialize`, for logs; empty when the client gave none.
    pub client_name: String,
    /// True once the client sent `notifications/initialized`.
    pub initialized: bool,
    pub last_used_unix_ms: i64,
}

/// A new session for this principal, with a fresh random id.
pub fn new_session(principal: Principal) -> Result<McpSession, McpError> {
    let random = random_bytes(SESSION_ID_RANDOM_BYTES).map_err(random_unavailable)?;
    Ok(McpSession {
        session_id: encode_base32(&random),
        protocol_version: String::new(),
        principal,
        client_name: String::new(),
        initialized: false,
        last_used_unix_ms: now_unix_ms(),
    })
}

/// Answers `initialize`: picks the revision, records the client's name and returns the `InitializeResult`. A session
/// takes one `initialize`; a client that lost its session starts a new one (the bridge does this after a restart).
pub(crate) fn initialize_session(session: &mut McpSession, params: &Value) -> Result<Value, McpError> {
    if !session.protocol_version.is_empty() {
        return Err(refusal("initialize was already sent on this session", INVALID_REQUEST));
    }
    let Some(Value::String(requested)) = params.get("protocolVersion") else {
        return Err(refusal(
            "initialize needs \"protocolVersion\" as a string",
            INVALID_PARAMS,
        ));
    };
    let protocol_version = pick_protocol_version(requested);
    session.protocol_version = protocol_version.to_string();
    session.client_name = client_name(params);
    Ok(initialize_result(protocol_version))
}

/// `notifications/initialized`: the client is ready for normal operation.
pub(crate) fn mark_initialized(session: &mut McpSession) {
    session.initialized = true;
}

/// Refuses every request but `initialize` and `ping` before `initialize` (MCP lifecycle).
pub(crate) fn require_initialize(session: &McpSession) -> Result<(), McpError> {
    if session.protocol_version.is_empty() {
        return Err(refusal("send initialize first", INVALID_REQUEST));
    }
    Ok(())
}

fn initialize_result(protocol_version: &str) -> Value {
    json!({
        "protocolVersion": protocol_version,
        // Tools only. The list never changes inside a session: a token's scopes are fixed, and plugins that load
        // later show up in the next session.
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "title": SERVER_TITLE, "version": APP_VERSION },
    })
}

fn client_name(params: &Value) -> String {
    match params.get("clientInfo").and_then(|info| info.get("name")) {
        Some(Value::String(name)) => name.clone(),
        _ => String::new(),
    }
}

fn refusal(reason: &str, json_rpc_code: i64) -> McpError {
    McpError::MalformedMessage {
        reason: reason.to_string(),
        json_rpc_code,
        id: None,
    }
}

fn random_unavailable(error: CoreError) -> McpError {
    let reason = match error {
        CoreError::RandomUnavailable { reason } => reason,
        other => format!("{other:?}"),
    };
    McpError::RandomUnavailable { reason }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::principal::PrincipalKind;
    use rastai_core::scopes::token_preset_scopes;

    fn token_session() -> McpSession {
        let principal = Principal {
            kind: PrincipalKind::Token,
            id: "tok_01J".to_string(),
            scopes: token_preset_scopes("mcp-default").unwrap(),
        };
        new_session(principal).unwrap()
    }

    #[test]
    fn a_new_session_has_a_random_id_and_no_revision() {
        let first = token_session();
        let second = token_session();
        assert_eq!(first.session_id.len(), 26);
        assert_ne!(first.session_id, second.session_id);
        assert!(first.protocol_version.is_empty());
        assert!(!first.initialized);
        assert_eq!(first.principal.kind, PrincipalKind::Token);
    }

    #[test]
    fn initialize_records_the_revision_and_the_client() {
        let mut session = token_session();
        let params = json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "claude-desktop"}});
        let result = initialize_session(&mut session, &params).unwrap();
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(session.protocol_version, "2025-06-18");
        assert_eq!(session.client_name, "claude-desktop");
        let again = initialize_session(&mut session, &params).unwrap_err();
        assert_eq!(again.code(), "RRS-C3001");
    }

    #[test]
    fn initialize_without_a_revision_is_refused() {
        let mut session = token_session();
        let refused = initialize_session(&mut session, &json!({})).unwrap_err();
        assert!(matches!(
            refused,
            McpError::MalformedMessage {
                json_rpc_code: INVALID_PARAMS,
                ..
            }
        ));
        assert!(session.protocol_version.is_empty());
    }
}
