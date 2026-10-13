//! `ServerError`, the one error type of this crate (contracts 12.2). The startup functions return it; the edge
//! (`rastai serve` in rastai-cli) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists. Errors of the crates the daemon calls keep their own
//! variant and code, so a refused start says what really went wrong (a held lock is `RRS-S0001`, a socket folder
//! another user owns `RRS-N2005`). Request-time refusals are `Diagnostic`s straight away (http/gate.rs).
//!
//! A `StoreError` becomes its `Diagnostic` in rastai-store's own `store_error_diagnostic`, the one place that fills
//! each catalog placeholder from the error's fields, for the daemon's run-time reports and the offline rebuild alike.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rastai_core::error::CoreError;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_node::error::{NodeError, node_error_diagnostic};
use rastai_store::error::{StoreError, store_error_diagnostic};
use rastai_transport::error::TransportError;

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_NO_FREE_PORT: &str = "RRS-C1001";
const CODE_OWNER_TOKEN_TOO_OPEN: &str = "RRS-C1008";
const CODE_OWNER_TOKEN_UNUSABLE: &str = "RRS-C1010";
const CODE_DATA_DIRECTORY_NOT_WRITABLE: &str = "RRS-S0004";
/// What `RRS-C0002` says a listen address should be.
const LISTEN_ADDRESS_EXPECTED: &str = "an address of this machine and a port this user may open";

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
    /// daemon.sock or node-hub.sock couldn't be bound (rastai-transport).
    Transport(TransportError),
    /// `daemon.key`, the embedded node's key or its pinned daemon keys can't be used (rastai-node, `RRS-N0006`).
    Node(NodeError),
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
            ServerError::Node(error) => error.code(),
        }
    }
}

/// A startup error as the catalog diagnostic of its code, every placeholder of the code's message filled.
/// `rastai serve` in rastai-cli prints it when the daemon can't start.
pub fn server_error_diagnostic(error: &ServerError) -> Diagnostic {
    match error {
        ServerError::OwnerTokenTooOpen { path, .. } | ServerError::RuntimeFileNotWritable { path, .. } => {
            diagnostic(error.code(), &[("path", &path_text(path))])
        }
        ServerError::OwnerTokenUnusable { path, reason } => {
            diagnostic(error.code(), &[("path", &path_text(path)), ("reason", reason)])
        }
        ServerError::ListenAddressUnusable { address, reason } => {
            let found = format!("{address} ({reason})");
            diagnostic(
                error.code(),
                &[("found", &found), ("expected", LISTEN_ADDRESS_EXPECTED)],
            )
        }
        ServerError::InvalidSetFlag { message } => {
            // The flag parser already built the whole message; only its words are kept.
            let mut refusal = diagnostic(error.code(), &[]);
            refusal.message = message.clone();
            refusal
        }
        ServerError::Core(CoreError::RandomUnavailable { reason }) => diagnostic(error.code(), &[("reason", reason)]),
        ServerError::Core(CoreError::InvalidTimestamp { text, reason }) => {
            diagnostic(error.code(), &[("found", text), ("expected", reason)])
        }
        ServerError::Store(store_error) => store_error_diagnostic(store_error),
        ServerError::Transport(transport_error) => transport_error_diagnostic(transport_error),
        ServerError::Node(node_error) => node_error_diagnostic(node_error),
        ServerError::NoFreePort { .. } | ServerError::Core(CoreError::HomeNotFound) => diagnostic(error.code(), &[]),
    }
}

/// The socket path and reason a transport error knows, as its message's placeholders.
fn transport_error_diagnostic(error: &TransportError) -> Diagnostic {
    let (path, reason) = match error {
        TransportError::SocketNotReachable { socket_path, reason }
        | TransportError::SocketNotCreated { socket_path, reason } => (path_text(socket_path), reason.clone()),
        TransportError::SocketFolderInsecure { folder, reason } => (path_text(folder), reason.clone()),
        TransportError::ConnectionFailed { reason } => (String::new(), reason.clone()),
        other => (String::new(), format!("{other:?}")),
    };
    diagnostic(
        error.code(),
        &[("path", &path), ("reason", &reason), ("platform", std::env::consts::OS)],
    )
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

impl From<NodeError> for ServerError {
    fn from(error: NodeError) -> ServerError {
        ServerError::Node(error)
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
    fn a_startup_diagnostic_fills_its_placeholders() {
        let path = PathBuf::from("/home/user/.config/rastai/auth/owner.token");
        let unusable = ServerError::OwnerTokenUnusable {
            path: path.clone(),
            reason: "permission denied".to_string(),
        };
        let shown = server_error_diagnostic(&unusable);
        assert_eq!(shown.code, "RRS-C1010");
        assert!(shown.message.contains("owner.token") && shown.message.contains("permission denied"));
        let bad_flag = ServerError::InvalidSetFlag {
            message: "the flag message".to_string(),
        };
        assert_eq!(server_error_diagnostic(&bad_flag).message, "the flag message");
        let held = ServerError::Store(StoreError::DaemonLockHeld {
            path: path.clone(),
            holder_pid: Some(42),
        });
        assert!(server_error_diagnostic(&held).message.contains("42"));
        for error in [unusable, held, ServerError::OwnerTokenTooOpen { path, mode: 0o644 }] {
            assert!(!server_error_diagnostic(&error).message.contains('{'), "{error:?}");
        }
    }

    #[test]
    fn a_node_startup_error_is_its_identity_diagnostic() {
        let unusable_key = ServerError::from(NodeError::IdentityFileUnusable {
            path: PathBuf::from("/home/user/.local/share/rastai/daemon.key"),
            reason: "permission denied".to_string(),
        });
        let shown = server_error_diagnostic(&unusable_key);
        assert_eq!(shown.code, "RRS-N0006");
        assert_eq!(
            shown.message,
            "The identity file /home/user/.local/share/rastai/daemon.key can't be used: permission denied."
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
