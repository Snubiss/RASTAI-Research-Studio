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
use crate::peer::{compare_users, credentials_unreadable, current_user_id, peer_identity_of};

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
    let own_user_id = current_user_id()?;
    match listener {
        LocalListener::Unix(unix_listener) => accept_unix(unix_listener, own_user_id).await,
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
    let own_user_id = current_user_id()?;
    keep_if_served_by_user(stream, own_user_id)
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
    // accepted, the live listener later sees this probe as a client that has already hung up. (A live listener
    // whose queue is full could make it wait; callers hold their process lock, so no second copy of us is there.)
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_probe) => Err(not_created(socket_path, "another process is listening on this socket")),
        // Refused is the only answer that proves nobody listens. Any other (no permission, say) proves nothing,
        // so the file stays.
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            fs::remove_file(socket_path).map_err(|error| not_created(socket_path, error))
        }
        Err(error) => Err(not_created(
            socket_path,
            format!("can't tell whether anyone listens on this socket, so it is left alone: {error}"),
        )),
    }
}

/// Accepts until a client is still there to be served, and refuses one that doesn't run as `own_user_id`
/// (RRS-N2006). `accept_local` passes our own user id; the tests pass another, to see the refusal on a real
/// connection. macOS can't read the credentials of a client that hung up before we looked (ENOTCONN), such as the
/// probe of a second `bind_local_socket`; there is nobody to serve, so that connection is skipped. Linux keeps the
/// credentials, and the closed connection simply reads as ended.
#[cfg(unix)]
async fn accept_unix(
    listener: &tokio::net::UnixListener,
    own_user_id: u32,
) -> Result<(LocalStream, PeerIdentity), TransportError> {
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
        compare_users(&peer, own_user_id)?;
        return Ok((LocalStream::Unix(stream), peer));
    }
}

/// The client's peer check: keeps the connection when the server runs as `own_user_id`, else refuses it
/// (RRS-N2006). `connect_local_socket` passes our own user id; the tests pass another. It is separate from the
/// folder check before it because a test can't make a folder another user owns, yet can pass another user id here.
#[cfg(unix)]
fn keep_if_served_by_user(stream: tokio::net::UnixStream, own_user_id: u32) -> Result<LocalStream, TransportError> {
    let server = peer_identity_of(&stream).map_err(credentials_unreadable)?;
    // Returning early drops `stream`, which closes the refused connection.
    compare_users(&server, own_user_id)?;
    Ok(LocalStream::Unix(stream))
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
#[allow(clippy::unwrap_used)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use tokio::io::AsyncReadExt;

    use super::*;

    const OWN_USER_ID: u32 = 1000;
    const ANOTHER_USER_ID: u32 = 1001;
    const ROOT_USER_ID: u32 = 0;
    const SOCKET_FOLDER: &str = "run";
    const SOCKET_FILE: &str = "unit.sock";
    /// A socket file nobody may read or write, so connecting to it is "permission denied", not "refused".
    const NO_PERMISSIONS: u32 = 0o000;

    /// A fresh test folder and a socket path inside it, short enough for the 107-byte Unix socket path limit.
    fn new_socket_path(test_tag: &str) -> (PathBuf, PathBuf) {
        let test_folder = std::env::temp_dir().join(format!("rtu-{}-{test_tag}", std::process::id()));
        let _ = fs::remove_dir_all(&test_folder);
        fs::create_dir_all(&test_folder).unwrap();
        let socket_path = test_folder.join(SOCKET_FOLDER).join(SOCKET_FILE);
        (test_folder, socket_path)
    }

    /// True once the other end has closed the connection: a read then returns no bytes.
    async fn was_closed(stream: &mut tokio::net::UnixStream) -> bool {
        let mut buffer = [0; 1];
        stream.read(&mut buffer).await.unwrap() == 0
    }

    #[tokio::test]
    async fn accepting_refuses_a_client_of_another_user_and_closes_it() {
        let (test_folder, socket_path) = new_socket_path("accept");
        let LocalListener::Unix(listener) = bind_local_socket(&socket_path).unwrap();
        let real_user_id = current_user_id().unwrap();
        let pretended_user_id = real_user_id.wrapping_add(1);

        let (accepted, connected) = tokio::join!(
            accept_unix(&listener, pretended_user_id),
            tokio::net::UnixStream::connect(&socket_path)
        );

        let expected = TransportError::PeerIsAnotherUser {
            peer_user_id: real_user_id,
            own_user_id: pretended_user_id,
        };
        assert_eq!(accepted.map(|_| ()), Err(expected));
        assert!(was_closed(&mut connected.unwrap()).await);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn connecting_refuses_a_server_of_another_user_and_closes_it() {
        let (test_folder, socket_path) = new_socket_path("connect");
        let LocalListener::Unix(listener) = bind_local_socket(&socket_path).unwrap();
        let real_user_id = current_user_id().unwrap();
        let pretended_user_id = real_user_id.wrapping_add(1);

        let (accepted, connected) = tokio::join!(
            accept_unix(&listener, real_user_id),
            tokio::net::UnixStream::connect(&socket_path)
        );
        let (LocalStream::Unix(mut server_end), _client) = accepted.unwrap();
        let refusal = keep_if_served_by_user(connected.unwrap(), pretended_user_id);

        let expected = TransportError::PeerIsAnotherUser {
            peer_user_id: real_user_id,
            own_user_id: pretended_user_id,
        };
        assert_eq!(refusal.map(|_| ()), Err(expected));
        assert!(was_closed(&mut server_end).await);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_server_of_our_own_user_is_kept() {
        let (test_folder, socket_path) = new_socket_path("own");
        let LocalListener::Unix(listener) = bind_local_socket(&socket_path).unwrap();
        let real_user_id = current_user_id().unwrap();

        let (accepted, connected) = tokio::join!(
            accept_unix(&listener, real_user_id),
            tokio::net::UnixStream::connect(&socket_path)
        );

        assert!(accepted.is_ok());
        assert!(keep_if_served_by_user(connected.unwrap(), real_user_id).is_ok());
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_socket_that_cant_be_probed_is_never_deleted() {
        let (test_folder, socket_path) = new_socket_path("probe");
        // A socket nobody listens on, but whose mode denies the probe: "permission denied" proves nothing.
        drop(bind_local_socket(&socket_path).unwrap());
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(NO_PERMISSIONS)).unwrap();
        // Root connects whatever the mode, so its probe is simply refused; the case can't happen for root.
        if current_user_id().unwrap() == ROOT_USER_ID {
            fs::remove_dir_all(&test_folder).unwrap();
            return;
        }

        let refusal = bind_local_socket(&socket_path).map(|_| ());

        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2007"));
        assert!(fs::symlink_metadata(&socket_path).unwrap().file_type().is_socket());
        fs::remove_dir_all(&test_folder).unwrap();
    }

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
