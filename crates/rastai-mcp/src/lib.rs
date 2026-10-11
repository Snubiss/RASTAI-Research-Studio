//! rastai-mcp: the in-house MCP (layout: rust-control-plane.md section 7). M1-29 lands JSON-RPC 2.0 messages, the
//! server side as plain data (a message in, a step out; the daemon executes) and the stdio bridge behind `rastai mcp`.
//! The client, for harness tools in sandboxes, arrives with M7-E02.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod bridge;
pub mod error;
pub mod jsonrpc;
pub mod protocol_versions;
pub mod server;
