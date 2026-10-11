//! Commands (layout: rust-control-plane.md sections 5.1 and 10): the core table and its row type, the registry
//! that merges plugin commands into it, the naming rules of the generated surfaces, and the command output.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod descriptor;
pub mod naming;
pub mod output;
pub mod registry;
pub mod table;
