//! Safetensors (layout: rust-control-plane.md section 4.5). M1-34 landed the one header check that Rust and Python
//! share (contracts 2.4 rule 10). M2-13 adds the reader over a `ByteSource` (tensors read on demand) and the layout
//! planner for writing. The dtype table (`dtypes.rs`) is older and both use it.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod dtypes;
pub mod header_check;
pub mod reader;
pub mod writer;
