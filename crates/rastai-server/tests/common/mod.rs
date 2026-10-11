//! What the integration tests share: fresh data folders per test (removed again at its end), a daemon started on
//! them, and plain HTTP requests to it. A test that fails stops before the removal, so its folder stays behind to
//! be looked at.
//!
//! Each test binary uses only the helpers it needs, so the rest look unused to that binary.
#![allow(dead_code)]

use std::path::PathBuf;

use rastai_core::paths::DataPaths;
use rastai_server::daemon::{DaemonOptions, RunningDaemon, start_daemon};
use rastai_server::shutdown::{request_shutdown, wait_for_shutdown};
use rastai_transport::http_client::{HttpRequest, HttpResponse, HttpTarget, send_request};

/// Port 0: the operating system picks a free port, so tests running at once never fight over 47820–47829.
pub const ANY_FREE_PORT: u16 = 0;
const REQUEST_TIMEOUT_MS: u64 = 5000;

/// Empty data folders for one test, under one short parent: the socket path must stay within 107 bytes.
pub fn test_paths(test_name: &str) -> DataPaths {
    let root = test_root(test_name);
    let _ = std::fs::remove_dir_all(&root);
    DataPaths {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
        state: root.join("state"),
        runtime: root.join("runtime"),
    }
}

fn test_root(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rsv-{}-{test_name}", std::process::id()))
}

/// Removes the parent folder `test_paths` made. Call it after the daemon stopped.
pub fn remove_test_folder(paths: &DataPaths) {
    if let Some(root) = paths.data.parent() {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// Options that serve on a port the operating system picks.
pub fn options_on_any_port() -> DaemonOptions {
    DaemonOptions {
        port: Some(ANY_FREE_PORT),
        ..DaemonOptions::default()
    }
}

#[allow(clippy::unwrap_used)]
pub async fn start_test_daemon(paths: &DataPaths) -> RunningDaemon {
    start_daemon(paths.clone(), &options_on_any_port()).await.unwrap()
}

#[allow(clippy::unwrap_used)]
pub async fn stop_test_daemon(running: RunningDaemon) {
    request_shutdown(&running.server);
    wait_for_shutdown(running).await.unwrap();
}

/// One request to the daemon's loopback port. `headers` may replace Host (a rebinding test sends its own).
#[allow(clippy::unwrap_used)]
pub async fn send_to_port(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
    let request = HttpRequest {
        method: method.to_string(),
        target: HttpTarget::LoopbackHttp { port },
        path: path.to_string(),
        headers: owned_headers(headers),
        body: body.to_vec(),
        timeout_ms: REQUEST_TIMEOUT_MS,
    };
    send_request(request).await.unwrap()
}

/// One request over daemon.sock.
#[allow(clippy::unwrap_used)]
pub async fn send_to_socket(paths: &DataPaths, method: &str, path: &str, headers: &[(&str, &str)]) -> HttpResponse {
    let request = HttpRequest {
        method: method.to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: rastai_core::paths::daemon_socket_path(paths),
        },
        path: path.to_string(),
        headers: owned_headers(headers),
        body: Vec::new(),
        timeout_ms: REQUEST_TIMEOUT_MS,
    };
    send_request(request).await.unwrap()
}

fn owned_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// A response header's value, by lowercase name.
pub fn header_value<'a>(response: &'a HttpResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.as_str())
}

/// The `code` of a refusal's diagnostic body.
#[allow(clippy::unwrap_used)]
pub fn refusal_code(response: &HttpResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    body["code"].as_str().unwrap().to_string()
}
