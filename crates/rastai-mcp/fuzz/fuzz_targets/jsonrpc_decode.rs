//! Fuzz target for `parse_message` (contracts 12.5): every MCP message comes from an outside client, so any bytes
//! in give a JSON-RPC message or a coded error out, never a panic:
//!
//! - a message the parser accepts writes back as one line that parses to the same message, and `handle_message`
//!   answers it without panicking, whatever its method and params;
//! - a refusal is always `RRS-C3001`, and its error reply is itself a message the parser accepts.
//!
//! Bytes that aren't UTF-8 are read lossily, so they still reach the parser (the bridge refuses them before it).
//! The first byte picks the session the message meets: a fresh one, so `initialize` runs, or one already
//! initialized, so `tools/list` and `tools/call` get past the lifecycle check. The rest is the message.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_core::principal::owner_principal;
use rastai_mcp::jsonrpc::{JsonRpcMessage, error_reply, message_json, parse_message};
use rastai_mcp::server::handle::{McpStep, handle_message};
use rastai_mcp::server::session::McpSession;
use rastai_mcp::server::tools::{McpTool, tool_from_command};
use rastai_proto::commands::v1::CommandDescriptor;

const MALFORMED_MESSAGE_CODE: &str = "RRS-C3001";
/// The revision of an already-initialized session.
const SESSION_PROTOCOL_VERSION: &str = "2025-06-18";

fuzz_target!(|bytes: &[u8]| {
    let Some((first_byte, message_bytes)) = bytes.split_first() else {
        return;
    };
    let already_initialized = first_byte % 2 == 1;
    let text = String::from_utf8_lossy(message_bytes);
    match parse_message(&text) {
        Ok(message) => check_accepted(message, already_initialized),
        Err(error) => {
            assert_eq!(error.code(), MALFORMED_MESSAGE_CODE);
            check_round_trip(&error_reply(None, &error));
        }
    }
});

fn check_accepted(message: JsonRpcMessage, already_initialized: bool) {
    check_round_trip(&message);
    let mut session = fuzz_session(already_initialized);
    let tools = [one_tool()];
    match handle_message(&mut session, message, &tools) {
        McpStep::Reply(reply) => check_round_trip(&reply),
        McpStep::ExecuteTool { arguments_json, .. } => check_arguments(&arguments_json),
        McpStep::NoReply => {}
    }
}

/// A fresh session (no revision yet) or one that already chose its revision. Built by hand rather than with
/// `new_session`, so a fuzz run doesn't read the random source on every input.
fn fuzz_session(already_initialized: bool) -> McpSession {
    let protocol_version = if already_initialized {
        SESSION_PROTOCOL_VERSION.to_string()
    } else {
        String::new()
    };
    McpSession {
        session_id: "FUZZ".to_string(),
        protocol_version,
        principal: owner_principal(),
        client_name: String::new(),
        initialized: already_initialized,
        last_used_unix_ms: 0,
    }
}

/// The message as one line, which parses back to the same message.
fn check_round_trip(message: &JsonRpcMessage) {
    let line = message_json(message);
    assert!(!line.contains('\n'), "a message must write as one line");
    let again = parse_message(&line).expect("a written message parses");
    assert_eq!(&again, message);
}

fn one_tool() -> McpTool {
    tool_from_command(&CommandDescriptor {
        command_id: "system.status".to_string(),
        scope: "read".to_string(),
        ..CommandDescriptor::default()
    })
}

/// Tool arguments are always handed on as a JSON object.
fn check_arguments(arguments_json: &str) {
    let arguments: serde_json::Value = serde_json::from_str(arguments_json).expect("arguments are JSON");
    assert!(arguments.is_object(), "arguments must be an object");
}
