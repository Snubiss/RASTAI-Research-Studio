//! rastai-store: everything the daemon persists (layout: rust-control-plane.md section 6). M1-23 lands `rastai.db`
//! behind one writer thread, its read-only reader pool, migration 1 and the daemon lock. M1-24 lands the event logs:
//! the `RRSEVLG1` frame format, the reader and the writer (contracts 7.1). M1-25 lands the indexer, the blob store
//! and free space; `vault.db` (M3) joins it later. M1-27 adds `write_config_file`, the atomic write of `settings.json`,
//! and the journal's finished row with its redacted result.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod blobs;
pub mod config_files;
pub mod data_dir_lock;
pub mod disk_space;
pub mod error;
pub mod event_log;
pub mod index;
pub mod indexer;
pub mod reader_pool;
pub mod rebuild;
pub mod sqlite;
pub mod store;
pub mod writer_thread;
