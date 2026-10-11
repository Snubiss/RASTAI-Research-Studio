//! rastai-server: the daemon (layout: rust-control-plane.md section 8). M1-26 part 1 lands the startup sequence of
//! flows/platform.md 2.1, the HTTP listeners, the request gate of 4.2, one-time login codes and origin-bound
//! sessions (4.1), clean shutdown and `/api/health`.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod auth_files;
pub mod daemon;
pub mod error;
pub mod http;
pub mod server_state;
pub mod shutdown;
