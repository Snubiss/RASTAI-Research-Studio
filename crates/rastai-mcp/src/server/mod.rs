//! The MCP server side (layout: rust-control-plane.md 7.2): sessions, `handle_message` and the tools built from the
//! command registry. It never executes a command: a tool call comes back as `McpStep::ExecuteTool`.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod handle;
pub mod session;
pub mod tools;
