//! The zip container (layout 4.5). The reader is in-house; DEFLATE is `miniz_oxide`, imported only in `inflate.rs`
//! (ADR-0037). The stored-only writer joins with M2-20.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod inflate;
pub mod reader;
