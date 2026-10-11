//! `McpError`, the one error type of this crate (contracts 12.2). The edge (the daemon's `/mcp` handler, the bridge)
//! turns it into a JSON-RPC error reply with `jsonrpc::error_reply`, which carries the catalog code and the
//! diagnostic `error_diagnostic` builds.
//!
//! Every variant maps to a code that contracts 10.3 lists. A message that isn't a JSON-RPC 2.0 message MCP allows is
//! `RRS-C3001`; a bridge started without a token is `RRS-C3002`; a session id that can't be made because the
//! operating system's random source failed is rastai-core's `RRS-C0502`.

use rastai_diag::diagnostic::{Diagnostic, diagnostic};

use crate::jsonrpc::JsonRpcId;

const CODE_MALFORMED_MESSAGE: &str = "RRS-C3001";
const CODE_NO_TOKEN: &str = "RRS-C3002";
const CODE_RANDOM_SOURCE_UNREADABLE: &str = "RRS-C0502";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// The text isn't a JSON-RPC 2.0 message MCP allows: not JSON, a batch, a missing or wrong member, or a request
    /// this session can't take yet. `reason` fills the catalog message. `json_rpc_code` is the JSON-RPC error code the
    /// reply carries (`-32700` parse error, `-32600` invalid request, `-32601` method not found, `-32602` invalid
    /// params). `id` is the request's id when it could be read, so the client can match the error to its request.
    MalformedMessage {
        reason: String,
        json_rpc_code: i64,
        id: Option<JsonRpcId>,
    },
    /// The bridge was started without a token. It never falls back to the owner (flows/platform.md 4.3).
    NoToken,
    /// A session id couldn't be made: the operating system's random source failed. `reason` keeps its words.
    RandomUnavailable { reason: String },
}

impl McpError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            McpError::MalformedMessage { .. } => CODE_MALFORMED_MESSAGE,
            McpError::NoToken => CODE_NO_TOKEN,
            McpError::RandomUnavailable { .. } => CODE_RANDOM_SOURCE_UNREADABLE,
        }
    }
}

/// The error as a catalog diagnostic, with its placeholders filled.
pub fn error_diagnostic(error: &McpError) -> Diagnostic {
    match error {
        McpError::MalformedMessage { reason, .. } => diagnostic(error.code(), &[("reason", reason)]),
        McpError::NoToken => diagnostic(error.code(), &[]),
        McpError::RandomUnavailable { reason } => diagnostic(error.code(), &[("reason", reason)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn malformed() -> McpError {
        McpError::MalformedMessage {
            reason: "batch arrays aren't supported".to_string(),
            json_rpc_code: -32600,
            id: None,
        }
    }

    #[test]
    fn every_error_has_its_catalog_code() {
        let random = McpError::RandomUnavailable {
            reason: "r".to_string(),
        };
        assert_eq!(malformed().code(), "RRS-C3001");
        assert_eq!(McpError::NoToken.code(), "RRS-C3002");
        assert_eq!(random.code(), "RRS-C0502");
    }

    #[test]
    fn the_diagnostic_fills_the_catalog_message() {
        let found = error_diagnostic(&malformed());
        assert_eq!(found.code, "RRS-C3001");
        assert_eq!(found.message, "Malformed MCP request: batch arrays aren't supported.");
        assert_eq!(
            error_diagnostic(&McpError::NoToken).message,
            "The MCP bridge has no token."
        );
    }
}
