//! Bind and connect local sockets (layout: rust-node-gateway-proto.md section 4.2). Unix sockets now; Windows named
//! pipes are a later branch of these same enums and functions (tier 2), and until then every function here returns
//! `NotSupportedOnThisPlatform` (RRS-N2009) on Windows.
//!
//! The socket's folder is the security boundary (contracts 3.2, 8.1): `bind_local_socket` creates it with mode 0700
//! and refuses one that another user owns or that group or others can use, so a folder planted in the shared /tmp
//! can't catch the sockets. `connect_local_socket` checks the same folder before it connects, and both ends check
//! that the peer is the same user.

use std::path::Path;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};

use crate::error::TransportError;
use crate::peer::PeerIdentity;
#[cfg(unix)]
use crate::peer::{check_same_user, credentials_unreadable, current_user_id, peer_identity_of};

/// The socket folder's mode: only its owner may list, enter or create files in it (contracts 8.1).
pub const SOCKET_DIRECTORY_MODE: u32 = 0o700;
/// The permission bits of a mode; the file-type bits above them are ignored.
#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o777;
/// Group and others bits. Any of them set on the socket folder is refused.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;

/// A bound local socket, waiting for connections.
#[derive(Debug)]
pub enum LocalListener {
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
    // NamedPipe arrives with tier 2 (Windows). Until then this enum has no variant on Windows, so no value of it
    // can exist there, and a `match` on one has no arms.
}

/// One connected local socket, either end.
#[derive(Debug)]
pub enum LocalStream {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    // NamedPipe arrives with tier 2 (Windows), as in LocalListener.
}

/// Binds a local socket at `socket_path`, after creating its folder with mode 0700 and checking it (RRS-N2005).
/// A stale socket file (one nobody listens on any more) is replaced. A live one is refused, and a file that isn't
/// a socket is never deleted; both are `SocketNotCreated` (RRS-N2007). Callers take their process lock first
/// (`daemon.lock`, the node's lock), so a live socket here means a second copy of the process.
#[cfg(unix)]
pub fn bind_local_socket(socket_path: &Path) -> Result<LocalListener, TransportError> {
    let folder = socket_folder(socket_path)?;
    create_socket_folder(folder, socket_path)?;
    check_socket_folder(folder)?;
    remove_stale_socket(socket_path)?;
    let listener = tokio::net::UnixListener::bind(socket_path).map_err(|error| not_created(socket_path, error))?;
    Ok(LocalListener::Unix(listener))
}

#[cfg(not(unix))]
pub fn bind_local_socket(_socket_path: &Path) -> Result<LocalListener, TransportError> {
    Err(TransportError::NotSupportedOnThisPlatform {
        what: "bind_local_socket",
    })
}

/// The next connection and who made it. A peer of another user is refused (RRS-N2006) and its connection closed;
/// an accept loop logs that and keeps accepting.
#[cfg(unix)]
pub async fn accept_local(listener: &LocalListener) -> Result<(LocalStream, PeerIdentity), TransportError> {
    match listener {
        LocalListener::Unix(unix_listener) => accept_unix(unix_listener).await,
    }
}

#[cfg(not(unix))]
pub async fn accept_local(listener: &LocalListener) -> Result<(LocalStream, PeerIdentity), TransportError> {
    // No LocalListener can exist here yet (see the enum), so there is nothing to match.
    match *listener {}
}

/// Connects to a local socket after checking its folder (RRS-N2005), and refuses a socket that another user
/// serves (RRS-N2006). Nothing listening is `SocketNotReachable` (RRS-N2004).
#[cfg(unix)]
pub async fn connect_local_socket(socket_path: &Path) -> Result<LocalStream, TransportError> {
    check_folder_before_connecting(socket_path)?;
    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|error| not_reachable(socket_path, error))?;
    let server = peer_identity_of(&stream).map_err(credentials_unreadable)?;
    check_same_user(&server)?;
    Ok(LocalStream::Unix(stream))
}

#[cfg(not(unix))]
pub async fn connect_local_socket(_socket_path: &Path) -> Result<LocalStream, TransportError> {
    Err(TransportError::NotSupportedOnThisPlatform {
        what: "connect_local_socket",
    })
}

/// The client's half of the folder rule, before it connects: only this user can have bound a socket in a 0700
/// folder this user owns, so the server is ours. The gRPC channel calls it too. A missing folder only means
/// nothing was ever bound there, so it is "not reachable" (RRS-N2004), not "insecure".
#[cfg(unix)]
pub(crate) fn check_folder_before_connecting(socket_path: &Path) -> Result<(), TransportError> {
    let folder = socket_folder(socket_path)?;
    if !folder.exists() {
        return Err(not_reachable(socket_path, "its folder doesn't exist"));
    }
    check_socket_folder(folder)
}

/// Checks the folder a socket lives in without creating it: a folder, owned by this user, mode 0700.
#[cfg(unix)]
fn check_socket_folder(folder: &Path) -> Result<(), TransportError> {
    let metadata = fs::symlink_metadata(folder).map_err(|error| TransportError::SocketFolderInsecure {
        folder: folder.to_path_buf(),
        reason: format!("can't be read: {error}"),
    })?;
    let facts = FolderFacts {
        is_folder: metadata.file_type().is_dir(),
        owner_user_id: metadata.uid(),
        mode: metadata.mode() & PERMISSION_BITS,
    };
    match folder_problem(&facts, current_user_id()?) {
        None => Ok(()),
        Some(reason) => Err(TransportError::SocketFolderInsecure {
            folder: folder.to_path_buf(),
            reason,
        }),
    }
}

/// The parent folder of a socket path. A bare file name has none we could check, so it is refused.
#[cfg(unix)]
fn socket_folder(socket_path: &Path) -> Result<&Path, TransportError> {
    match socket_path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => Ok(folder),
        _ => Err(TransportError::SocketNotCreated {
            socket_path: socket_path.to_path_buf(),
            reason: "the path has no folder; socket paths come from rastai-core's paths.rs".to_string(),
        }),
    }
}

/// What the folder check needs to know about a folder, so the decision can be tested with any owner and mode.
#[cfg(unix)]
struct FolderFacts {
    is_folder: bool,
    owner_user_id: u32,
    mode: u32,
}

/// Why a socket folder is refused, or None when it is fine.
#[cfg(unix)]
fn folder_problem(facts: &FolderFacts, own_user_id: u32) -> Option<String> {
    if !facts.is_folder {
        return Some("it isn't a folder (a symbolic link or a file)".to_string());
    }
    if facts.owner_user_id != own_user_id {
        return Some(format!(
            "it is owned by user {}, not by user {own_user_id}",
            facts.owner_user_id
        ));
    }
    if facts.mode & GROUP_AND_OTHER_BITS != 0 {
        return Some(format!(
            "its mode is {:04o}; only its owner may use it ({SOCKET_DIRECTORY_MODE:04o})",
            facts.mode
        ));
    }
    None
}

/// Creates the folder, and any missing folder above it, with mode 0700. One that already exists is left as it is
/// for the check that follows: we never loosen or "repair" a folder someone else may have made.
#[cfg(unix)]
fn create_socket_folder(folder: &Path, socket_path: &Path) -> Result<(), TransportError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(SOCKET_DIRECTORY_MODE);
    builder.create(folder).map_err(|error| not_created(socket_path, error))
}

/// Removes a socket file nobody listens on any more. A socket that still answers belongs to a running process and
/// a file that isn't a socket may be the user's; both are refused, never deleted.
#[cfg(unix)]
fn remove_stale_socket(socket_path: &Path) -> Result<(), TransportError> {
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(not_created(socket_path, error)),
    };
    if !metadata.file_type().is_socket() {
        return Err(not_created(
            socket_path,
            "a file that isn't a socket is in the way; it is never deleted",
        ));
    }
    // A blocking connect is fine here: on a local socket it answers at once, accepted or refused. If it is
    // accepted, the live listener later sees this probe as a client that has already hung up.
    if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
        return Err(not_created(socket_path, "another process is listening on this socket"));
    }
    fs::remove_file(socket_path).map_err(|error| not_created(socket_path, error))
}

/// Accepts until a client is still there to be served. macOS can't read the credentials of a client that hung up
/// before we looked (ENOTCONN), such as the probe of a second `bind_local_socket`; there is nobody to serve, so
/// that connection is skipped. Linux keeps the credentials, and the closed connection simply reads as ended.
#[cfg(unix)]
async fn accept_unix(listener: &tokio::net::UnixListener) -> Result<(LocalStream, PeerIdentity), TransportError> {
    loop {
        let (stream, _client_address) = listener
            .accept()
            .await
            .map_err(|error| crate::error::connection_failed(format!("accepting a connection: {error}")))?;
        let peer = match peer_identity_of(&stream) {
            Ok(peer) => peer,
            Err(error) if error.kind() == ErrorKind::NotConnected => continue,
            Err(error) => return Err(credentials_unreadable(error)),
        };
        // Returning early drops `stream`, which closes the refused connection.
        check_same_user(&peer)?;
        return Ok((LocalStream::Unix(stream), peer));
    }
}

#[cfg(unix)]
fn not_created(socket_path: &Path, reason: impl std::fmt::Display) -> TransportError {
    TransportError::SocketNotCreated {
        socket_path: socket_path.to_path_buf(),
        reason: reason.to_string(),
    }
}

#[cfg(unix)]
fn not_reachable(socket_path: &Path, reason: impl std::fmt::Display) -> TransportError {
    TransportError::SocketNotReachable {
        socket_path: socket_path.to_path_buf(),
        reason: reason.to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const OWN_USER_ID: u32 = 1000;
    const ANOTHER_USER_ID: u32 = 1001;

    fn folder(owner_user_id: u32, mode: u32) -> FolderFacts {
        FolderFacts {
            is_folder: true,
            owner_user_id,
            mode,
        }
    }

    #[test]
    fn our_own_0700_folder_passes() {
        assert_eq!(folder_problem(&folder(OWN_USER_ID, 0o700), OWN_USER_ID), None);
    }

    #[test]
    fn a_folder_another_user_owns_is_refused_even_at_0700() {
        // The /tmp/rastai-<uid> case (contracts 8.1): another user made the folder before us.
        let problem = folder_problem(&folder(ANOTHER_USER_ID, 0o700), OWN_USER_ID);
        assert_eq!(problem, Some("it is owned by user 1001, not by user 1000".to_string()));
    }

    #[test]
    fn any_group_or_other_bit_is_refused() {
        for mode in [0o710, 0o701, 0o750, 0o755, 0o777, 0o704] {
            assert!(
                folder_problem(&folder(OWN_USER_ID, mode), OWN_USER_ID).is_some(),
                "mode {mode:o}"
            );
        }
    }

    #[test]
    fn a_tighter_mode_than_0700_passes() {
        // 0500 can't hold a new socket, but binding says so itself; the folder is still private.
        assert_eq!(folder_problem(&folder(OWN_USER_ID, 0o500), OWN_USER_ID), None);
    }

    #[test]
    fn a_symbolic_link_or_file_is_refused() {
        let link = FolderFacts {
            is_folder: false,
            owner_user_id: OWN_USER_ID,
            mode: 0o700,
        };
        assert!(folder_problem(&link, OWN_USER_ID).is_some());
    }

    #[test]
    fn a_bare_file_name_has_no_folder_to_check() {
        let refusal = socket_folder(Path::new("daemon.sock"));
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2007"));
    }
}
