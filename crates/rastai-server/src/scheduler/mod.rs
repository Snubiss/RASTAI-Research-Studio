//! Job scheduling (layout: rust-control-plane.md 8.1; flows/platform.md 5.1): the queue, which node a job runs on,
//! and its launch. Chains (M5) and daemon tasks join with their milestones.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod launch;
pub mod placement;
pub mod queue;
