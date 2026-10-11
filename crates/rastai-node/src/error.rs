//! `NodeError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (the tonic
//! service in node_service.rs, the CLI's `node serve`, the daemon's start of the embedded node) turns it into a
//! `Diagnostic` through `code()` and `node_error_diagnostic`.
//!
//! Every variant maps to a code that contracts 10.3 lists and node layout 11.1 gives this crate. The `reason` fields
//! keep the operating system's or the library's own words for the log.

use std::path::PathBuf;

use rastai_core::error::CoreError;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_transport::error::TransportError;

const CODE_PROTOCOL_VERSION_UNSUPPORTED: &str = "RRS-N0001";
const CODE_DAEMON_NOT_PAIRED: &str = "RRS-N0002";
const CODE_NODE_ALREADY_RUNNING: &str = "RRS-N0003";
const CODE_IDENTITY_FILE_UNUSABLE: &str = "RRS-N0006";
const CODE_HANDSHAKE_STALE: &str = "RRS-N0007";
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
