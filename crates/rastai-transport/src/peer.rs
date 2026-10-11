//! Who is at the other end of a local socket, and the same-user check every local socket applies (contracts 3.2:
//! daemon.sock, node.sock, worker-host.sock and the job sockets all serve their own user only).
//!
//! The kernel reports the peer's user and process ids (SO_PEERCRED on Linux, getpeereid on macOS), read through
//! tokio. Our own user id is read the same way, from a socket pair this process holds both ends of: std has no
//! getuid, and this crate may not use `unsafe` or libc (ADR-0033).

use crate::error::TransportError;

/// The user and process of a local socket's peer. The process id is None where the platform doesn't report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerIdentity {
    pub user_id: u32,
    pub process_id: Option<i32>,
}

/// Ok when the peer runs as this process's user, else `PeerIsAnotherUser` (RRS-N2006).
/// It reads our own user id from a socket pair, so call it inside the tokio runtime, as every socket function is.
pub fn check_same_user(peer: &PeerIdentity) -> Result<(), TransportError> {
    let own_user_id = current_user_id()?;
    compare_users(peer, own_user_id)
}

/// The decision alone, so a test can pass any pair of ids.
pub(crate) fn compare_users(peer: &PeerIdentity, own_user_id: u32) -> Result<(), TransportError> {
    if peer.user_id == own_user_id {
        return Ok(());
    }
    Err(TransportError::PeerIsAnotherUser {
        peer_user_id: peer.user_id,
        own_user_id,
    })
}

/// This process's effective user id: the peer of a socket pair we hold both ends of is ourselves.
#[cfg(unix)]
pub(crate) fn current_user_id() -> Result<u32, TransportError> {
    let (one_end, _other_end) = tokio::net::UnixStream::pair().map_err(|error| TransportError::SocketNotCreated {
        socket_path: std::path::PathBuf::new(),
        reason: format!("socket pair for reading our own user id: {error}"),
    })?;
    let ourselves = peer_identity_of(&one_end).map_err(credentials_unreadable)?;
    Ok(ourselves.user_id)
}

/// The peer's credentials couldn't be read, keeping the operating system's words.
#[cfg(unix)]
pub(crate) fn credentials_unreadable(error: std::io::Error) -> TransportError {
    crate::error::connection_failed(format!("reading the peer's credentials: {error}"))
}

/// Windows has no Unix user ids; its named pipes (tier 2) will check the pipe's owner instead.
#[cfg(not(unix))]
pub(crate) fn current_user_id() -> Result<u32, TransportError> {
    Err(TransportError::NotSupportedOnThisPlatform {
        what: "check_same_user",
    })
}

/// The peer of a connected Unix socket, as the kernel reports it. The operating system's error is kept as it is,
/// because the accept loop treats one kind of it (a client that already hung up) differently.
#[cfg(unix)]
pub(crate) fn peer_identity_of(stream: &tokio::net::UnixStream) -> std::io::Result<PeerIdentity> {
    let credentials = stream.peer_cred()?;
    Ok(PeerIdentity {
        user_id: credentials.uid(),
        process_id: credentials.pid(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const OWN_USER_ID: u32 = 1000;
    const ANOTHER_USER_ID: u32 = 1001;

    #[test]
    fn a_peer_of_the_same_user_passes() {
        let peer = PeerIdentity {
            user_id: OWN_USER_ID,
            process_id: Some(42),
        };
        assert_eq!(compare_users(&peer, OWN_USER_ID), Ok(()));
    }

    #[test]
    fn a_peer_of_another_user_is_refused_with_both_ids() {
        let peer = PeerIdentity {
            user_id: ANOTHER_USER_ID,
            process_id: None,
        };
        let refusal = compare_users(&peer, OWN_USER_ID);
        let expected = TransportError::PeerIsAnotherUser {
            peer_user_id: ANOTHER_USER_ID,
            own_user_id: OWN_USER_ID,
        };
        assert_eq!(refusal, Err(expected));
    }

    #[test]
    fn root_is_another_user_too() {
        // No exception for root (user 0): the rule is the same user, nothing else.
        let root = PeerIdentity {
            user_id: 0,
            process_id: Some(1),
        };
        assert!(compare_users(&root, OWN_USER_ID).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn our_own_user_id_matches_the_owner_of_a_file_we_create() {
        use std::os::unix::fs::MetadataExt;
        let file_path = std::env::temp_dir().join(format!("rastai-transport-peer-{}", std::process::id()));
        std::fs::write(&file_path, b"x").unwrap();
        let file_owner = std::fs::metadata(&file_path).unwrap().uid();
        let _ = std::fs::remove_file(&file_path);
        assert_eq!(current_user_id(), Ok(file_owner));
    }
}
