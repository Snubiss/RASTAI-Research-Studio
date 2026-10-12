//! Run-log indexing (layout: rust-control-plane.md 13.1, flows/platform.md 6.1): one tail per open run, and the
//! daemon log's replay at startup.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod daemon_log_replay;
pub mod run_tail;
