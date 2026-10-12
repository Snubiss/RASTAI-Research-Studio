//! The plugin host (layout: rust-control-plane.md 12; flows/platform.md 7.2 and 7.3): discovery of installed,
//! preinstalled and dev plugins, capability negotiation over the table of contracts 5.5, registration of their
//! commands, and their states with `plugin.state_changed`. Plugins load through the host from day one (spec §31);
//! installing archives arrives with M8 and starting plugin modules with M4.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod capabilities;
pub mod contributions;
pub mod discovery;
pub mod lifecycle;
pub mod load;
