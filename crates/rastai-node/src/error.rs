//! `NodeError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (the tonic
//! service in node_service.rs, the CLI's `node serve`, the daemon's start of the embedded node) turns it into a
//! `Diagnostic` through `code()` and `node_error_diagnostic`.
//!
//! Every variant maps to a code that contracts 10.3 lists and node layout 11.1 gives this crate. The `reason` fields
//! keep the operating system's or the library's own words for the log.

use std::path::PathBuf;

use rastai_core::error::CoreError;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_store::error::StoreError;
use rastai_store::error::store_error_diagnostic;
use rastai_transport::error::TransportError;

const CODE_PROTOCOL_VERSION_UNSUPPORTED: &str = "RRS-N0001";
const CODE_DAEMON_NOT_PAIRED: &str = "RRS-N0002";
const CODE_NODE_ALREADY_RUNNING: &str = "RRS-N0003";
const CODE_IDENTITY_FILE_UNUSABLE: &str = "RRS-N0006";
const CODE_HANDSHAKE_STALE: &str = "RRS-N0007";
const CODE_PATH_OUTSIDE_ALLOWED: &str = "RRS-N3001";
const CODE_DOWNLOAD_HASH_MISMATCH: &str = "RRS-N3003";
const CODE_MODEL_NOT_FOUND: &str = "RRS-N3004";
const CODE_MODEL_GATED: &str = "RRS-N3005";
const CODE_MODEL_OFFLINE: &str = "RRS-N3006";
const CODE_DOWNLOAD_INTERRUPTED: &str = "RRS-N3009";
const CODE_DOWNLOAD_LOCK_TIMEOUT: &str = "RRS-N3011";
const CODE_REDIRECT_REFUSED: &str = "RRS-N3013";
/// What N0003's message shows when the running node's process id can't be read.
const UNKNOWN_PROCESS_ID: &str = "unknown";
/// What N0002's message shows for a signature checked without a node to name (see `SignatureRefused`).
const UNKNOWN_NODE_ID: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeError {
    /// The daemon speaks another node protocol version. `node_version` is ours, `daemon_version` the request's.
    ProtocolVersionUnsupported {
        node_id: String,
        node_version: u32,
        daemon_version: u32,
    },
    /// The daemon's key isn't pinned in paired_daemons.json, its id isn't a key, or its signature doesn't verify
    /// with that key: in each case the caller hasn't proved it holds a paired daemon key.
    DaemonNotPaired { node_id: String, reason: String },
    /// A signature didn't verify, or a public key or signature had the wrong form (identity.rs). identity.rs doesn't
    /// know whose key it checked, so a caller that does names it: the handshake turns this into `DaemonNotPaired`
    /// with the node's id. Reaching the edge unmapped, its message names node "unknown".
    SignatureRefused { reason: String },
    /// The daemon's clock is more than 5 minutes off ours, or its nonce was used before (designer flow C3a).
    HandshakeStale { daemon_id: String, reason: String },
    /// Another node already answers on node.sock (one node agent per user per machine, designer flow C1).
    NodeAlreadyRunning { process_id: Option<i32> },
    /// node.key (or daemon.key) or paired_daemons.json can't be read, created or written, is damaged, or is open to
    /// group or others.
    IdentityFileUnusable { path: PathBuf, reason: String },
    /// An error from rastai-core, such as an unreadable random source (RRS-C0502).
    Core(CoreError),
    /// An error from rastai-transport, such as a socket folder another user owns (RRS-N2005).
    Transport(TransportError),
    /// An error from rastai-store: a download that needs more disk than the cache's volume has (RRS-S3001), or a
    /// cache folder that can't be written.
    Store(StoreError),
    /// The Hub has no such repository, or the repository has no such file (RRS-N3004). `file_name` is empty when the
    /// repository itself is missing.
    ModelNotFound { repository: String, file_name: String },
    /// The repository is gated and this account has no access, or its license isn't accepted (RRS-N3005).
    ModelGated { repository: String },
    /// Offline mode is on, so nothing was fetched (RRS-N3006).
    ModelOffline { repository: String },
    /// A file's connection kept failing until its retries ran out. What was downloaded is kept (RRS-N3009). `reason`
    /// is the last failure's words, for the log.
    DownloadInterrupted { file_name: String, reason: String },
    /// Another program held a file's download lock for the whole wait (RRS-N3011).
    DownloadLockTimeout { file_name: String },
    /// A downloaded file's SHA-256 isn't the one the Hub lists. The partial file was removed (RRS-N3003).
    DownloadHashMismatch { path: PathBuf },
    /// A redirect a file answered with can't be followed: it has no Location, or the chain is longer than the limit
    /// (RRS-N3013). The Hub gives the same answer on a retry, so this is never retried. `reason` is the words for the log.
    RedirectRefused { file_name: String, reason: String },
    /// The Hub listed a file name or revision that would leave the cache folder (RRS-N3001).
    UnsafeHubPath { path: String },
}

impl NodeError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            NodeError::ProtocolVersionUnsupported { .. } => CODE_PROTOCOL_VERSION_UNSUPPORTED,
            NodeError::DaemonNotPaired { .. } => CODE_DAEMON_NOT_PAIRED,
            NodeError::SignatureRefused { .. } => CODE_DAEMON_NOT_PAIRED,
            NodeError::HandshakeStale { .. } => CODE_HANDSHAKE_STALE,
            NodeError::NodeAlreadyRunning { .. } => CODE_NODE_ALREADY_RUNNING,
            NodeError::IdentityFileUnusable { .. } => CODE_IDENTITY_FILE_UNUSABLE,
            NodeError::Core(error) => error.code(),
            NodeError::Transport(error) => error.code(),
            NodeError::Store(error) => error.code(),
            NodeError::ModelNotFound { .. } => CODE_MODEL_NOT_FOUND,
            NodeError::ModelGated { .. } => CODE_MODEL_GATED,
            NodeError::ModelOffline { .. } => CODE_MODEL_OFFLINE,
            NodeError::DownloadInterrupted { .. } => CODE_DOWNLOAD_INTERRUPTED,
            NodeError::DownloadLockTimeout { .. } => CODE_DOWNLOAD_LOCK_TIMEOUT,
            NodeError::RedirectRefused { .. } => CODE_REDIRECT_REFUSED,
            NodeError::DownloadHashMismatch { .. } => CODE_DOWNLOAD_HASH_MISMATCH,
            NodeError::UnsafeHubPath { .. } => CODE_PATH_OUTSIDE_ALLOWED,
        }
    }
}

impl From<CoreError> for NodeError {
    fn from(error: CoreError) -> NodeError {
        NodeError::Core(error)
    }
}

impl From<TransportError> for NodeError {
    fn from(error: TransportError) -> NodeError {
        NodeError::Transport(error)
    }
}

/// The diagnostic for this crate's own errors, with every placeholder of its catalog entry filled. Errors of
/// rastai-core and rastai-transport get their code and their reason in the message slot their entry has, or the
/// bare code when the entry has no such slot.
pub fn node_error_diagnostic(error: &NodeError) -> Diagnostic {
    match error {
        NodeError::ProtocolVersionUnsupported {
            node_id,
            node_version,
            daemon_version,
        } => {
            let node_version = node_version.to_string();
            let daemon_version = daemon_version.to_string();
            // The message is written from the daemon's side: "Node X speaks version N; this daemon supports M".
            diagnostic(
                CODE_PROTOCOL_VERSION_UNSUPPORTED,
                &[
                    ("node_id", node_id),
                    ("version", &node_version),
                    ("supported_versions", &daemon_version),
                ],
            )
        }
        NodeError::DaemonNotPaired { node_id, .. } => diagnostic(CODE_DAEMON_NOT_PAIRED, &[("node_id", node_id)]),
        NodeError::SignatureRefused { .. } => diagnostic(CODE_DAEMON_NOT_PAIRED, &[("node_id", UNKNOWN_NODE_ID)]),
        NodeError::HandshakeStale { daemon_id, reason } => {
            diagnostic(CODE_HANDSHAKE_STALE, &[("daemon_id", daemon_id), ("reason", reason)])
        }
        NodeError::NodeAlreadyRunning { process_id } => node_already_running_diagnostic(*process_id),
        NodeError::IdentityFileUnusable { path, reason } => {
            let path = path.display().to_string();
            diagnostic(CODE_IDENTITY_FILE_UNUSABLE, &[("path", &path), ("reason", reason)])
        }
        NodeError::Core(core_error) => core_error_diagnostic(core_error),
        NodeError::Transport(transport_error) => transport_error_diagnostic(transport_error),
        NodeError::Store(store_error) => store_error_diagnostic(store_error),
        _ => download_error_diagnostic(error),
    }
}

/// The Hugging Face download errors. Each message names the repository or file it is about, as its catalog entry does.
fn download_error_diagnostic(error: &NodeError) -> Diagnostic {
    match error {
        NodeError::ModelNotFound { repository, file_name } => diagnostic(
            CODE_MODEL_NOT_FOUND,
            &[("repository", repository), ("file_name", file_name)],
        ),
        NodeError::ModelGated { repository } => diagnostic(CODE_MODEL_GATED, &[("repository", repository)]),
        NodeError::ModelOffline { repository } => diagnostic(CODE_MODEL_OFFLINE, &[("repository", repository)]),
        NodeError::DownloadInterrupted { file_name, .. } => {
            diagnostic(CODE_DOWNLOAD_INTERRUPTED, &[("file_name", file_name)])
        }
        NodeError::DownloadLockTimeout { file_name } => {
            diagnostic(CODE_DOWNLOAD_LOCK_TIMEOUT, &[("file_name", file_name)])
        }
        NodeError::DownloadHashMismatch { path } => {
            let path = path.display().to_string();
            diagnostic(CODE_DOWNLOAD_HASH_MISMATCH, &[("path", &path)])
        }
        NodeError::RedirectRefused { file_name, reason } => {
            diagnostic(CODE_REDIRECT_REFUSED, &[("file_name", file_name), ("reason", reason)])
        }
        NodeError::UnsafeHubPath { path } => diagnostic(CODE_PATH_OUTSIDE_ALLOWED, &[("path", path)]),
        // Only the variants above reach here; the other arms of node_error_diagnostic answer the rest.
        _ => diagnostic(error.code(), &[]),
    }
}

impl From<StoreError> for NodeError {
    fn from(error: StoreError) -> NodeError {
        NodeError::Store(error)
    }
}

fn node_already_running_diagnostic(process_id: Option<i32>) -> Diagnostic {
    let process_id = match process_id {
        Some(process_id) => process_id.to_string(),
        None => UNKNOWN_PROCESS_ID.to_string(),
    };
    diagnostic(CODE_NODE_ALREADY_RUNNING, &[("pid", &process_id)])
}

/// rastai-core's errors that reach this crate's edge: the random source (the session token, a new node.key) and
/// the home folder. A bad timestamp never reaches it.
fn core_error_diagnostic(error: &CoreError) -> Diagnostic {
    match error {
        CoreError::RandomUnavailable { reason } => diagnostic("RRS-C0502", &[("reason", reason)]),
        CoreError::HomeNotFound => diagnostic("RRS-C0501", &[]),
        CoreError::InvalidTimestamp { text, reason } => {
            diagnostic("RRS-E1004", &[("found", text), ("expected", reason)])
        }
    }
}

/// rastai-transport's errors, from binding and serving node.sock. Variants that carry no path name none.
fn transport_error_diagnostic(error: &TransportError) -> Diagnostic {
    match error {
        TransportError::SocketNotReachable { socket_path, .. } => {
            let path = socket_path.display().to_string();
            diagnostic("RRS-N2004", &[("path", &path)])
        }
        TransportError::SocketFolderInsecure { folder, .. } => {
            let path = folder.display().to_string();
            diagnostic("RRS-N2005", &[("path", &path)])
        }
        TransportError::PeerIsAnotherUser { .. } => diagnostic("RRS-N2006", &[("path", "")]),
        TransportError::SocketNotCreated { socket_path, reason } => {
            let path = socket_path.display().to_string();
            diagnostic("RRS-N2007", &[("path", &path), ("reason", reason)])
        }
        TransportError::ConnectionFailed { reason } => diagnostic("RRS-N2008", &[("path", ""), ("reason", reason)]),
        TransportError::TimedOut { timeout_ms } => {
            let reason = format!("no answer within {timeout_ms} ms");
            diagnostic("RRS-N2008", &[("path", ""), ("reason", &reason)])
        }
        TransportError::NotSupportedOnThisPlatform { .. } => {
            diagnostic("RRS-N2009", &[("platform", std::env::consts::OS)])
        }
        TransportError::EgressTagMissing { host } => diagnostic("RRS-N3008", &[("host", host.as_str())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_catalog_code() {
        let reason = "r".to_string();
        let version = NodeError::ProtocolVersionUnsupported {
            node_id: "node_x".to_string(),
            node_version: 1,
            daemon_version: 2,
        };
        let not_paired = NodeError::DaemonNotPaired {
            node_id: "node_x".to_string(),
            reason: reason.clone(),
        };
        let stale = NodeError::HandshakeStale {
            daemon_id: "daemon_x".to_string(),
            reason: reason.clone(),
        };
        let identity = NodeError::IdentityFileUnusable {
            path: PathBuf::from("/home/user/.local/share/rastai/identity/node.key"),
            reason: reason.clone(),
        };
        assert_eq!(version.code(), "RRS-N0001");
        assert_eq!(not_paired.code(), "RRS-N0002");
        assert_eq!(NodeError::SignatureRefused { reason }.code(), "RRS-N0002");
        assert_eq!(NodeError::NodeAlreadyRunning { process_id: None }.code(), "RRS-N0003");
        assert_eq!(identity.code(), "RRS-N0006");
        assert_eq!(stale.code(), "RRS-N0007");
        let refused = NodeError::RedirectRefused {
            file_name: "config.json".to_string(),
            reason: "no location".to_string(),
        };
        assert_eq!(refused.code(), "RRS-N3013");
    }

    // The version refusal is read by the daemon, so it names the node's version and the daemon's.
    #[test]
    fn the_version_refusal_names_both_versions() {
        let error = NodeError::ProtocolVersionUnsupported {
            node_id: "node_x".to_string(),
            node_version: 1,
            daemon_version: 7,
        };
        let result = node_error_diagnostic(&error);
        assert_eq!(result.code, "RRS-N0001");
        assert!(result.message.contains("node_x"), "{}", result.message);
        assert!(result.message.contains("version 1"), "{}", result.message);
        assert!(result.message.contains('7'), "{}", result.message);
    }

    // A signature refusal no caller named a node for still reads as a sentence, never "with node .".
    #[test]
    fn a_signature_refusal_without_a_node_says_unknown() {
        let error = NodeError::SignatureRefused {
            reason: "the signature doesn't verify".to_string(),
        };
        let result = node_error_diagnostic(&error);
        assert_eq!(result.code, "RRS-N0002");
        assert!(result.message.contains("node unknown."), "{}", result.message);
    }
}
