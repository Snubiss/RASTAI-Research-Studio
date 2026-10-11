//! `TransportError`, the one error type of this crate (contracts 12.2). Library functions return it; the edge (a
//! tonic service, an HTTP handler, the CLI) turns it into a `Diagnostic` through `code()`.
//!
//! Every variant maps to a code that contracts 10.3 lists, all in the `RRS-N` heartbeat and reachability range
//! (2000–2999). The `reason` fields keep the operating system's or the library's own words for the log.

use std::path::PathBuf;

const CODE_SOCKET_NOT_REACHABLE: &str = "RRS-N2004";
const CODE_SOCKET_FOLDER_INSECURE: &str = "RRS-N2005";
const CODE_PEER_IS_ANOTHER_USER: &str = "RRS-N2006";
const CODE_SOCKET_NOT_CREATED: &str = "RRS-N2007";
const CODE_CONNECTION_FAILED: &str = "RRS-N2008";
const CODE_NOT_SUPPORTED_ON_THIS_PLATFORM: &str = "RRS-N2009";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Nothing is listening at the socket's path: the file or its folder is missing, or the listener is gone.
    SocketNotReachable { socket_path: PathBuf, reason: String },
    /// The socket's folder is owned by another user, open to group or others, or not a folder (a symbolic link or
    /// a file). A folder planted in the shared /tmp would otherwise catch the sockets (contracts 8.1). `reason` says
    /// which.
    SocketFolderInsecure { folder: PathBuf, reason: String },
    /// The process at the other end of a local socket runs as another user.
    PeerIsAnotherUser { peer_user_id: u32, own_user_id: u32 },
    /// The socket's folder or the socket file couldn't be created. Also a live socket already at the path, or a
    /// file in the way that isn't a socket (never deleted).
    SocketNotCreated { socket_path: PathBuf, reason: String },
    /// A local connection (a socket or loopback TCP) failed after it opened, or a request couldn't be sent.
    ConnectionFailed { reason: String },
    /// A local request took longer than its `timeout_ms`.
    TimedOut { timeout_ms: u64 },
    /// Local sockets aren't built for this platform yet: Windows named pipes come with tier 2. `what` names the
    /// function that was called.
    NotSupportedOnThisPlatform { what: &'static str },
}

impl TransportError {
    /// The catalog code for this error (contracts 10.3).
    pub fn code(&self) -> &'static str {
        match self {
            TransportError::SocketNotReachable { .. } => CODE_SOCKET_NOT_REACHABLE,
            TransportError::SocketFolderInsecure { .. } => CODE_SOCKET_FOLDER_INSECURE,
            TransportError::PeerIsAnotherUser { .. } => CODE_PEER_IS_ANOTHER_USER,
            TransportError::SocketNotCreated { .. } => CODE_SOCKET_NOT_CREATED,
            TransportError::ConnectionFailed { .. } => CODE_CONNECTION_FAILED,
            TransportError::TimedOut { .. } => CODE_CONNECTION_FAILED,
            TransportError::NotSupportedOnThisPlatform { .. } => CODE_NOT_SUPPORTED_ON_THIS_PLATFORM,
        }
    }
}

/// A failure of an open connection, keeping the library's words.
pub(crate) fn connection_failed(reason: impl std::fmt::Display) -> TransportError {
    TransportError::ConnectionFailed {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_catalog_code() {
        let path = PathBuf::from("/run/user/1000/rastai/daemon.sock");
        let reason = "r".to_string();
        let not_reachable = TransportError::SocketNotReachable {
            socket_path: path.clone(),
            reason: reason.clone(),
        };
        let insecure = TransportError::SocketFolderInsecure {
            folder: path.clone(),
            reason: reason.clone(),
        };
        let another_user = TransportError::PeerIsAnotherUser {
            peer_user_id: 1001,
            own_user_id: 1000,
        };
        let not_created = TransportError::SocketNotCreated {
            socket_path: path,
            reason: reason.clone(),
        };
        assert_eq!(not_reachable.code(), "RRS-N2004");
        assert_eq!(insecure.code(), "RRS-N2005");
        assert_eq!(another_user.code(), "RRS-N2006");
        assert_eq!(not_created.code(), "RRS-N2007");
        assert_eq!(TransportError::ConnectionFailed { reason }.code(), "RRS-N2008");
        assert_eq!(TransportError::TimedOut { timeout_ms: 5 }.code(), "RRS-N2008");
        let unsupported = TransportError::NotSupportedOnThisPlatform {
            what: "bind_local_socket",
        };
        assert_eq!(unsupported.code(), "RRS-N2009");
    }
}
