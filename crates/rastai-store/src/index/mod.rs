//! `rastai.db`, the index: one file per group of tables (layout section 6.1), the write enum the writer thread
//! applies and the reader every read function takes. Callers pass row structs and filters, never SQL.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod artifacts;
pub mod documents;
pub mod journal;
pub mod kept_tables;
pub mod log_cursors;
pub mod reads;
pub mod runs;
pub mod writes;
