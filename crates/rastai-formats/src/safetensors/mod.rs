//! Safetensors (layout: rust-control-plane.md section 4.5). M1-34 lands the one header check that Rust and Python
//! share (contracts 2.4 rule 10); the reader over a `ByteSource`, the writer's layout planner and `model_hash` join
//! with M2-13.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod dtypes;
pub mod header_check;
