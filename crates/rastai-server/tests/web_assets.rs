//! The web app over a running daemon (layout rust-control-plane.md 8.1; frontend 8.8): "/" and "/index.html" are the
//! index page, `no-store`, with the content security policy; hashed assets are `immutable`; anything the build doesn't
//! embed is not found. These run against whatever build.rs embedded, the real app or its placeholder.

mod common;

#[allow(clippy::unwrap_used)]
mod serving {
    use rastai_server::http::web_assets::{CONTENT_SECURITY_POLICY, WEB_ASSETS};

    use crate::common::{remove_test_folder, send_to_port, start_test_daemon, stop_test_daemon, test_paths};

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[tokio::test]
    async fn the_root_serves_the_index_page_with_no_store_and_the_policy() {
        let paths = test_paths("web_root");
        let running = start_test_daemon(&paths).await;
        let response = send_to_port(running.http_port, "GET", "/", &[], b"").await;
        assert_eq!(response.status, 200);
        assert_eq!(
            header(&response.headers, "content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(header(&response.headers, "cache-control"), Some("no-store"));
        assert_eq!(
            header(&response.headers, "content-security-policy"),
            Some(CONTENT_SECURITY_POLICY)
        );
        assert_eq!(header(&response.headers, "x-content-type-options"), Some("nosniff"));
        assert!(
            String::from_utf8_lossy(&response.body)
                .to_lowercase()
                .starts_with("<!doctype html>")
        );
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test]
    async fn the_index_page_by_its_own_name_is_the_same_page() {
        let paths = test_paths("web_index");
        let running = start_test_daemon(&paths).await;
        let root = send_to_port(running.http_port, "GET", "/", &[], b"").await;
        let named = send_to_port(running.http_port, "GET", "/index.html", &[], b"").await;
        assert_eq!(named.status, 200);
        assert_eq!(named.body, root.body);
        assert_eq!(header(&named.headers, "cache-control"), Some("no-store"));
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test]
    async fn every_hashed_asset_is_immutable_and_every_other_file_is_not_cached() {
        let paths = test_paths("web_assets");
        let running = start_test_daemon(&paths).await;
        for asset in WEB_ASSETS {
            let response = send_to_port(running.http_port, "GET", asset.path, &[], b"").await;
            assert_eq!(response.status, 200, "{}", asset.path);
            let expected = if asset.path.starts_with("/assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-store"
            };
            assert_eq!(
                header(&response.headers, "cache-control"),
                Some(expected),
                "{}",
                asset.path
            );
        }
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test]
    async fn a_file_the_build_doesnt_embed_is_not_found_and_a_post_is_not_served() {
        let paths = test_paths("web_missing");
        let running = start_test_daemon(&paths).await;
        let missing = send_to_port(running.http_port, "GET", "/assets/no-such-file.js", &[], b"").await;
        assert_eq!(missing.status, 404);
        let post = send_to_port(running.http_port, "POST", "/", &[], b"x").await;
        assert_eq!(post.status, 404);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test]
    async fn a_head_request_gets_the_headers_and_no_body() {
        let paths = test_paths("web_head");
        let running = start_test_daemon(&paths).await;
        let response = send_to_port(running.http_port, "HEAD", "/", &[], b"").await;
        assert_eq!(response.status, 200);
        assert_eq!(header(&response.headers, "cache-control"), Some("no-store"));
        assert!(response.body.is_empty());
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }
}
