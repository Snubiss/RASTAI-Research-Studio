//! `ServerError`, the one error type of this crate (contracts 12.2). The startup functions return it; the edge
//! (`rastai serve` in rastai-cli) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists. Errors of the crates the daemon calls keep their own
//! variant and code, so a refused start says what really went wrong (a held lock is `RRS-S0001`, a socket folder
//! another user owns `RRS-N2005`). Request-time refusals are `Diagnostic`s straight away (http/gate.rs).
//!
//! `store_error_diagnostic` is the one place a `StoreError` the daemon reports at run time (a command's failure, a run
//! tail's problem) becomes a `Diagnostic`, with each catalog placeholder filled from the error's own fields.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rastai_core::error::CoreError;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_store::error::StoreError;
use rastai_transport::error::TransportError;

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_NO_FREE_PORT: &str = "RRS-C1001";
const CODE_OWNER_TOKEN_TOO_OPEN: &str = "RRS-C1008";
const CODE_OWNER_TOKEN_UNUSABLE: &str = "RRS-C1010";
const CODE_DATA_DIRECTORY_NOT_WRITABLE: &str = "RRS-S0004";
/// What `RRS-S0001` says when the lock file's holder couldn't be read.
const UNKNOWN_HOLDER_PID: &str = "unknown";
/// What `RRS-S3001` names as the operation when the store reports it (the caller knows no better name), and what
/// it says when the free space couldn't be read.
const STORE_WRITE_OPERATION: &str = "This write";
const UNKNOWN_NEEDED_BYTES: &str = "some";
const UNKNOWN_FREE_BYTES: &str = "an unknown number";
const WRITER_STOPPED_REASON: &str = "the index writer has stopped";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// `owner.token`'s mode isn't exactly 0600, such as one group or others can read or write. The daemon never
    /// rewrites it: a loose mode may mean someone already read or replaced the token (ADR-0034 decision 8). `mode`
    /// is the file's permission bits.
    OwnerTokenTooOpen { path: PathBuf, mode: u32 },
    /// `owner.token` or its folder can't be read or created: a permission, a folder in the file's place, a file
    /// that doesn't hold 64 lowercase hex characters, an I/O error. `reason` keeps the operating system's words.
    OwnerTokenUnusable { path: PathBuf, reason: String },
    /// Every port tried was already in use on the listen address.
    NoFreePort { tried_ports: Vec<u16> },
    /// Binding failed for a reason other than a port in use: `daemon.listen` names an address this machine doesn't
    /// have, or the port is one this user may not open. It is `RRS-C0002`, because the fix is to the daemon's own
    /// arguments (`daemon.listen`, `daemon.port`, `--port`); `reason` keeps the operating system's words.
    ListenAddressUnusable { address: SocketAddr, reason: String },
    /// A file in the runtime folder (`daemon.port`) couldn't be written.
    RuntimeFileNotWritable { path: PathBuf, reason: String },
    /// A `--set` flag that isn't `section.key=value`. `message` is the `RRS-C0002` diagnostic's, which names the
    /// flag and the form expected.
    InvalidSetFlag { message: String },
    /// No home folder, or no random source (rastai-core).
    Core(CoreError),
    /// The daemon lock, the runtime folder or rastai.db (rastai-store).
    Store(StoreError),
    /// daemon.sock couldn't be bound (rastai-transport).
    Transport(TransportError),
}

impl ServerError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            ServerError::OwnerTokenTooOpen { .. } => CODE_OWNER_TOKEN_TOO_OPEN,
            ServerError::OwnerTokenUnusable { .. } => CODE_OWNER_TOKEN_UNUSABLE,
            ServerError::NoFreePort { .. } => CODE_NO_FREE_PORT,
            ServerError::ListenAddressUnusable { .. } => CODE_INVALID_ARGUMENTS,
            ServerError::RuntimeFileNotWritable { .. } => CODE_DATA_DIRECTORY_NOT_WRITABLE,
            ServerError::InvalidSetFlag { .. } => CODE_INVALID_ARGUMENTS,
            ServerError::Core(error) => error.code(),
            ServerError::Store(error) => error.code(),
            ServerError::Transport(error) => error.code(),
        }
    }
}

/// A store error as the catalog diagnostic of its code, every placeholder of the code's message filled.
pub(crate) fn store_error_diagnostic(error: &StoreError) -> Diagnostic {
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

impl From<CoreError> for ServerError {
    fn from(error: CoreError) -> ServerError {
        ServerError::Core(error)
    }
}

impl From<StoreError> for ServerError {
    fn from(error: StoreError) -> ServerError {
        ServerError::Store(error)
    }
}

impl From<TransportError> for ServerError {
    fn from(error: TransportError) -> ServerError {
        ServerError::Transport(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_catalog_code() {
        let path = PathBuf::from("/home/user/.config/rastai/auth/owner.token");
        let reason = "r".to_string();
        let too_open = ServerError::OwnerTokenTooOpen {
            path: path.clone(),
            mode: 0o644,
        };
        let unusable = ServerError::OwnerTokenUnusable {
            path: path.clone(),
            reason: reason.clone(),
        };
        let no_port = ServerError::NoFreePort {
            tried_ports: vec![47820],
        };
        let not_writable = ServerError::RuntimeFileNotWritable { path, reason };
        let bad_flag = ServerError::InvalidSetFlag {
            message: "m".to_string(),
        };
        assert_eq!(too_open.code(), "RRS-C1008");
        assert_eq!(unusable.code(), "RRS-C1010");
        assert_eq!(no_port.code(), "RRS-C1001");
        assert_eq!(not_writable.code(), "RRS-S0004");
        assert_eq!(bad_flag.code(), "RRS-C0002");
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
    fn wrapped_errors_keep_their_own_code() {
        let store = ServerError::from(StoreError::WriterStopped);
        let core = ServerError::from(CoreError::HomeNotFound);
        let transport = ServerError::from(TransportError::TimedOut { timeout_ms: 1 });
        assert_eq!(store.code(), "RRS-S3005");
        assert_eq!(core.code(), "RRS-C0501");
        assert_eq!(transport.code(), "RRS-N2008");
    }
}
