//! rastai-cli: the `rastai` binary (layout: rust-control-plane.md section 9). M1-31 lands the modes, the in-house
//! argument parser, the verbs generated from `ListCommands`, following jobs through Subscribe, `--json` output and
//! the exit codes. main.rs is the binary; this library holds everything it calls, so the tests in tests/ can reach
//! the parser and the argument builder directly.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod arguments;
pub mod command_cache;
pub mod daemon_client;
pub mod error;
pub mod exit_codes;
pub mod follow;
pub mod help;
pub mod local_commands;
pub mod mcp_bridge;
pub mod modes;
pub mod open_ui;
pub mod output;
pub mod serve;
pub mod verb_arguments;
pub mod verbs;
