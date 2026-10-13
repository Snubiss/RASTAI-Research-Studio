//! Event logs (contracts 7.1, flows/platform.md 6): `runs/<run_id>/events.log` and the daemon's own log. One file
//! format, `RRSEVLG1` and then frames, read by `reader.rs` and written by `writer.rs`. Only `frame.rs` knows the
//! bytes of a frame, and only `daemon_log.rs` how the daemon's log is split into segments and named.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod daemon_log;
pub mod frame;
pub mod reader;
pub mod writer;
