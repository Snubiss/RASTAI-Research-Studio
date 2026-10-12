//! The request gate: one request head per branch of flows/platform.md 4.2 with ADR-0034's rules, then the same
//! rules through a running daemon over loopback TCP and daemon.sock, sign-in included (4.1).
//!
//! The table calls `check_request_gate` directly, so every branch is pinned without a network; the end-to-end
//! part proves the router puts the gate in front of every path and that `POST /api/login` sets the cookie.

mod common;

#[allow(clippy::unwrap_used)]
mod table {
    use rastai_core::principal::PrincipalKind;
    use rastai_core::scopes::Scope;
    use rastai_formats::hashing::hash_bytes;
    use rastai_server::http::gate::{Listener, RequestHead, check_request_gate, read_request_head};
    use rastai_server::http::sessions::{
        AuthState, NewSession, ScopedToken, add_scoped_token, create_login_code, new_auth_state, redeem_login_code,
        revoke_scoped_token,
    };

    const OWNER_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const COOKIE_NAME: &str = "rastai_session_0a1b2c3d";
    const DAEMON_HOST: &str = "127.0.0.1:47820";
    const DAEMON_ORIGIN: &str = "http://127.0.0.1:47820";
    /// A page on another loopback port: a second daemon through a tunnel, a dev server, a notebook.
    const OTHER_PORT_ORIGIN: &str = "http://127.0.0.1:9000";
    const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
    const BLOB_PATH: &str = "/api/blobs/blake3:0000000000000000000000000000000000000000000000000000000000000000";
    const LOGIN_PATH: &str = "/api/login";
    const HEALTH_PATH: &str = "/api/health";
    const NOW_UNIX_MS: i64 = 1_800_000_000_000;

    fn auth() -> AuthState {
        new_auth_state(hash_bytes(OWNER_TOKEN.as_bytes()), "0a1b2c3d")
    }

    /// Signs in at `origin` and returns the cookie's value and the request key.
    fn sign_in(auth: &AuthState, origin: &str) -> NewSession {
        let now = rastai_core::time::now_unix_ms();
        let code = create_login_code(auth, now).unwrap();
        redeem_login_code(auth, &code.code, origin, now).unwrap()
    }

    /// A request with only a method, path and Host; each test adds what its row needs.
    fn request(method: &str, path: &str, host: &str) -> RequestHead {
        RequestHead {
            method: method.to_string(),
            path: path.to_string(),
            host: host.to_string(),
            origin: None,
            sec_fetch_site: None,
            cookie_session_id: None,
            session_key: None,
            bearer_token: None,
        }
    }

    /// A browser's cookie request from the page at `origin` (Host follows the origin, as a browser's does).
    fn cookie_request(method: &str, path: &str, origin: &str, session: &NewSession) -> RequestHead {
        let host = origin.trim_start_matches("http://");
        RequestHead {
            origin: Some(origin.to_string()),
            cookie_session_id: Some(session.session_id.clone()),
            session_key: Some(session.session_key.clone()),
            ..request(method, path, host)
        }
    }

    /// The gate over loopback TCP: the principal's kind, or the refusal's code.
    fn tcp_gate(head: &RequestHead, auth: &AuthState) -> Result<Option<PrincipalKind>, String> {
        match check_request_gate(head, &Listener::LoopbackTcp, auth) {
            Ok(principal) => Ok(principal.map(|found| found.kind)),
            Err(refusal) => Err(refusal.code),
        }
    }

    fn refused_with(head: &RequestHead, auth: &AuthState) -> String {
        tcp_gate(head, auth).unwrap_err()
    }

    fn passes_as_owner(head: &RequestHead, auth: &AuthState) -> bool {
        tcp_gate(head, auth).unwrap() == Some(PrincipalKind::Owner)
    }

    // ---- Host: DNS rebinding ----

    #[test]
    fn a_foreign_host_is_refused_whatever_else_the_request_carries() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        let mut rebound = cookie_request("POST", EXECUTE_PATH, DAEMON_ORIGIN, &session);
        rebound.host = "attacker.test".to_string();
        rebound.origin = Some("http://attacker.test".to_string());
        assert_eq!(refused_with(&rebound, &auth), "RRS-C1003");
        let health = request("GET", HEALTH_PATH, "attacker.test:47820");
        assert_eq!(refused_with(&health, &auth), "RRS-C1003");
        let bearer = RequestHead {
            bearer_token: Some(OWNER_TOKEN.to_string()),
            ..request("POST", EXECUTE_PATH, "attacker.test")
        };
        assert_eq!(refused_with(&bearer, &auth), "RRS-C1003");
        assert_eq!(refused_with(&request("GET", HEALTH_PATH, ""), &auth), "RRS-C1003");
    }

    #[test]
    fn the_host_refusal_names_the_host() {
        let auth = auth();
        let head = request("GET", HEALTH_PATH, "attacker.test");
        let refusal = check_request_gate(&head, &Listener::LoopbackTcp, &auth).unwrap_err();
        assert!(refusal.message.contains("attacker.test"), "{}", refusal.message);
    }

    // ---- Origin ----

    #[test]
    fn origin_null_is_refused() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        let mut from_iframe = cookie_request("POST", EXECUTE_PATH, DAEMON_ORIGIN, &session);
        from_iframe.origin = Some("null".to_string());
        assert_eq!(refused_with(&from_iframe, &auth), "RRS-C1004");
        let health = RequestHead {
            origin: Some("null".to_string()),
            ..request("GET", HEALTH_PATH, DAEMON_HOST)
        };
        assert_eq!(refused_with(&health, &auth), "RRS-C1004");
    }

    #[test]
    fn an_origin_that_is_not_http_plus_host_is_refused() {
        let auth = auth();
        for origin in [
            "http://attacker.test",
            "https://127.0.0.1:47820",
            "http://localhost:47820",
            "http://127.0.0.1",
        ] {
            let head = RequestHead {
                origin: Some(origin.to_string()),
                ..request("GET", HEALTH_PATH, DAEMON_HOST)
            };
            assert_eq!(refused_with(&head, &auth), "RRS-C1004", "{origin}");
        }
    }

    // ---- Public paths and credentials ----

    #[test]
    fn public_paths_pass_with_no_credential() {
        let auth = auth();
        for path in [
            HEALTH_PATH,
            "/",
            "/assets/index.js",
            "/plugins/eval/1.0.0/ui/index.html",
        ] {
            assert_eq!(
                tcp_gate(&request("GET", path, DAEMON_HOST), &auth).unwrap(),
                None,
                "{path}"
            );
        }
    }

    #[test]
    fn no_credential_is_refused() {
        let auth = auth();
        for (method, path) in [
            ("POST", EXECUTE_PATH),
            ("GET", BLOB_PATH),
            ("POST", "/mcp"),
            ("GET", "/v1/models"),
        ] {
            let head = RequestHead {
                origin: Some(DAEMON_ORIGIN.to_string()),
                ..request(method, path, DAEMON_HOST)
            };
            assert_eq!(refused_with(&head, &auth), "RRS-C1002", "{path}");
        }
    }

    #[test]
    fn an_unknown_cookie_is_refused() {
        let auth = auth();
        let stranger = NewSession {
            session_id: "f".repeat(64),
            session_key: "e".repeat(64),
        };
        let head = cookie_request("POST", EXECUTE_PATH, DAEMON_ORIGIN, &stranger);
        assert_eq!(refused_with(&head, &auth), "RRS-C1002");
    }

    // ---- Sessions bound to their origin ----

    #[test]
    fn a_session_works_on_any_loopback_port_it_was_redeemed_on() {
        let auth = auth();
        let session = sign_in(&auth, "http://127.0.0.1:47900");
        let head = cookie_request("POST", EXECUTE_PATH, "http://127.0.0.1:47900", &session);
        assert!(passes_as_owner(&head, &auth));
    }

    #[test]
    fn a_cookie_redeemed_at_47820_is_refused_from_another_port() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        // A forwarder on port 9000: Host and Origin both say 9000, so the Host and Origin rules alone pass it.
        let forwarded = cookie_request("POST", EXECUTE_PATH, OTHER_PORT_ORIGIN, &session);
        assert_eq!(refused_with(&forwarded, &auth), "RRS-C1004");
        let other_daemon_port = cookie_request("POST", EXECUTE_PATH, "http://127.0.0.1:47900", &session);
        assert_eq!(refused_with(&other_daemon_port, &auth), "RRS-C1004");
        let get_from_other_port = cookie_request("GET", BLOB_PATH, OTHER_PORT_ORIGIN, &session);
        assert_eq!(refused_with(&get_from_other_port, &auth), "RRS-C1004");
    }

    #[test]
    fn a_tunnel_on_another_local_port_signs_in_there() {
        let auth = auth();
        let session = sign_in(&auth, OTHER_PORT_ORIGIN);
        let through_tunnel = cookie_request("POST", EXECUTE_PATH, OTHER_PORT_ORIGIN, &session);
        assert!(passes_as_owner(&through_tunnel, &auth));
        // The same Host with the daemon's own port as Origin breaks "Origin is http:// plus Host".
        let mut mismatched = through_tunnel.clone();
        mismatched.origin = Some(DAEMON_ORIGIN.to_string());
        assert_eq!(refused_with(&mismatched, &auth), "RRS-C1004");
    }

    // ---- Cookie requests without Origin ----

    #[test]
    fn a_cookie_get_without_origin_passes_only_from_the_same_origin_or_a_typed_url() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        for (site, passes) in [
            (Some("same-origin"), true),
            (Some("none"), true),
            (Some("same-site"), false),
            (Some("cross-site"), false),
            (None, false),
        ] {
            for method in ["GET", "HEAD"] {
                let mut head = cookie_request(method, BLOB_PATH, DAEMON_ORIGIN, &session);
                head.origin = None;
                head.session_key = None;
                head.sec_fetch_site = site.map(str::to_string);
                if passes {
                    assert!(passes_as_owner(&head, &auth), "{method} {site:?}");
                } else {
                    assert_eq!(refused_with(&head, &auth), "RRS-C1004", "{method} {site:?}");
                }
            }
        }
    }

    #[test]
    fn a_cookie_post_without_origin_is_refused() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        let mut head = cookie_request("POST", EXECUTE_PATH, DAEMON_ORIGIN, &session);
        head.origin = None;
        head.sec_fetch_site = Some("same-origin".to_string());
        assert_eq!(refused_with(&head, &auth), "RRS-C1004");
        head.method = "PUT".to_string();
        head.path = "/api/blobs".to_string();
        assert_eq!(refused_with(&head, &auth), "RRS-C1004");
    }

    // ---- The session's request key ----

    #[test]
    fn a_cookie_post_needs_the_session_key() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        let with_key = cookie_request("POST", EXECUTE_PATH, DAEMON_ORIGIN, &session);
        assert!(passes_as_owner(&with_key, &auth));
        let mut without_key = with_key.clone();
        without_key.session_key = None;
        assert_eq!(refused_with(&without_key, &auth), "RRS-C1002");
        let mut wrong_key = with_key.clone();
        wrong_key.session_key = Some("0".repeat(64));
        assert_eq!(refused_with(&wrong_key, &auth), "RRS-C1002");
        let mut other_sessions_key = with_key.clone();
        other_sessions_key.session_key = Some(sign_in(&auth, DAEMON_ORIGIN).session_key);
        assert_eq!(refused_with(&other_sessions_key, &auth), "RRS-C1002");
    }

    #[test]
    fn a_cookie_get_of_a_blob_needs_no_key() {
        let auth = auth();
        let session = sign_in(&auth, DAEMON_ORIGIN);
        let mut with_origin = cookie_request("GET", BLOB_PATH, DAEMON_ORIGIN, &session);
        with_origin.session_key = None;
        assert!(passes_as_owner(&with_origin, &auth));
        let mut typed_url = with_origin.clone();
        typed_url.origin = None;
        typed_url.sec_fetch_site = Some("none".to_string());
        assert!(passes_as_owner(&typed_url, &auth));
    }

    // ---- Login ----

    #[test]
    fn a_login_without_origin_is_refused_and_its_code_still_works() {
        let auth = auth();
        let code = create_login_code(&auth, NOW_UNIX_MS).unwrap();
        let without_origin = request("POST", LOGIN_PATH, DAEMON_HOST);
        assert_eq!(refused_with(&without_origin, &auth), "RRS-C1004");
        // The gate refused before anything read the code, so it is still live.
        assert!(redeem_login_code(&auth, &code.code, DAEMON_ORIGIN, NOW_UNIX_MS + 1).is_ok());
    }

    #[test]
    fn a_login_with_its_origin_passes_the_gate_with_no_principal() {
        let auth = auth();
        let login = RequestHead {
            origin: Some(DAEMON_ORIGIN.to_string()),
            ..request("POST", LOGIN_PATH, DAEMON_HOST)
        };
        assert_eq!(tcp_gate(&login, &auth).unwrap(), None);
        let mut foreign = login.clone();
        foreign.origin = Some(OTHER_PORT_ORIGIN.to_string());
        assert_eq!(refused_with(&foreign, &auth), "RRS-C1004");
    }

    // ---- Bearer tokens ----

    #[test]
    fn the_owner_token_works_without_origin_and_with_the_right_one() {
        let auth = auth();
        let bearer = RequestHead {
            bearer_token: Some(OWNER_TOKEN.to_string()),
            ..request("POST", EXECUTE_PATH, DAEMON_HOST)
        };
        assert!(passes_as_owner(&bearer, &auth));
        let mut with_origin = bearer.clone();
        with_origin.origin = Some(DAEMON_ORIGIN.to_string());
        assert!(passes_as_owner(&with_origin, &auth));
        let mut foreign_origin = bearer.clone();
        foreign_origin.origin = Some("http://attacker.test".to_string());
        assert_eq!(refused_with(&foreign_origin, &auth), "RRS-C1004");
    }

    #[test]
    fn an_unknown_bearer_token_is_refused() {
        let auth = auth();
        let bearer = RequestHead {
            bearer_token: Some("tok_not_issued".to_string()),
            ..request("POST", EXECUTE_PATH, DAEMON_HOST)
        };
        assert_eq!(refused_with(&bearer, &auth), "RRS-C1002");
    }

    #[test]
    fn a_scoped_token_carries_its_own_scopes_until_it_is_revoked() {
        let auth = auth();
        add_scoped_token(
            &auth,
            ScopedToken {
                token_id: "tok_01J".to_string(),
                token_hash: hash_bytes(b"scoped-token-text"),
                scopes: vec![Scope::Chat],
                revoked: false,
            },
        );
        let bearer = RequestHead {
            bearer_token: Some("scoped-token-text".to_string()),
            ..request("POST", EXECUTE_PATH, DAEMON_HOST)
        };
        let principal = check_request_gate(&bearer, &Listener::LoopbackTcp, &auth)
            .unwrap()
            .unwrap();
        assert_eq!(principal.kind, PrincipalKind::Token);
        assert_eq!(principal.id, "tok_01J");
        assert_eq!(principal.scopes, vec![Scope::Chat]);
        // The Host and Origin rules still come first.
        let rebinding = RequestHead {
            host: "attacker.test".to_string(),
            ..bearer.clone()
        };
        assert_eq!(refused_with(&rebinding, &auth), "RRS-C1003");
        revoke_scoped_token(&auth, "tok_01J");
        assert_eq!(refused_with(&bearer, &auth), "RRS-C1006");
    }

    // ---- daemon.sock ----

    #[tokio::test]
    async fn the_socket_owner_passes_and_another_user_is_refused() {
        let auth = auth();
        let own_user_id = own_user_id();
        // Over daemon.sock the peer's user is the credential; Host and Origin don't apply.
        let head = request("POST", EXECUTE_PATH, "attacker.test");
        let owner = check_request_gate(&head, &Listener::DaemonSocket { peer_uid: own_user_id }, &auth).unwrap();
        assert_eq!(owner.map(|principal| principal.kind), Some(PrincipalKind::Owner));
        let another = Listener::DaemonSocket {
            peer_uid: own_user_id.wrapping_add(1),
        };
        assert_eq!(
            check_request_gate(&head, &another, &auth).unwrap_err().code,
            "RRS-C1002"
        );
    }

    /// Our own user id, read the way rastai-transport reads it: the peer of a socket pair we hold both ends of.
    #[cfg(unix)]
    fn own_user_id() -> u32 {
        let (one_end, _other_end) = tokio::net::UnixStream::pair().unwrap();
        one_end.peer_cred().unwrap().uid()
    }

    #[cfg(not(unix))]
    fn own_user_id() -> u32 {
        0
    }

    // ---- Reading the headers ----

    /// The head `read_request_head` reads, or the refusal's code.
    fn read(headers: &[(&str, &[u8])]) -> Result<RequestHead, String> {
        read_request_head("POST", EXECUTE_PATH, headers, COOKIE_NAME).map_err(|refusal| refusal.code)
    }

    #[test]
    fn the_headers_are_read_into_the_head() {
        let cookie = format!("theme=dark; {COOKIE_NAME}=abc; rastai_session_ffffffff=other");
        let headers: Vec<(&str, &[u8])> = vec![
            ("host", DAEMON_HOST.as_bytes()),
            ("origin", DAEMON_ORIGIN.as_bytes()),
            ("sec-fetch-site", b"same-origin"),
            ("cookie", cookie.as_bytes()),
            ("rastai-session-key", b"key"),
            ("authorization", b"Bearer token"),
        ];
        let head = read(&headers).unwrap();
        assert_eq!(head.host, DAEMON_HOST);
        assert_eq!(head.origin.as_deref(), Some(DAEMON_ORIGIN));
        assert_eq!(head.sec_fetch_site.as_deref(), Some("same-origin"));
        assert_eq!(head.cookie_session_id.as_deref(), Some("abc"));
        assert_eq!(head.session_key.as_deref(), Some("key"));
        assert_eq!(head.bearer_token.as_deref(), Some("token"));
    }

    #[test]
    fn a_header_sent_twice_is_refused_with_its_rule() {
        let host = DAEMON_HOST.as_bytes();
        let two_hosts: Vec<(&str, &[u8])> = vec![("host", host), ("host", b"attacker.test")];
        assert_eq!(read(&two_hosts).unwrap_err(), "RRS-C1003");
        let two_origins: Vec<(&str, &[u8])> = vec![("host", host), ("origin", b"a"), ("origin", b"b")];
        assert_eq!(read(&two_origins).unwrap_err(), "RRS-C1004");
        let two_sites: Vec<(&str, &[u8])> =
            vec![("host", host), ("sec-fetch-site", b"none"), ("sec-fetch-site", b"none")];
        assert_eq!(read(&two_sites).unwrap_err(), "RRS-C1004");
        let two_keys: Vec<(&str, &[u8])> = vec![
            ("host", host),
            ("rastai-session-key", b"a"),
            ("rastai-session-key", b"a"),
        ];
        assert_eq!(read(&two_keys).unwrap_err(), "RRS-C1002");
        let two_tokens: Vec<(&str, &[u8])> = vec![
            ("host", host),
            ("authorization", b"Bearer a"),
            ("authorization", b"Bearer b"),
        ];
        assert_eq!(read(&two_tokens).unwrap_err(), "RRS-C1002");
    }

    #[test]
    fn a_header_that_is_not_visible_text_is_refused() {
        let control: Vec<(&str, &[u8])> = vec![("host", b"127.0.0.1:47820\x00")];
        assert_eq!(read(&control).unwrap_err(), "RRS-C1003");
        let not_ascii: Vec<(&str, &[u8])> = vec![
            ("host", DAEMON_HOST.as_bytes()),
            ("origin", "http://127.0.0.1:47820é".as_bytes()),
        ];
        assert_eq!(read(&not_ascii).unwrap_err(), "RRS-C1004");
    }

    #[test]
    fn two_different_session_cookies_are_refused_and_a_repeat_is_not() {
        let host = DAEMON_HOST.as_bytes();
        let tossed = format!("{COOKIE_NAME}=planted; {COOKIE_NAME}=real");
        let headers: Vec<(&str, &[u8])> = vec![("host", host), ("cookie", tossed.as_bytes())];
        assert_eq!(read(&headers).unwrap_err(), "RRS-C1002");
        let first = format!("{COOKIE_NAME}=same");
        let repeated: Vec<(&str, &[u8])> = vec![
            ("host", host),
            ("cookie", first.as_bytes()),
            ("cookie", first.as_bytes()),
        ];
        assert_eq!(read(&repeated).unwrap().cookie_session_id.as_deref(), Some("same"));
    }

    #[test]
    fn other_cookies_and_other_schemes_are_no_credential() {
        let headers: Vec<(&str, &[u8])> = vec![
            ("host", DAEMON_HOST.as_bytes()),
            ("cookie", b"rastai_session_ffffffff=x; junk\xff=\xfe"),
            ("authorization", b"Basic dXNlcjpwYXNz"),
        ];
        let head = read(&headers).unwrap();
        assert_eq!(head.cookie_session_id, None);
        assert_eq!(head.bearer_token, None);
    }
}

/// The same rules through a running daemon: the router must put the gate in front of every path.
#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod end_to_end {
    use rastai_server::daemon::RunningDaemon;
    use rastai_server::http::sessions::create_login_code;
    use rastai_transport::http_client::HttpResponse;

    use crate::common::{
        header_value, refusal_code, remove_test_folder, send_to_port, send_to_socket, start_test_daemon,
        stop_test_daemon, test_paths,
    };

    const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
    const BLOB_PATH: &str = "/api/blobs/blake3:00";
    /// A command any credential may run: what a request that passed the gate sends.
    const STATUS_REQUEST: &[u8] = br#"{"command_id": "system.status"}"#;

    /// The gate let the request through and the command ran: HTTP 200 with `ok: true`.
    fn assert_executed(response: HttpResponse) {
        assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["ok"], true, "{body}");
    }

    fn assert_refused(response: HttpResponse, status: u16, code: &str) {
        assert_eq!((response.status, refusal_code(&response).as_str()), (status, code));
    }

    fn daemon_host(port: u16) -> String {
        format!("127.0.0.1:{port}")
    }

    fn daemon_origin(port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn login_body(code: &str) -> Vec<u8> {
        format!("{{\"code\": \"{code}\"}}").into_bytes()
    }

    /// The cookie pair (`name=value`) from a login answer's Set-Cookie.
    fn cookie_pair(set_cookie: &str) -> String {
        set_cookie.split(';').next().unwrap().to_string()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_answers_on_both_listeners_with_the_versions() {
        let paths = test_paths("health");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let over_tcp = send_to_port(port, "GET", "/api/health", &[], b"").await;
        assert_eq!(over_tcp.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&over_tcp.body).unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(
            body["apis"],
            serde_json::json!(["rastai.commands.v1", "rastai.events.v1"])
        );
        assert_eq!(body["node_protocol"], 1);
        assert!(body["plugin_api"].is_string() && body["version"].is_string());
        let over_socket = send_to_socket(&paths, "GET", "/api/health", &[], b"").await;
        assert_eq!(over_socket.status, 200);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dns_rebinding_is_refused_on_every_path() {
        let paths = test_paths("rebind");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        for (method, path) in [
            ("GET", "/api/health"),
            ("POST", "/api/login"),
            ("POST", EXECUTE_PATH),
            ("GET", "/x"),
        ] {
            let response = send_to_port(port, method, path, &[("Host", "attacker.test")], b"").await;
            assert_eq!(response.status, 403, "{path}");
            assert_eq!(refusal_code(&response), "RRS-C1003", "{path}");
        }
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    /// Signs in over HTTP from the daemon's own origin; returns the cookie pair (`name=value`) and the request key.
    async fn sign_in_over_http(running: &RunningDaemon) -> (String, String) {
        let port = running.http_port;
        let code = create_login_code(&running.server.auth, rastai_core::time::now_unix_ms()).unwrap();
        let origin = daemon_origin(port);
        let login = send_to_port(
            port,
            "POST",
            "/api/login",
            &[("Origin", &origin)],
            &login_body(&code.code),
        )
        .await;
        assert_eq!(login.status, 200);
        let set_cookie = header_value(&login, "set-cookie").unwrap();
        let body: serde_json::Value = serde_json::from_slice(&login.body).unwrap();
        (
            cookie_pair(set_cookie),
            body["session_key"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sign_in_sets_an_http_only_strict_cookie_and_returns_the_key() {
        let paths = test_paths("signin");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let code = create_login_code(&running.server.auth, rastai_core::time::now_unix_ms()).unwrap();
        let origin = daemon_origin(port);
        let login = send_to_port(
            port,
            "POST",
            "/api/login",
            &[("Origin", &origin)],
            &login_body(&code.code),
        )
        .await;
        assert_eq!(login.status, 200);
        let set_cookie = header_value(&login, "set-cookie").unwrap();
        assert!(set_cookie.starts_with("rastai_session_"), "{set_cookie}");
        assert!(
            set_cookie.ends_with("; HttpOnly; SameSite=Strict; Path=/"),
            "{set_cookie}"
        );
        assert_eq!(header_value(&login, "cache-control"), Some("no-store"));
        let body: serde_json::Value = serde_json::from_slice(&login.body).unwrap();
        let key = body["session_key"].as_str().unwrap();
        assert_eq!(key.len(), 64);
        assert!(
            key.chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cookie_works_only_from_its_origin_and_with_its_key() {
        let paths = test_paths("cookie");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let origin = daemon_origin(port);
        let (cookie, key) = sign_in_over_http(&running).await;
        // Passing the gate reaches Execute, which runs the command.
        let with_key = [
            ("Origin", origin.as_str()),
            ("Cookie", cookie.as_str()),
            ("Rastai-Session-Key", key.as_str()),
            ("Content-Type", "application/json"),
        ];
        assert_executed(send_to_port(port, "POST", EXECUTE_PATH, &with_key, STATUS_REQUEST).await);
        let without_key = [
            ("Origin", origin.as_str()),
            ("Cookie", cookie.as_str()),
            ("Content-Type", "application/json"),
        ];
        assert_refused(
            send_to_port(port, "POST", EXECUTE_PATH, &without_key, STATUS_REQUEST).await,
            401,
            "RRS-C1002",
        );
        let without_origin = [("Cookie", cookie.as_str()), ("Rastai-Session-Key", key.as_str())];
        assert_refused(
            send_to_port(port, "POST", EXECUTE_PATH, &without_origin, b"{}").await,
            403,
            "RRS-C1004",
        );
        // The same cookie forwarded from a page on port 9000: Host and Origin both say 9000.
        let forwarded = [
            ("Host", "127.0.0.1:9000"),
            ("Origin", "http://127.0.0.1:9000"),
            ("Cookie", cookie.as_str()),
            ("Rastai-Session-Key", key.as_str()),
        ];
        assert_refused(
            send_to_port(port, "POST", EXECUTE_PATH, &forwarded, b"{}").await,
            403,
            "RRS-C1004",
        );
        // A blob GET from the app itself, with no Origin and no key; the same from an <img> on another port.
        let blob = [("Cookie", cookie.as_str()), ("Sec-Fetch-Site", "same-origin")];
        assert_eq!(send_to_port(port, "GET", BLOB_PATH, &blob, b"").await.status, 404);
        let img_from_other_port = [("Cookie", cookie.as_str()), ("Sec-Fetch-Site", "same-site")];
        assert_refused(
            send_to_port(port, "GET", BLOB_PATH, &img_from_other_port, b"").await,
            403,
            "RRS-C1004",
        );
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_login_without_origin_is_refused_and_the_code_still_works() {
        let paths = test_paths("loginorigin");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let code = create_login_code(&running.server.auth, rastai_core::time::now_unix_ms()).unwrap();
        let body = login_body(&code.code);

        let refused = send_to_port(port, "POST", "/api/login", &[], &body).await;
        assert_eq!((refused.status, refusal_code(&refused)), (403, "RRS-C1004".to_string()));
        assert_eq!(header_value(&refused, "set-cookie"), None);
        let foreign = [("Origin", "http://127.0.0.1:9000")];
        let refused = send_to_port(port, "POST", "/api/login", &foreign, &body).await;
        assert_eq!((refused.status, refusal_code(&refused)), (403, "RRS-C1004".to_string()));

        let origin = daemon_origin(port);
        let login = send_to_port(port, "POST", "/api/login", &[("Origin", &origin)], &body).await;
        assert_eq!(login.status, 200);
        let reused = send_to_port(port, "POST", "/api/login", &[("Origin", &origin)], &body).await;
        assert_eq!((reused.status, refusal_code(&reused)), (401, "RRS-C1007".to_string()));
        let malformed = send_to_port(port, "POST", "/api/login", &[("Origin", &origin)], b"code=1").await;
        assert_eq!(
            (malformed.status, refusal_code(&malformed)),
            (400, "RRS-C0002".to_string())
        );

        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_owner_token_and_the_socket_peer_are_credentials() {
        let paths = test_paths("bearer");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        let token = std::fs::read_to_string(rastai_core::paths::owner_token_path(&paths)).unwrap();
        let bearer = format!("Bearer {token}");
        let with_token = [("Authorization", bearer.as_str()), ("Content-Type", "application/json")];
        assert_executed(send_to_port(port, "POST", EXECUTE_PATH, &with_token, STATUS_REQUEST).await);
        let wrong = send_to_port(port, "POST", EXECUTE_PATH, &[("Authorization", "Bearer nope")], b"{}").await;
        assert_eq!((wrong.status, refusal_code(&wrong)), (401, "RRS-C1002".to_string()));
        let nothing = send_to_port(port, "POST", EXECUTE_PATH, &[], b"{}").await;
        assert_eq!((nothing.status, refusal_code(&nothing)), (401, "RRS-C1002".to_string()));
        // Over daemon.sock the peer's user is the credential, whatever Host says.
        let socket_headers = [("Host", "attacker.test"), ("Content-Type", "application/json")];
        assert_executed(send_to_socket(&paths, "POST", EXECUTE_PATH, &socket_headers, STATUS_REQUEST).await);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_rule_accepts_every_loopback_name() {
        let paths = test_paths("loopnames");
        let running = start_test_daemon(&paths).await;
        let port = running.http_port;
        for host in [
            daemon_host(port),
            format!("localhost:{port}"),
            format!("[::1]:{port}"),
            "127.0.0.1:47900".to_string(),
        ] {
            let response = send_to_port(port, "GET", "/api/health", &[("Host", &host)], b"").await;
            assert_eq!(response.status, 200, "{host}");
        }
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }
}
