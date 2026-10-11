//! The HTTP side of the daemon: the only folder that uses axum (contracts 2.5; layout section 8.5).
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod gate;
pub mod health;
pub mod listeners;
pub mod router;
pub mod sessions;
