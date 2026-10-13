//! `StoreError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (a command
//! handler, the daemon's startup, the node's tailer, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists in the `RRS-S` range, but `TokenCreatorRevoked`, the token
//! rule the writer's transaction enforces, which is the gate's `RRS-C1006`. SQLite's own failures are sorted by
//! `sqlite.rs` into these variants, so a full disk, a damaged file and an unwritable folder each get their own code.
//! `store_error_diagnostic` fills the catalog message's placeholders from the error's own fields.
//! A torn event-log tail is not an error: the reader reports it as `FrameRead::Incomplete`, and the writer truncates
//! it and says how many bytes it dropped (`truncated_tail_bytes`), which the caller logs as `RRS-S2001` (info).

use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::{Diagnostic, diagnostic};

const CODE_DAEMON_LOCK_HELD: &str = "RRS-S0001";
const CODE_SCHEMA_TOO_NEW: &str = "RRS-S0002";
const CODE_INDEX_DAMAGED: &str = "RRS-S0003";
const CODE_DATA_DIRECTORY_NOT_WRITABLE: &str = "RRS-S0004";
const CODE_READ_FAILED: &str = "RRS-S0006";
const CODE_RUNTIME_NOT_PRIVATE: &str = "RRS-S0007";
const CODE_BLOB_HASH_MISMATCH: &str = "RRS-S1001";
const CODE_BLOB_MISSING: &str = "RRS-S1002";
const CODE_EVENT_LOG_CORRUPT: &str = "RRS-S2002";
const CODE_EVENT_SEQUENCE_GAP: &str = "RRS-S2003";
const CODE_RUN_LOG_RECORD_REFUSED: &str = "RRS-S2004";
const CODE_NOT_ENOUGH_FREE_SPACE: &str = "RRS-S3001";
const CODE_DISK_FULL: &str = "RRS-S3002";
const CODE_WRITE_FAILED: &str = "RRS-S3005";
/// The gate's code for a revoked token: the one refusal here that isn't the store's own.
const CODE_TOKEN_REVOKED: &str = "RRS-C1006";
/// What `RRS-S0001` says when the lock file's holder couldn't be read.
const UNKNOWN_HOLDER_PID: &str = "unknown";
/// What `RRS-S3001` names as the operation when the store reports it (the caller knows no better name), and what
/// it says when the free space couldn't be read.
const STORE_WRITE_OPERATION: &str = "This write";
const UNKNOWN_NEEDED_BYTES: &str = "some";
const UNKNOWN_FREE_BYTES: &str = "an unknown number";
const WRITER_STOPPED_REASON: &str = "the index writer has stopped";

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
    /// The runtime folder, or the `daemon.lock` in it, could be reached by another user: the folder is a symbolic
    /// link, not a folder, another user's or open to group or others, or `daemon.lock` is a link or not a regular
    /// file. Nothing was written to it. `reason` says which.
    RuntimeNotPrivate { path: PathBuf, reason: String },
    /// SQLite reported a full disk during a write, or the disk filled up during an event-log write (the writer has
    /// already cut the partial frame off again).
    DiskFull { reason: String },
    /// A write failed for any other reason (a constraint, an I/O error, a lock held past the busy timeout).
    /// `reason` keeps SQLite's own words.
    DatabaseWriteFailed { reason: String },
    /// A read on a reader's connection failed on a file that isn't damaged: on a healthy file only an I/O error or
    /// the busy timeout can do it. `reason` keeps SQLite's own words.
    DatabaseReadFailed { reason: String },
    /// The writer thread has stopped (the store was closed, or the thread ended), so the write was never applied.
    WriterStopped,
    /// A token whose creator, token `token_id`, is revoked or gone wasn't inserted (contracts 8.2, threat model R28).
    /// Not a fault of the store but the rule that a revoked token's successors die with it, so its code is the
    /// gate's `RRS-C1006`, the one its holder gets.
    TokenCreatorRevoked { token_id: String },
    /// A bad event-log frame with more of the file after it, or a file that doesn't start with `RRSEVLG1` (offset 0).
    /// Bit rot or a foreign file, never a crash: a crash only tears the tail.
    EventLogCorrupt { path: PathBuf, offset: u64 },
    /// An event record's sequence isn't the previous one plus 1 (the first is 1). The reader stops before that record.
    EventSequenceGap {
        path: PathBuf,
        expected_sequence: u64,
        found_sequence: u64,
    },
    /// A record in run `run_id`'s log that a run's log may not carry (contracts 7.1): it names another run, its
    /// payload is an event only the daemon's log carries, or it would take over another run's checkpoint or rewrite
    /// an existing artifact. The job writes its run's log, so the log is untrusted: the record and the rest of its
    /// batch are not applied and the ack stays before it. The log counts as damaged at `sequence`, as at a corrupt
    /// frame. `reason` says which rule it broke.
    RunLogRecordRefused {
        run_id: String,
        sequence: u64,
        reason: String,
    },
    /// Opening or reading an event log failed: a missing file, a permission, an I/O error. `reason` keeps the
    /// operating system's words. The store's read code, `RRS-S0006`, like `DatabaseReadFailed`: nothing was written,
    /// so the user is never told a write failed.
    EventLogReadFailed { path: PathBuf, reason: String },
    /// Creating, writing, truncating or syncing an event log failed for a reason other than a full disk; `reason`
    /// keeps the operating system's words.
    EventLogWriteFailed { path: PathBuf, reason: String },
    /// A blob's bytes don't hash to its name, so it was moved out of the store to `quarantine_path`
    /// (`<data>/blobs/.quarantine/`). Bit rot or a foreign write; the store never serves it again.
    BlobHashMismatch { hash: String, quarantine_path: PathBuf },
    /// No blob with this hash is in the store. Also a hash that isn't `blake3:<64 lowercase hex>`, since no blob
    /// can have that name.
    BlobMissing { hash: String },
    /// Reading a blob, or the file `put_file` stores, failed for a reason other than its absence. `reason` keeps the
    /// operating system's words. The read code, `RRS-S0006`, like a failed database or event-log read.
    BlobReadFailed { path: PathBuf, reason: String },
    /// Writing, linking, renaming or syncing a blob file failed for a reason other than a full disk.
    BlobWriteFailed { path: PathBuf, reason: String },
    /// Writing a config file (`settings.json`, keybindings, a layout) or reading the one it replaces failed for a
    /// reason other than a full disk. The file is left as it was (`write_config_file`).
    ConfigFileWriteFailed { path: PathBuf, reason: String },
    /// The volume that holds `path` has `free_bytes` free, less than `needed_bytes` plus `margin_bytes`
    /// (`storage.free_space_margin_bytes`, handed in by the caller).
    NotEnoughFreeSpace {
        path: PathBuf,
        needed_bytes: u64,
        margin_bytes: u64,
        free_bytes: u64,
    },
    /// The free space of the volume that holds `path` couldn't be read: `statvfs` failed (a missing path, a
    /// permission), or this platform has no free-space call yet. Unknown counts as not enough (ADR-0033 decision 4),
    /// so this is `RRS-S3001` too and a check never passes without a number.
    FreeSpaceUnknown { path: PathBuf, reason: String },
}

impl StoreError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::DaemonLockHeld { .. } => CODE_DAEMON_LOCK_HELD,
            StoreError::SchemaTooNew { .. } => CODE_SCHEMA_TOO_NEW,
            StoreError::IndexDamaged { .. } => CODE_INDEX_DAMAGED,
            StoreError::DataDirectoryNotWritable { .. } => CODE_DATA_DIRECTORY_NOT_WRITABLE,
            StoreError::RuntimeNotPrivate { .. } => CODE_RUNTIME_NOT_PRIVATE,
            StoreError::DiskFull { .. } => CODE_DISK_FULL,
            StoreError::DatabaseWriteFailed { .. } => CODE_WRITE_FAILED,
            StoreError::DatabaseReadFailed { .. } => CODE_READ_FAILED,
            StoreError::WriterStopped => CODE_WRITE_FAILED,
            StoreError::TokenCreatorRevoked { .. } => CODE_TOKEN_REVOKED,
            StoreError::EventLogCorrupt { .. } => CODE_EVENT_LOG_CORRUPT,
            StoreError::EventSequenceGap { .. } => CODE_EVENT_SEQUENCE_GAP,
            StoreError::RunLogRecordRefused { .. } => CODE_RUN_LOG_RECORD_REFUSED,
            StoreError::EventLogReadFailed { .. } => CODE_READ_FAILED,
            StoreError::EventLogWriteFailed { .. } => CODE_WRITE_FAILED,
            StoreError::BlobHashMismatch { .. } => CODE_BLOB_HASH_MISMATCH,
            StoreError::BlobMissing { .. } => CODE_BLOB_MISSING,
            StoreError::BlobReadFailed { .. } => CODE_READ_FAILED,
            StoreError::BlobWriteFailed { .. } => CODE_WRITE_FAILED,
            StoreError::ConfigFileWriteFailed { .. } => CODE_WRITE_FAILED,
            StoreError::NotEnoughFreeSpace { .. } => CODE_NOT_ENOUGH_FREE_SPACE,
            StoreError::FreeSpaceUnknown { .. } => CODE_NOT_ENOUGH_FREE_SPACE,
        }
    }
}

/// A store error as the catalog diagnostic of its code, every placeholder of the code's message filled: the one place
/// a `StoreError` becomes a `Diagnostic`, for the daemon's reports (rastai-server) and the rebuild's (`rebuild.rs`).
pub fn store_error_diagnostic(error: &StoreError) -> Diagnostic {
    let values = store_error_values(error);
    let mut pairs = Vec::new();
    for (name, value) in &values {
        pairs.push((*name, value.as_str()));
    }
    diagnostic(error.code(), &pairs)
}

/// The placeholder values of each variant's catalog message (catalog/errors/RRS-S.json).
fn store_error_values(error: &StoreError) -> Vec<(&'static str, String)> {
    match error {
        StoreError::DaemonLockHeld { path, holder_pid } => {
            let pid = match holder_pid {
                Some(pid) => pid.to_string(),
                None => UNKNOWN_HOLDER_PID.to_string(),
            };
            vec![("path", path_text(path)), ("pid", pid)]
        }
        StoreError::SchemaTooNew {
            path, found_version, ..
        } => {
            vec![("path", path_text(path)), ("version", found_version.to_string())]
        }
        StoreError::DataDirectoryNotWritable { path, .. } => vec![("path", path_text(path))],
        StoreError::RuntimeNotPrivate { path, reason } => vec![("path", path_text(path)), ("reason", reason.clone())],
        StoreError::EventLogCorrupt { path, offset } => vec![("path", path_text(path)), ("offset", offset.to_string())],
        StoreError::EventSequenceGap {
            path,
            expected_sequence,
            found_sequence,
        } => vec![
            ("path", path_text(path)),
            ("expected_sequence", expected_sequence.to_string()),
            ("found_sequence", found_sequence.to_string()),
        ],
        StoreError::RunLogRecordRefused {
            run_id,
            sequence,
            reason,
        } => vec![
            ("run_id", run_id.clone()),
            ("sequence", sequence.to_string()),
            ("reason", reason.clone()),
        ],
        StoreError::BlobHashMismatch { hash, .. } | StoreError::BlobMissing { hash } => {
            vec![("content_hash", hash.clone())]
        }
        // `RRS-C1006` says only that the token was revoked: which creator it was is the owner's to see in token.list.
        StoreError::TokenCreatorRevoked { .. } => Vec::new(),
        _ => store_error_reason_values(error),
    }
}

/// The variants whose message is `{reason}` alone (`RRS-S0003`, `RRS-S0006`, `RRS-S3002`, `RRS-S3005`), and the
/// free-space ones (`RRS-S3001`).
fn store_error_reason_values(error: &StoreError) -> Vec<(&'static str, String)> {
    let reason = match error {
        StoreError::NotEnoughFreeSpace {
            path,
            needed_bytes,
            margin_bytes,
            free_bytes,
        } => {
            let needed = needed_bytes.saturating_add(*margin_bytes).to_string();
            return free_space_values(path, needed, free_bytes.to_string());
        }
        StoreError::FreeSpaceUnknown { path, reason } => {
            let free = format!("{UNKNOWN_FREE_BYTES} ({reason})");
            return free_space_values(path, UNKNOWN_NEEDED_BYTES.to_string(), free);
        }
        StoreError::IndexDamaged { reason }
        | StoreError::DiskFull { reason }
        | StoreError::DatabaseWriteFailed { reason }
        | StoreError::DatabaseReadFailed { reason } => reason.clone(),
        StoreError::EventLogReadFailed { path, reason }
        | StoreError::EventLogWriteFailed { path, reason }
        | StoreError::BlobReadFailed { path, reason }
        | StoreError::BlobWriteFailed { path, reason }
        | StoreError::ConfigFileWriteFailed { path, reason } => format!("{}: {reason}", path.display()),
        StoreError::WriterStopped => WRITER_STOPPED_REASON.to_string(),
        other => format!("{other:?}"),
    };
    vec![("reason", reason)]
}

fn free_space_values(path: &Path, needed_bytes: String, free_bytes: String) -> Vec<(&'static str, String)> {
    vec![
        ("operation", STORE_WRITE_OPERATION.to_string()),
        ("needed_bytes", needed_bytes),
        ("volume", path_text(path)),
        ("free_bytes", free_bytes),
    ]
}

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

/// An I/O error while opening or reading an event log.
pub(crate) fn event_log_read_error(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    StoreError::EventLogReadFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// Sorts an I/O error while writing an event log into its variant: a full disk gets its own code, everything else
/// keeps the operating system's reason.
pub(crate) fn event_log_write_error(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    if error.kind() == std::io::ErrorKind::StorageFull {
        return StoreError::DiskFull {
            reason: format!("{}: {error}", path.display()),
        };
    }
    StoreError::EventLogWriteFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// An I/O error while reading a blob or a file being stored. A missing blob is `BlobMissing`, which the caller
/// decides, since only it knows the hash.
pub(crate) fn blob_read_error(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    StoreError::BlobReadFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// Sorts an I/O error while writing a blob file: a full disk gets its own code, everything else keeps the operating
/// system's reason.
pub(crate) fn blob_write_error(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    if error.kind() == std::io::ErrorKind::StorageFull {
        return StoreError::DiskFull {
            reason: format!("{}: {error}", path.display()),
        };
    }
    StoreError::BlobWriteFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// Sorts an I/O error while writing a config file: a full disk gets its own code, everything else keeps the operating
/// system's reason.
pub(crate) fn config_file_write_error(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    if error.kind() == std::io::ErrorKind::StorageFull {
        return StoreError::DiskFull {
            reason: format!("{}: {error}", path.display()),
        };
    }
    StoreError::ConfigFileWriteFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
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
            path: path.clone(),
            reason: reason.clone(),
        };
        let not_private = StoreError::RuntimeNotPrivate {
            path,
            reason: reason.clone(),
        };
        assert_eq!(lock.code(), "RRS-S0001");
        assert_eq!(schema.code(), "RRS-S0002");
        assert_eq!(damaged.code(), "RRS-S0003");
        assert_eq!(not_writable.code(), "RRS-S0004");
        assert_eq!(not_private.code(), "RRS-S0007");
        assert_eq!(StoreError::DiskFull { reason: reason.clone() }.code(), "RRS-S3002");
        let write_failed = StoreError::DatabaseWriteFailed { reason: reason.clone() };
        let read_failed = StoreError::DatabaseReadFailed { reason };
        assert_eq!(write_failed.code(), "RRS-S3005");
        assert_eq!(read_failed.code(), "RRS-S0006");
        assert_eq!(StoreError::WriterStopped.code(), "RRS-S3005");
        let creator_revoked = StoreError::TokenCreatorRevoked {
            token_id: "tok_a".to_string(),
        };
        assert_eq!(creator_revoked.code(), "RRS-C1006");
    }

    #[test]
    fn every_event_log_error_has_its_catalog_code() {
        let path = PathBuf::from("events.log");
        let corrupt = StoreError::EventLogCorrupt {
            path: path.clone(),
            offset: 8,
        };
        let gap = StoreError::EventSequenceGap {
            path: path.clone(),
            expected_sequence: 3,
            found_sequence: 4,
        };
        let refused = StoreError::RunLogRecordRefused {
            run_id: "run_a".to_string(),
            sequence: 2,
            reason: "r".to_string(),
        };
        let read_failed = StoreError::EventLogReadFailed {
            path: path.clone(),
            reason: "r".to_string(),
        };
        let write_failed = StoreError::EventLogWriteFailed {
            path,
            reason: "r".to_string(),
        };
        assert_eq!(corrupt.code(), "RRS-S2002");
        assert_eq!(gap.code(), "RRS-S2003");
        assert_eq!(refused.code(), "RRS-S2004");
        assert_eq!(read_failed.code(), "RRS-S0006");
        assert_eq!(write_failed.code(), "RRS-S3005");
    }

    #[test]
    fn event_log_io_errors_are_sorted_by_read_write_and_full_disk() {
        let path = std::path::Path::new("events.log");
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(event_log_write_error(path, &full).code(), "RRS-S3002");
        assert_eq!(event_log_write_error(path, &denied).code(), "RRS-S3005");
        assert_eq!(event_log_read_error(path, &denied).code(), "RRS-S0006");
    }

    #[test]
    fn every_blob_and_free_space_error_has_its_catalog_code() {
        let path = PathBuf::from("blobs");
        let hash = "blake3:00".to_string();
        let mismatch = StoreError::BlobHashMismatch {
            hash: hash.clone(),
            quarantine_path: path.clone(),
        };
        let not_enough = StoreError::NotEnoughFreeSpace {
            path: path.clone(),
            needed_bytes: 2,
            margin_bytes: 1,
            free_bytes: 1,
        };
        let unknown = StoreError::FreeSpaceUnknown {
            path: path.clone(),
            reason: "r".to_string(),
        };
        assert_eq!(mismatch.code(), "RRS-S1001");
        assert_eq!(StoreError::BlobMissing { hash }.code(), "RRS-S1002");
        assert_eq!(not_enough.code(), "RRS-S3001");
        assert_eq!(unknown.code(), "RRS-S3001");
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(blob_write_error(&path, &full).code(), "RRS-S3002");
        assert_eq!(blob_write_error(&path, &denied).code(), "RRS-S3005");
        assert_eq!(blob_read_error(&path, &denied).code(), "RRS-S0006");
    }

    #[test]
    fn a_store_diagnostic_fills_every_placeholder_of_its_message() {
        let path = PathBuf::from("/data/runs/run_a/events.log");
        let errors = [
            StoreError::EventSequenceGap {
                path: path.clone(),
                expected_sequence: 4,
                found_sequence: 6,
            },
            StoreError::EventLogCorrupt {
                path: path.clone(),
                offset: 120,
            },
            StoreError::EventLogWriteFailed {
                path: path.clone(),
                reason: "permission denied".to_string(),
            },
            StoreError::DaemonLockHeld {
                path: path.clone(),
                holder_pid: None,
            },
            StoreError::FreeSpaceUnknown {
                path: path.clone(),
                reason: "statvfs failed".to_string(),
            },
            StoreError::WriterStopped,
        ];
        for error in &errors {
            let found = store_error_diagnostic(error);
            assert_eq!(found.code, error.code());
            assert!(!found.message.contains('{'), "{}", found.message);
        }
        let gap = store_error_diagnostic(&errors[0]);
        assert_eq!(
            gap.message,
            "Event sequence gap in /data/runs/run_a/events.log: expected 4, found 6."
        );
    }

    #[test]
    fn config_file_errors_are_sorted_by_full_disk() {
        let path = std::path::Path::new("settings.json");
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(config_file_write_error(path, &full).code(), "RRS-S3002");
        assert_eq!(config_file_write_error(path, &denied).code(), "RRS-S3005");
    }
}
