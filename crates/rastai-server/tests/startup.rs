//! Startup and shutdown (flows/platform.md 2.1, layout section 8.2): the owner token is created 0600 and refused
//! when its mode isn't 0600 or it is unreadable or malformed, before any database or listener; ports fall back and
//! land in daemon.port; health is up within a second; a clean stop removes the runtime files and releases the lock.

mod common;

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod on_unix {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    use rastai_core::paths::{DataPaths, daemon_port_path, daemon_socket_path, owner_token_path, rastai_db_path};
    use rastai_server::daemon::{DaemonOptions, start_daemon};
    use rastai_server::http::listeners::{FIRST_DEFAULT_PORT, LAST_DEFAULT_PORT, bind_tcp};
    use rastai_store::data_dir_lock::take_daemon_lock;
    use tokio::net::TcpListener;

    use crate::common::{
        options_on_any_port, refused_start_in_test, remove_test_folder, send_to_port, start_test_daemon,
        stop_test_daemon, test_paths,
    };

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    /// flows/platform.md 2.1: startup to health under 1 s.
    const HEALTHY_WITHIN: Duration = Duration::from_secs(1);
    /// An owner token in its one valid form (contracts 8.1).
    const VALID_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn mode_of(path: &std::path::Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Nothing of a refused start is left: no socket, no port file, no database.
    fn nothing_was_started(paths: &DataPaths) -> bool {
        !daemon_socket_path(paths).exists() && !daemon_port_path(paths).exists() && !rastai_db_path(paths).exists()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_first_start_writes_a_private_token_and_daemon_port_and_is_healthy_within_a_second() {
        let paths = test_paths("firststart");
        let started_at = Instant::now();
        let running = start_test_daemon(&paths).await;
        let health = send_to_port(running.http_port, "GET", "/api/health", &[], b"").await;
        let elapsed = started_at.elapsed();
        assert_eq!(health.status, 200);
        assert!(elapsed < HEALTHY_WITHIN, "healthy after {elapsed:?}");

        let token_path = owner_token_path(&paths);
        assert_eq!(mode_of(&token_path), 0o600);
        assert_eq!(mode_of(token_path.parent().unwrap()), 0o700);
        let token = fs::read_to_string(&token_path).unwrap();
        assert_eq!(token.len(), 64);
        let port_text = fs::read_to_string(daemon_port_path(&paths)).unwrap();
        assert_eq!(port_text, format!("{}\n", running.http_port));

        stop_test_daemon(running).await;
        assert!(!daemon_port_path(&paths).exists(), "a clean stop removes daemon.port");
        assert!(!daemon_socket_path(&paths).exists(), "a clean stop removes daemon.sock");

        // The next start keeps the same token: the daemon never rewrites it.
        let running = start_test_daemon(&paths).await;
        assert_eq!(fs::read_to_string(&token_path).unwrap(), token);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_others_can_read_refuses_the_start_until_it_is_deleted() {
        let paths = test_paths("openmode");
        let running = start_test_daemon(&paths).await;
        stop_test_daemon(running).await;
        fs::remove_file(rastai_db_path(&paths)).unwrap();
        let token_path = owner_token_path(&paths);
        let token = fs::read_to_string(&token_path).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o644)).unwrap();

        let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
        assert_eq!(refused.code(), "RRS-C1008");
        assert!(nothing_was_started(&paths), "no listener bound, no database opened");
        assert_eq!(
            fs::read_to_string(&token_path).unwrap(),
            token,
            "the token is never rewritten"
        );
        assert_eq!(mode_of(&token_path), 0o644, "nor its mode changed");

        fs::remove_file(&token_path).unwrap();
        let running = start_test_daemon(&paths).await;
        assert_eq!(mode_of(&token_path), 0o600);
        assert_ne!(
            fs::read_to_string(&token_path).unwrap(),
            token,
            "a new token was written"
        );
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_group_writable_token_refuses_the_start_too() {
        let paths = test_paths("groupwrite");
        let running = start_test_daemon(&paths).await;
        stop_test_daemon(running).await;
        fs::remove_file(rastai_db_path(&paths)).unwrap();
        let token_path = owner_token_path(&paths);
        let token = fs::read_to_string(&token_path).unwrap();
        // Not readable by others, but writable by the group: someone may have put a token they know in its place.
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o620)).unwrap();

        let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
        assert_eq!(refused.code(), "RRS-C1008");
        assert!(nothing_was_started(&paths));
        assert_eq!(fs::read_to_string(&token_path).unwrap(), token);
        assert_eq!(mode_of(&token_path), 0o620, "the mode is left for the owner to fix");
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_that_cant_be_read_refuses_the_start() {
        let paths = test_paths("unusable");
        let token_path = owner_token_path(&paths);
        fs::create_dir_all(&token_path).unwrap();
        let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
        assert_eq!(refused.code(), "RRS-C1010");
        assert!(nothing_was_started(&paths));
        fs::remove_dir(&token_path).unwrap();

        fs::write(&token_path, VALID_TOKEN).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads any file, so the open this test is about can't fail for it.
        if fs::read(&token_path).is_err() {
            let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
            assert_eq!(refused.code(), "RRS-C1010");
            assert!(nothing_was_started(&paths));
        }
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read_to_string(&token_path).unwrap(), VALID_TOKEN, "never rewritten");
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_that_isnt_64_lowercase_hex_characters_refuses_the_start() {
        let paths = test_paths("malformed");
        let token_path = owner_token_path(&paths);
        fs::create_dir_all(token_path.parent().unwrap()).unwrap();
        let uppercase = VALID_TOKEN.to_uppercase();
        let truncated = &VALID_TOKEN[..32];
        for text in ["", "3f", "password\n", truncated, uppercase.as_str()] {
            fs::write(&token_path, text).unwrap();
            fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600)).unwrap();
            let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
            assert_eq!(refused.code(), "RRS-C1010", "{text:?}");
            assert!(nothing_was_started(&paths));
            assert_eq!(fs::read_to_string(&token_path).unwrap(), text, "never rewritten");
        }
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_valid_token_in_a_read_only_auth_folder_starts() {
        let paths = test_paths("readonly");
        let token_path = owner_token_path(&paths);
        let auth_folder = token_path.parent().unwrap();
        fs::create_dir_all(auth_folder).unwrap();
        fs::write(&token_path, format!("{VALID_TOKEN}\n")).unwrap();
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(auth_folder, fs::Permissions::from_mode(0o500)).unwrap();

        let running = start_test_daemon(&paths).await;
        stop_test_daemon(running).await;
        fs::set_permissions(auth_folder, fs::Permissions::from_mode(0o700)).unwrap();
        remove_test_folder(&paths);
    }

    /// Whether one port of 47820–47829 can be bound right now. A real daemon or other services may hold all ten.
    async fn a_default_port_is_free() -> bool {
        for port in FIRST_DEFAULT_PORT..=LAST_DEFAULT_PORT {
            if TcpListener::bind((LOOPBACK, port)).await.is_ok() {
                return true;
            }
        }
        false
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_busy_port_falls_back_into_the_range_and_daemon_port_says_which() {
        if !a_default_port_is_free().await {
            eprintln!("skipped: every port of 47820-47829 is in use on this machine");
            return;
        }
        let paths = test_paths("fallback");
        let busy = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let options = DaemonOptions {
            port: Some(busy_port),
            ..DaemonOptions::default()
        };
        let running = start_daemon(paths.clone(), &options).await.unwrap();
        assert_ne!(running.http_port, busy_port);
        assert!((FIRST_DEFAULT_PORT..=LAST_DEFAULT_PORT).contains(&running.http_port));
        let port_text = fs::read_to_string(daemon_port_path(&paths)).unwrap();
        assert_eq!(port_text.trim(), running.http_port.to_string());
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test]
    async fn no_free_port_is_rrs_c1001() {
        let first = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let second = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let ports = [first.local_addr().unwrap().port(), second.local_addr().unwrap().port()];
        let refused = bind_tcp(LOOPBACK, &ports).await.err().unwrap();
        assert_eq!(refused.code(), "RRS-C1001");
        let fallback = bind_tcp(LOOPBACK, &[ports[0], 0]).await.unwrap();
        assert_ne!(fallback.local_addr().unwrap().port(), ports[0]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_daemon_on_the_same_folders_is_refused_and_the_lock_is_freed_by_a_clean_stop() {
        let paths = test_paths("twice");
        let running = start_test_daemon(&paths).await;
        let refused = start_daemon(paths.clone(), &options_on_any_port()).await.err().unwrap();
        assert_eq!(refused.code(), "RRS-S0001");
        stop_test_daemon(running).await;
        let running = start_test_daemon(&paths).await;
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_lock_is_kept_while_something_still_holds_the_server() {
        let paths = test_paths("stillheld");
        let running = start_test_daemon(&paths).await;
        // Stands in for a connection task that outlived the grace period and still holds the store.
        let still_held = running.server.clone();
        stop_test_daemon(running).await;
        let refused = take_daemon_lock(&paths).err().unwrap();
        assert_eq!(refused.code(), "RRS-S0001");
        drop(still_held);
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_malformed_set_flag_refuses_the_start() {
        let paths = test_paths("badflag");
        let options = DaemonOptions {
            set_flags: vec!["daemon.port".to_string()],
            ..options_on_any_port()
        };
        let refused = start_daemon(paths.clone(), &options).await.err().unwrap();
        assert_eq!(refused.code(), "RRS-C0002");
        remove_test_folder(&paths);
    }
}
