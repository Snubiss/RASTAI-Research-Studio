//! The daemon's side of its nodes (layout: rust-control-plane.md 8.1; flows/platform.md 2.1–2.3): the links and
//! their handshake, the `NodeHubService` the nodes call, and the reconciliation after a connect. Pairing (M9) joins
//! with remote nodes.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod node_hub;
pub mod node_links;
pub mod reconcile;
