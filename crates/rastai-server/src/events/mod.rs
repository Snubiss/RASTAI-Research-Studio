//! The daemon's events (layout: rust-control-plane.md 8.3 and 13.1): the bus every UI stream reads, and the
//! daemon's own durable log with `record_daemon_event`.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod bus;
pub mod daemon_events;
