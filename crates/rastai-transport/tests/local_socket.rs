//! Local sockets end to end (layout: rust-node-gateway-proto.md section 4.7): bind and connect, the peer's
//! credentials, the 0700 socket folder (a looser one is refused, contracts 8.1), a stale socket file replaced.
//! "A folder another user owns is refused" needs a second user, so the folder decision is unit-tested with a made-up
//! owner in src/local_socket.rs. So is "a peer of another user is refused": those unit tests run the real accept and
//! connect checks on a real connection, passing them a user id that isn't ours.
//!
//! Unix sockets only; on Windows every function returns RRS-N2009 until named pipes arrive (tier 2).

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod on_unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use rastai_transport::local_socket::{
        LocalListener, LocalStream, SOCKET_DIRECTORY_MODE, accept_local, bind_local_socket, connect_local_socket,
    };
    use rastai_transport::peer::{PeerIdentity, check_same_user};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::common::{SOCKET_FOLDER, new_test_folder};

    const SOCKET_FILE: &str = "test.sock";
    const TEST_FOLDER_PREFIX: &str = "rtl";
    const GREETING: &[u8] = b"hello over a local socket";
    /// Where a symbolic link put in place of the socket folder points.
    const LINK_TARGET_FOLDER: &str = "elsewhere";

    fn socket_path_in(test_folder: &Path) -> PathBuf {
        crate::common::socket_path_in(test_folder, SOCKET_FILE)
    }

    fn unix_stream(stream: LocalStream) -> tokio::net::UnixStream {
        match stream {
            LocalStream::Unix(unix_stream) => unix_stream,
        }
    }

    /// Connects while the listener accepts, and returns both ends and the peer the server saw.
    async fn connect_and_accept(
        listener: &LocalListener,
        socket_path: &Path,
    ) -> (tokio::net::UnixStream, tokio::net::UnixStream, PeerIdentity) {
        let (accepted, connected) = tokio::join!(accept_local(listener), connect_local_socket(socket_path));
        let (server_end, peer) = accepted.unwrap();
        (unix_stream(server_end), unix_stream(connected.unwrap()), peer)
    }

    #[tokio::test]
    async fn bind_and_connect_carry_bytes_and_name_this_process_as_the_peer() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let listener = bind_local_socket(&socket_path).unwrap();

        let (mut server_end, mut client_end, peer) = connect_and_accept(&listener, &socket_path).await;
        client_end.write_all(GREETING).await.unwrap();
        let mut received = vec![0; GREETING.len()];
        server_end.read_exact(&mut received).await.unwrap();

        assert_eq!(received, GREETING);
        let own_process_id = i32::try_from(std::process::id()).unwrap();
        assert_eq!(peer.process_id, Some(own_process_id));
        assert_eq!(check_same_user(&peer), Ok(()));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn the_peer_check_refuses_another_user_id() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let listener = bind_local_socket(&socket_path).unwrap();
        let (_server_end, _client_end, peer) = connect_and_accept(&listener, &socket_path).await;

        let another_user = PeerIdentity {
            user_id: peer.user_id.wrapping_add(1),
            process_id: peer.process_id,
        };
        let refusal = check_same_user(&another_user);
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2006"));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn the_socket_folder_is_created_with_mode_0700() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let _listener = bind_local_socket(&socket_path).unwrap();

        let folder_mode = fs::metadata(test_folder.join(SOCKET_FOLDER))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(folder_mode, SOCKET_DIRECTORY_MODE);
        assert_eq!(folder_mode, 0o700);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_socket_folder_others_can_enter_is_refused_on_both_ends() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let socket_folder = test_folder.join(SOCKET_FOLDER);
        fs::create_dir(&socket_folder).unwrap();
        fs::set_permissions(&socket_folder, fs::Permissions::from_mode(0o755)).unwrap();

        let bind_refusal = bind_local_socket(&socket_path).map(|_| ());
        let connect_refusal = connect_local_socket(&socket_path).await.map(|_| ());

        assert_eq!(bind_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        assert_eq!(connect_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        // The folder is refused, never "repaired": its mode is what it was.
        let folder_mode = fs::metadata(&socket_folder).unwrap().permissions().mode() & 0o777;
        assert_eq!(folder_mode, 0o755);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_symbolic_link_to_a_private_folder_is_refused_on_both_ends() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        // The link's target is a folder of ours at 0700, so only the link itself can make it insecure.
        let link_target = test_folder.join(LINK_TARGET_FOLDER);
        fs::create_dir(&link_target).unwrap();
        fs::set_permissions(&link_target, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&link_target, test_folder.join(SOCKET_FOLDER)).unwrap();

        let bind_refusal = bind_local_socket(&socket_path).map(|_| ());
        let connect_refusal = connect_local_socket(&socket_path).await.map(|_| ());

        assert_eq!(bind_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        assert_eq!(connect_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        assert!(!link_target.join(SOCKET_FILE).exists());
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_dangling_symbolic_link_as_the_socket_folder_is_insecure_not_unreachable() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        // A link to a folder that doesn't exist yet: whoever creates the target later would hold our sockets.
        std::os::unix::fs::symlink(test_folder.join(LINK_TARGET_FOLDER), test_folder.join(SOCKET_FOLDER)).unwrap();

        let connect_refusal = connect_local_socket(&socket_path).await.map(|_| ());
        let bind_refusal = bind_local_socket(&socket_path).map(|_| ());

        assert_eq!(connect_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        assert_eq!(bind_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        // Binding never followed the link to create its target.
        assert!(!test_folder.join(LINK_TARGET_FOLDER).exists());
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_file_in_place_of_the_socket_folder_is_refused_as_not_a_folder() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        fs::write(test_folder.join(SOCKET_FOLDER), b"not a folder").unwrap();

        let bind_refusal = bind_local_socket(&socket_path).map(|_| ());
        let connect_refusal = connect_local_socket(&socket_path).await.map(|_| ());

        assert_eq!(bind_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        assert_eq!(connect_refusal.map_err(|error| error.code()), Err("RRS-N2005"));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_stale_socket_file_is_replaced() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        // A process that died without removing its socket: the file stays, nobody listens.
        drop(bind_local_socket(&socket_path).unwrap());
        assert!(socket_path.exists());

        let listener = bind_local_socket(&socket_path).unwrap();
        let (_server_end, _client_end, peer) = connect_and_accept(&listener, &socket_path).await;
        assert_eq!(check_same_user(&peer), Ok(()));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_live_socket_is_not_taken_over() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let listener = bind_local_socket(&socket_path).unwrap();

        let second_bind = bind_local_socket(&socket_path).map(|_| ());
        assert_eq!(second_bind.map_err(|error| error.code()), Err("RRS-N2007"));
        // The first listener still owns the path and still accepts.
        let (_server_end, _client_end, peer) = connect_and_accept(&listener, &socket_path).await;
        assert_eq!(check_same_user(&peer), Ok(()));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_file_that_isnt_a_socket_is_never_deleted() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        fs::create_dir(test_folder.join(SOCKET_FOLDER)).unwrap();
        fs::set_permissions(test_folder.join(SOCKET_FOLDER), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&socket_path, b"the user's file").unwrap();

        let refusal = bind_local_socket(&socket_path).map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2007"));
        assert_eq!(fs::read(&socket_path).unwrap(), b"the user's file");
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn connecting_with_nothing_listening_is_not_reachable() {
        let test_folder = new_test_folder(TEST_FOLDER_PREFIX);
        let socket_path = socket_path_in(&test_folder);
        let missing_folder = connect_local_socket(&socket_path).await.map(|_| ());
        // The folder exists and is private, but nobody bound the socket.
        drop(bind_local_socket(&test_folder.join(SOCKET_FOLDER).join("other.sock")).unwrap());
        let missing_socket = connect_local_socket(&socket_path).await.map(|_| ());

        assert_eq!(missing_folder.map_err(|error| error.code()), Err("RRS-N2004"));
        assert_eq!(missing_socket.map_err(|error| error.code()), Err("RRS-N2004"));
        fs::remove_dir_all(&test_folder).unwrap();
    }
}

#[cfg(windows)]
mod on_windows {
    use std::path::Path;

    use rastai_transport::local_socket::{bind_local_socket, connect_local_socket};

    #[tokio::test]
    async fn local_sockets_say_they_arent_supported_yet() {
        let socket_path = Path::new(r"C:\rastai-test\daemon.sock");
        let bind_refusal = bind_local_socket(socket_path).map(|_| ());
        let connect_refusal = connect_local_socket(socket_path).await.map(|_| ());
        assert_eq!(bind_refusal.map_err(|error| error.code()), Err("RRS-N2009"));
        assert_eq!(connect_refusal.map_err(|error| error.code()), Err("RRS-N2009"));
    }
}
