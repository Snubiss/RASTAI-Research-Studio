//! rastai-store: everything the daemon persists (layout: rust-control-plane.md section 6). M1-23 lands `rastai.db`
//! behind one writer thread, its read-only reader pool, migration 1 and the daemon lock. The event logs (M1-24), the
//! indexer, blob store and free space (M1-25) and `vault.db` (M3) join it later.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod data_dir_lock;
pub mod error;
pub mod index;
pub mod reader_pool;
pub mod sqlite;
pub mod store;
pub mod writer_thread;
