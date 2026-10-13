//! The HTTP side of the daemon: the only folder that uses axum (contracts 2.5; layout section 8.5).
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod command_endpoints;
pub mod connect;
pub mod connect_json;
pub mod event_endpoint;
pub mod gate;
pub mod health;
pub mod listeners;
pub mod mcp_endpoint;
pub mod router;
pub mod sessions;
pub mod web_assets;
