//! The command layer (layout: rust-control-plane.md 8.3, flows/platform.md 3.1 and 3.2): `execute_command`, where
//! every caller ends; the argument check, the journal and its redaction; `dispatch`, one arm per core command; and
//! the handlers built so far.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod arguments;
pub mod context;
pub mod dispatch;
pub mod execute;
pub mod handlers;
pub mod journal;
