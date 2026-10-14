//! What the integration tests share: fresh data folders per test (removed again at its end), a daemon started on
//! them, and plain HTTP requests to it. A test that fails stops before the removal, so its folder stays behind to
//! be looked at.
//!
//! Each test binary uses only the helpers it needs, so the rest look unused to that binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use rastai_core::paths::DataPaths;
use rastai_core::principal::{Principal, PrincipalKind, owner_principal};
use rastai_core::scopes::{Scope, scope_name};
use rastai_node::jobs::job_file::{JOB_FILE, read_job_file, recorded_process};
use rastai_node::jobs::signals::is_same_process;
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use rastai_server::commands::execute::execute_command;
use rastai_server::daemon::{DaemonOptions, RunningDaemon, start_daemon};
use rastai_server::error::ServerError;
use rastai_server::nodes::node_links::link_telemetry;
use rastai_server::server_state::Server;
use rastai_server::shutdown::{request_shutdown, wait_for_shutdown};
use rastai_store::error::StoreError;
use rastai_transport::http_client::{HttpRequest, HttpResponse, HttpTarget, send_request};

/// The procps tool that signals a process group, as rastai-node's signals.rs does (no `unsafe`, no libc).
const KILL_TOOL: &str = "kill";
const KILL_SIGNAL: &str = "KILL";

/// Port 0: the operating system picks a free port, so tests running at once never fight over 47820–47829.
pub const ANY_FREE_PORT: u16 = 0;
const REQUEST_TIMEOUT_MS: u64 = 5000;
/// Keeps the embedded node's heartbeats to the ones a change asks for, the first of which comes with the handshake:
/// a test that counts bus events then sees only its own once that first beat is in (`start_quiet_test_daemon`).
pub const QUIET_NODE_FLAG: &str = "nodes.heartbeat_seconds=3600";
/// How long `start_quiet_test_daemon` waits for the handshake's heartbeat.
const FIRST_HEARTBEAT_WAIT_MS: u64 = 10_000;
const FIRST_HEARTBEAT_POLL_MS: u64 = 20;
/// How long a test's start waits for a lock its own process released a moment ago (start_daemon_in_test). A child can
/// hold the lock file that long on a machine at load 100; the wait only runs when the lock is still held.
const LOCK_RELEASE_WAIT_MS: u64 = 10_000;
const LOCK_RELEASE_POLL_MS: u64 = 10;

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

/// Removes the parent folder `test_paths` made. Call it after the daemon stopped. Jobs the test left running are
/// killed first: the folder's job.json files are the only record of their groups, and removing them would lose it.
pub fn remove_test_folder(paths: &DataPaths) {
    kill_job_groups_under(&paths.data);
    if let Some(root) = paths.data.parent() {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// Kills every job group recorded under a test daemon's data folder when dropped, whatever the test left running:
/// `stop_test_daemon` leaves jobs running on purpose, and a test that fails before ending its job would leave its
/// worker behind (issue #244). Hold one in every test that starts jobs.
pub struct JobGroupGuard {
    data_folder: PathBuf,
}

impl JobGroupGuard {
    pub fn for_paths(paths: &DataPaths) -> JobGroupGuard {
        JobGroupGuard {
            data_folder: paths.data.clone(),
        }
    }
}

impl Drop for JobGroupGuard {
    fn drop(&mut self) {
        kill_job_groups_under(&self.data_folder);
    }
}

/// SIGKILLs the process group of every job.json under `folder` whose leader is still the recorded process, checked
/// by identity first, as the node does before a group signal. A group already gone is skipped. The same check as the
/// node tests' helper: rastai-node's test code isn't shared with this crate's tests.
pub fn kill_job_groups_under(folder: &Path) {
    for job_file_path in job_files_under(folder) {
        let Ok(job_file) = read_job_file(&job_file_path) else {
            continue;
        };
        let Some(leader) = recorded_process(&job_file) else {
            continue;
        };
        if is_same_process(&leader) {
            let group_target = format!("-{}", leader.process_group_id);
            let _ = std::process::Command::new(KILL_TOOL)
                .args(["-s", KILL_SIGNAL, "--", &group_target])
                .status();
        }
    }
}

/// Every job.json under `folder`, at any depth.
fn job_files_under(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders_to_read = vec![folder.to_path_buf()];
    while let Some(next_folder) = folders_to_read.pop() {
        let Ok(entries) = std::fs::read_dir(&next_folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders_to_read.push(path);
            } else if path.file_name().is_some_and(|name| name == JOB_FILE) {
                found.push(path);
            }
        }
    }
    found
}

/// Options that serve on a port the operating system picks.
pub fn options_on_any_port() -> DaemonOptions {
    DaemonOptions {
        port: Some(ANY_FREE_PORT),
        ..DaemonOptions::default()
    }
}

pub async fn start_test_daemon(paths: &DataPaths) -> RunningDaemon {
    start_daemon_in_test(paths, &options_on_any_port()).await
}

/// `start_daemon` for a test, which may start a daemon again on the same folders in the same process. The lock a
/// stopped daemon released can stay held for an instant by a child process another test's embedded node was just
/// starting (between its fork and its exec it holds every open file of this process), so a lock held by this very
/// process is tried again for up to `LOCK_RELEASE_WAIT_MS`. Any other failure fails the test.
pub async fn start_daemon_in_test(paths: &DataPaths, options: &DaemonOptions) -> RunningDaemon {
    match start_once_the_lock_is_free(paths, options).await {
        Ok(running) => running,
        Err(error) => panic!("the test daemon didn't start: {} {error:?}", error.code()),
    }
}

/// The start of a test that expects a refusal (`RRS-C10xx`), on folders an earlier start of the same test released or
/// on fresh ones. It waits out the lock the same way `start_daemon_in_test` does, so the refusal it checks is the one
/// the start reached, not the lock's.
pub async fn refused_start_in_test(paths: &DataPaths, options: &DaemonOptions) -> ServerError {
    match start_once_the_lock_is_free(paths, options).await {
        Ok(running) => {
            stop_test_daemon(running).await;
            panic!("the start was accepted, but a refusal was expected");
        }
        Err(error) => error,
    }
}

/// `start_daemon`, tried again while the only thing holding the lock is this process (see `start_daemon_in_test`).
/// Another holder, another daemon for instance, is returned at once.
async fn start_once_the_lock_is_free(paths: &DataPaths, options: &DaemonOptions) -> Result<RunningDaemon, ServerError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(LOCK_RELEASE_WAIT_MS);
    loop {
        let error = match start_daemon(paths.clone(), options).await {
            Ok(running) => return Ok(running),
            Err(error) => error,
        };
        let held_here = matches!(
            &error,
            ServerError::Store(StoreError::DaemonLockHeld { holder_pid: Some(pid), .. }) if *pid == std::process::id()
        );
        if !held_here || std::time::Instant::now() > deadline {
            return Err(error);
        }
        tokio::time::sleep(std::time::Duration::from_millis(LOCK_RELEASE_POLL_MS)).await;
    }
}

/// A daemon whose embedded node beats only on a change, started with `extra_flags` too, returned once the
/// handshake's heartbeat has arrived: from then on nothing publishes on the bus but the test itself.
#[allow(clippy::unwrap_used)]
pub async fn start_quiet_test_daemon(paths: &DataPaths, extra_flags: &[&str]) -> RunningDaemon {
    let mut set_flags = vec![QUIET_NODE_FLAG.to_string()];
    set_flags.extend(extra_flags.iter().map(|flag| flag.to_string()));
    let options = DaemonOptions {
        set_flags,
        ..options_on_any_port()
    };
    let running = start_daemon_in_test(paths, &options).await;
    let node_id = running.embedded_node.node_id.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(FIRST_HEARTBEAT_WAIT_MS);
    while link_telemetry(&running.server, &node_id).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the handshake's heartbeat never came"
        );
        tokio::time::sleep(std::time::Duration::from_millis(FIRST_HEARTBEAT_POLL_MS)).await;
    }
    running
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
        egress: None,
    };
    send_request(request).await.unwrap()
}

/// One request over daemon.sock.
#[allow(clippy::unwrap_used)]
pub async fn send_to_socket(
    paths: &DataPaths,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let request = HttpRequest {
        method: method.to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: rastai_core::paths::daemon_socket_path(paths),
        },
        path: path.to_string(),
        headers: owned_headers(headers),
        body: body.to_vec(),
        timeout_ms: REQUEST_TIMEOUT_MS,
        egress: None,
    };
    send_request(request).await.unwrap()
}

fn owned_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// `Authorization: Bearer <owner token>`, read from the test daemon's `owner.token`.
#[allow(clippy::unwrap_used)]
pub fn owner_bearer(paths: &DataPaths) -> String {
    let token = std::fs::read_to_string(rastai_core::paths::owner_token_path(paths)).unwrap();
    format!("Bearer {}", token.trim())
}

/// An `ExecuteRequest` for a command and its arguments, everything else at its default.
pub fn execute_request(command_id: &str, arguments_json: &str) -> ExecuteRequest {
    ExecuteRequest {
        command_id: command_id.to_string(),
        arguments_json: arguments_json.to_string(),
        ..ExecuteRequest::default()
    }
}

/// Runs a command in-process, the way every caller ends (`execute_command`).
pub async fn run_command(server: &Server, principal: &Principal, request: ExecuteRequest) -> ExecuteResponse {
    execute_command(server, principal, request).await
}

/// A scoped token's principal holding exactly these scopes.
pub fn token_principal(scopes: Vec<Scope>) -> Principal {
    Principal {
        kind: PrincipalKind::Token,
        id: "tok_01JA2B3C4D5E6F7G8H9J0K1M2N".to_string(),
        scopes,
    }
}

/// A scoped token the owner made with these scopes, as the gate would admit it. `token.create` checks a `token:`
/// creator against `rastai.db` (contracts 8.2), so a principal with a made-up id can't make tokens; this one can.
#[allow(clippy::unwrap_used)]
pub async fn created_token_principal(server: &Server, scopes: Vec<Scope>) -> Principal {
    let scope_names: Vec<String> = scopes.iter().map(scope_name).collect();
    let arguments = serde_json::json!({"name": "made-by-the-owner", "scopes": scope_names});
    let request = execute_request("token.create", &arguments.to_string());
    let created = ok_result(&run_command(server, &owner_principal(), request).await);
    Principal {
        kind: PrincipalKind::Token,
        id: created["token_id"].as_str().unwrap().to_string(),
        scopes,
    }
}

/// The codes of a response's diagnostics, in order.
pub fn response_codes(response: &ExecuteResponse) -> Vec<String> {
    response.diagnostics.iter().map(|item| item.code.clone()).collect()
}

/// The result of a response that must have succeeded, as JSON.
#[allow(clippy::unwrap_used)]
pub fn ok_result(response: &ExecuteResponse) -> serde_json::Value {
    assert!(response.ok, "{:?}", response.diagnostics);
    serde_json::from_str(&response.result_json).unwrap()
}

/// A response header's value, by lowercase name.
pub fn header_value<'a>(response: &'a HttpResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.as_str())
}

/// The catalog code of a refusal: the diagnostic body itself on `/api/` paths, the one detail of the Connect error
/// envelope (`{"code": "unauthenticated", "details": [{"debug": <diagnostic>}]}`) on Connect paths.
#[allow(clippy::unwrap_used)]
pub fn refusal_code(response: &HttpResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    match body["details"][0]["debug"]["code"].as_str() {
        Some(code) => code.to_string(),
        None => body["code"].as_str().unwrap().to_string(),
    }
}
