//! rastai-core: the shared vocabulary of the Rust processes that isn't a document (layout: rust-control-plane.md
//! section 5). M1-20 lands version constants, data paths, ids and their base32, randomness, time, principals and
//! scopes, the state-machine tables of the proto state enums, and the Diagnostic and CommandDescriptor wire
//! conversions. M1-21 adds the command table, registry, naming and settings.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod commands;
pub mod crockford;
pub mod error;
pub mod ids;
pub mod paths;
pub mod principal;
pub mod random;
pub mod scopes;
pub mod states;
pub mod time;
pub mod versions;
pub mod wire;
