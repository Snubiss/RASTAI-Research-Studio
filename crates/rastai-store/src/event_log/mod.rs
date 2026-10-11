//! Event logs (contracts 7.1, flows/platform.md 6): `runs/<run_id>/events.log` and the daemon's own log. One file
//! format, `RRSEVLG1` and then frames, read by `reader.rs` and written by `writer.rs`. Only `frame.rs` knows the
//! bytes of a frame.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod frame;
pub mod reader;
pub mod writer;
