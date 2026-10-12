//! Command handlers, one file per area group (layout: rust-control-plane.md 8.1). Each takes `(&CommandContext, Args)`
//! and returns `Result<CommandOutput, Vec<Diagnostic>>`; none sees HTTP.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod command;
pub mod plugin;
pub mod settings;
pub mod system;
pub mod token;
