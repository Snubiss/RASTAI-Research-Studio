//! `StoreError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (a command
//! handler, the daemon's startup, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists in the `RRS-S` range. SQLite's own failures are sorted by
//! `sqlite.rs` into these variants, so a full disk, a damaged file and an unwritable folder each get their own code.

use std::path::PathBuf;

const CODE_DAEMON_LOCK_HELD: &str = "RRS-S0001";
const CODE_SCHEMA_TOO_NEW: &str = "RRS-S0002";
const CODE_INDEX_DAMAGED: &str = "RRS-S0003";
const CODE_DATA_DIRECTORY_NOT_WRITABLE: &str = "RRS-S0004";
const CODE_DISK_FULL: &str = "RRS-S3002";
const CODE_WRITE_FAILED: &str = "RRS-S3005";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Another process holds `<runtime>/daemon.lock`. `holder_pid` is the number that process wrote into the file,
    /// when the file could be read (Windows keeps a locked file closed to other processes).
    DaemonLockHeld { path: PathBuf, holder_pid: Option<u32> },
    /// The database was migrated by a newer app: its `schema_migrations` has a version this build doesn't know.
    /// Nothing was written to the file.
    SchemaTooNew {
        path: PathBuf,
        found_version: i64,
        newest_known_version: i64,
    },
    /// `rastai.db` failed `quick_check`, isn't a database at all, or holds a row this crate can't read back.
    /// The fix is `rastai storage rebuild-index` (contracts 10.3). The catalog message names the file itself.
    IndexDamaged { reason: String },
    /// The data folder, or the database file in it, can't be created or opened for writing.
    DataDirectoryNotWritable { path: PathBuf, reason: String },
    /// SQLite reported a full disk during a write.
    DiskFull { reason: String },
    /// A database call failed for any other reason (a constraint, an I/O error, a lock held past the busy timeout).
    /// `reason` keeps SQLite's own words. Nearly every such failure is a write; a read that fails this way (on a
    /// healthy file only an I/O error or the busy timeout can do it) shares the code until the catalog has one for it.
    DatabaseCallFailed { reason: String },
    /// The writer thread has stopped (the store was closed, or the thread ended), so the write was never applied.
    WriterStopped,
}

impl StoreError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::DaemonLockHeld { .. } => CODE_DAEMON_LOCK_HELD,
            StoreError::SchemaTooNew { .. } => CODE_SCHEMA_TOO_NEW,
            StoreError::IndexDamaged { .. } => CODE_INDEX_DAMAGED,
            StoreError::DataDirectoryNotWritable { .. } => CODE_DATA_DIRECTORY_NOT_WRITABLE,
            StoreError::DiskFull { .. } => CODE_DISK_FULL,
            StoreError::DatabaseCallFailed { .. } => CODE_WRITE_FAILED,
            StoreError::WriterStopped => CODE_WRITE_FAILED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_catalog_code() {
        let path = PathBuf::from("rastai.db");
        let reason = "r".to_string();
        let lock = StoreError::DaemonLockHeld {
            path: path.clone(),
            holder_pid: None,
        };
        let schema = StoreError::SchemaTooNew {
            path: path.clone(),
            found_version: 2,
            newest_known_version: 1,
        };
        let damaged = StoreError::IndexDamaged { reason: reason.clone() };
        let not_writable = StoreError::DataDirectoryNotWritable {
            path,
            reason: reason.clone(),
        };
        assert_eq!(lock.code(), "RRS-S0001");
        assert_eq!(schema.code(), "RRS-S0002");
        assert_eq!(damaged.code(), "RRS-S0003");
        assert_eq!(not_writable.code(), "RRS-S0004");
        assert_eq!(StoreError::DiskFull { reason: reason.clone() }.code(), "RRS-S3002");
        assert_eq!(StoreError::DatabaseCallFailed { reason }.code(), "RRS-S3005");
        assert_eq!(StoreError::WriterStopped.code(), "RRS-S3005");
    }
}
