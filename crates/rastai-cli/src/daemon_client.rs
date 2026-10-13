//! The Connect client over daemon.sock (layout: rust-control-plane.md 9.1, 9.3 step 1): `/api/health`, starting a
//! daemon when none answers, and the binary Connect calls (`application/proto`; `application/connect+proto` for the
//! Subscribe stream, layout 11.3). Over daemon.sock the daemon knows the CLI as the owner by its peer credentials,
//! so no token is ever read or sent (layout 9.7).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use prost::Message;
use rastai_core::paths::{DataPaths, daemon_socket_path, read_path_environment, resolve_paths};
use rastai_core::versions::{API_PACKAGES, APP_VERSION};
use rastai_diag::catalog::find_entry;
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic};
use rastai_proto::commands::v1::{
    CommandDescriptor, ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse,
};
use rastai_proto::events::v1::SubscribeRequest;
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{HttpRequest, HttpStream, HttpTarget, open_response_stream, send_request};
use serde_json::Value;

use crate::error::CliError;
use crate::exit_codes::{EXIT_NOT_PERMITTED, exit_code_for_failure};

const HEALTH_PATH: &str = "/api/health";
const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
const LIST_COMMANDS_PATH: &str = "/rastai.commands.v1.CommandService/ListCommands";
const SUBSCRIBE_PATH: &str = "/rastai.events.v1.EventService/Subscribe";
const GET_METHOD: &str = "GET";
const POST_METHOD: &str = "POST";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const PROTO_CONTENT_TYPE: &str = "application/proto";
const STREAM_CONTENT_TYPE: &str = "application/connect+proto";
/// Every request names its client, which the daemon logs (contracts 15.4).
const CLIENT_HEADER: &str = "Rastai-Client";
const CLIENT_NAME: &str = "rastai-cli";
const HTTP_OK: u16 = 200;

/// A daemon that answers at all answers health at once.
const HEALTH_TIMEOUT_MS: u64 = 2_000;
/// A sync command runs inside the call, a job command returns its job id at once. Five minutes covers the slowest
/// sync command, as for the MCP bridge.
const REQUEST_TIMEOUT_MS: u64 = 300_000;
/// How long a started daemon has to answer health (contracts 15.10, flows/platform.md 2.1).
const DAEMON_LOCK_WAIT_SECONDS: u64 = 10;
const HEALTH_POLL_MS: u64 = 100;
/// The daemon a verb starts: headless (no browser for a CLI call), and `--json` so its final line says why it
/// stopped if it can't start.
const SERVE_ARGUMENTS: &[&str] = &["serve", "--headless", "--json"];

// The stream envelope (connectrpc.com/docs/protocol): one flags byte, then the length as a big-endian u32.
const MESSAGE_FLAGS: u8 = 0;
const ENVELOPE_HEADER_BYTES: usize = 5;

const VERSION_SKEW_CODE: &str = "RRS-C0006";
const NOT_LISTENING_CODE: &str = "RRS-N2004";
const NOT_SUPPORTED_PLATFORM_CODE: &str = "RRS-N2009";
const DIAGNOSTICS_KEY: &str = "diagnostics";
const DETAILS_KEY: &str = "details";
/// Each Connect error detail carries the diagnostic as JSON under this key (rastai-server http/connect.rs).
const DEBUG_KEY: &str = "debug";

/// A daemon that answered health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonClient {
    pub socket_path: PathBuf,
    /// The daemon's `version` from health.
    pub daemon_version: String,
    /// Problems that don't stop the call, such as a daemon of another version (`RRS-C0006`).
    pub warnings: Vec<Diagnostic>,
}

/// The fields of `/api/health` the CLI reads (layout 11.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonHealth {
    pub version: String,
    pub apis: Vec<String>,
}

/// This user's data folders, from the environment (`RASTAI_HOME`, the XDG variables).
pub fn data_paths() -> Result<DataPaths, CliError> {
    match read_path_environment() {
        Ok(environment) => Ok(resolve_paths(&environment)),
        Err(error) => Err(CliError::Failed(vec![diagnostic(error.code(), &[])])),
    }
}

/// The daemon of these data folders, started first when none answers (unless `no_start`). A daemon of another
/// version is used with warning `RRS-C0006`; one that lacks an API package this CLI calls is refused with it.
pub async fn ensure_daemon_running(paths: &DataPaths, no_start: bool) -> Result<DaemonClient, CliError> {
    let socket_path = daemon_socket_path(paths);
    let health = match read_health(&socket_path).await {
        Ok(health) => health,
        Err(error) if no_start => {
            return Err(CliError::DaemonUnavailable(vec![transport_diagnostic(
                &error,
                &socket_path,
            )]));
        }
        Err(_) => start_daemon(&socket_path).await?,
    };
    client_for(socket_path, health)
}

/// `GET /api/health`.
pub async fn read_health(socket_path: &Path) -> Result<DaemonHealth, TransportError> {
    let request = HttpRequest {
        method: GET_METHOD.to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: socket_path.to_path_buf(),
        },
        path: HEALTH_PATH.to_string(),
        headers: vec![client_header()],
        body: Vec::new(),
        timeout_ms: HEALTH_TIMEOUT_MS,
    };
    let response = send_request(request).await?;
    let body: Value = serde_json::from_slice(&response.body).unwrap_or_default();
    let apis = body["apis"].as_array().cloned().unwrap_or_default();
    Ok(DaemonHealth {
        version: body["version"].as_str().unwrap_or_default().to_string(),
        apis: apis.iter().filter_map(Value::as_str).map(str::to_string).collect(),
    })
}

fn client_for(socket_path: PathBuf, health: DaemonHealth) -> Result<DaemonClient, CliError> {
    let skew = diagnostic(
        VERSION_SKEW_CODE,
        &[("daemon_version", &health.version), ("app_version", APP_VERSION)],
    );
    let lacks_an_api = API_PACKAGES
        .iter()
        .any(|package| !health.apis.iter().any(|api| api == package));
    if lacks_an_api {
        return Err(CliError::DaemonUnavailable(vec![skew]));
    }
    let warnings = if health.version == APP_VERSION {
        Vec::new()
    } else {
        vec![skew]
    };
    Ok(DaemonClient {
        socket_path,
        daemon_version: health.version,
        warnings,
    })
}

/// Spawns `rastai serve --headless` detached and waits up to `DAEMON_LOCK_WAIT_SECONDS` for its health. A daemon
/// that stops before answering left its reason in its final `--json` line. Health is still awaited after that until
/// the deadline: two CLIs starting a daemon at once both spawn one, and the loser stops with `RRS-S0001` while the
/// winner comes up (flows/platform.md 2.1).
async fn start_daemon(socket_path: &Path) -> Result<DaemonHealth, CliError> {
    let mut child = spawn_daemon(socket_path)?;
    let deadline = Instant::now() + Duration::from_secs(DAEMON_LOCK_WAIT_SECONDS);
    let mut reasons = Vec::new();
    while Instant::now() < deadline {
        if let Ok(health) = read_health(socket_path).await {
            return Ok(health);
        }
        if reasons.is_empty() && child_has_exited(&mut child) {
            reasons = final_diagnostics_of(&mut child);
        }
        tokio::time::sleep(Duration::from_millis(HEALTH_POLL_MS)).await;
    }
    if reasons.is_empty() {
        reasons.push(not_listening(socket_path));
    }
    Err(CliError::DaemonUnavailable(reasons))
}

fn spawn_daemon(socket_path: &Path) -> Result<Child, CliError> {
    let program = std::env::current_exe().map_err(|_| CliError::DaemonUnavailable(vec![not_listening(socket_path)]))?;
    let mut command = Command::new(program);
    command
        .args(SERVE_ARGUMENTS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    detach_from_terminal(&mut command);
    command
        .spawn()
        .map_err(|_| CliError::DaemonUnavailable(vec![not_listening(socket_path)]))
}

/// Its own process group, so a Ctrl-C meant for this CLI doesn't stop the daemon it started.
#[cfg(unix)]
fn detach_from_terminal(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn detach_from_terminal(_command: &mut Command) {}

fn child_has_exited(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(Some(_)))
}

/// The diagnostics of a stopped `rastai serve --json`: its last stdout line is `{"exit_code", "diagnostics"}`.
fn final_diagnostics_of(child: &mut Child) -> Vec<Diagnostic> {
    let mut text = String::new();
    if let Some(stdout) = child.stdout.as_mut() {
        let _ = stdout.read_to_string(&mut text);
    }
    let Some(last_line) = text.lines().last() else {
        return Vec::new();
    };
    let value: Value = serde_json::from_str(last_line).unwrap_or_default();
    let listed = value[DIAGNOSTICS_KEY].as_array().cloned().unwrap_or_default();
    listed.iter().filter_map(diagnostic_from_json).collect()
}

/// The commands the daemon has registered.
pub async fn list_commands(client: &DaemonClient) -> Result<Vec<CommandDescriptor>, CliError> {
    let request = ListCommandsRequest { section: String::new() };
    let body = unary_call(client, LIST_COMMANDS_PATH, request.encode_to_vec()).await?;
    match ListCommandsResponse::decode(body.as_slice()) {
        Ok(response) => Ok(response.commands),
        Err(error) => Err(broken_answer(client, &error.to_string())),
    }
}

/// `CommandService.Execute`. A command that fails is `ok: false` here, not an error.
pub async fn execute(client: &DaemonClient, request: &ExecuteRequest) -> Result<ExecuteResponse, CliError> {
    let body = unary_call(client, EXECUTE_PATH, request.encode_to_vec()).await?;
    ExecuteResponse::decode(body.as_slice()).map_err(|error| broken_answer(client, &error.to_string()))
}

/// Opens `EventService.Subscribe`; follow.rs reads its envelopes. `chunk_timeout_ms` bounds the wait for the head
/// and then for each chunk.
pub async fn open_event_stream(
    client: &DaemonClient,
    request: &SubscribeRequest,
    chunk_timeout_ms: u64,
) -> Result<HttpStream, CliError> {
    let message = request.encode_to_vec();
    let mut body = Vec::with_capacity(ENVELOPE_HEADER_BYTES + message.len());
    body.push(MESSAGE_FLAGS);
    body.extend_from_slice(&(message.len() as u32).to_be_bytes());
    body.extend_from_slice(&message);
    let request = post_request(client, SUBSCRIBE_PATH, STREAM_CONTENT_TYPE, body, chunk_timeout_ms);
    let stream = open_response_stream(request)
        .await
        .map_err(|error| CliError::DaemonUnavailable(vec![transport_diagnostic(&error, &client.socket_path)]))?;
    if stream.status != HTTP_OK {
        return Err(broken_answer(client, &format!("HTTP {}", stream.status)));
    }
    Ok(stream)
}

/// One unary Connect call. A refusal before the command runs (the gate, an unreadable body) comes back as a Connect
/// error envelope with a non-200 status; its diagnostic decides the error.
async fn unary_call(client: &DaemonClient, path: &str, body: Vec<u8>) -> Result<Vec<u8>, CliError> {
    let request = post_request(client, path, PROTO_CONTENT_TYPE, body, REQUEST_TIMEOUT_MS);
    let response = send_request(request)
        .await
        .map_err(|error| CliError::DaemonUnavailable(vec![transport_diagnostic(&error, &client.socket_path)]))?;
    if response.status == HTTP_OK {
        return Ok(response.body);
    }
    Err(refusal_from_envelope(&response.body, client))
}

fn post_request(client: &DaemonClient, path: &str, content_type: &str, body: Vec<u8>, timeout_ms: u64) -> HttpRequest {
    HttpRequest {
        method: POST_METHOD.to_string(),
        target: HttpTarget::LocalSocket {
            socket_path: client.socket_path.clone(),
        },
        path: path.to_string(),
        headers: vec![
            (CONTENT_TYPE_HEADER.to_string(), content_type.to_string()),
            client_header(),
        ],
        body,
        timeout_ms,
    }
}

/// The Connect error envelope's diagnostic: a credential or scope refusal is `NotPermitted`, the rest `Failed`.
pub fn refusal_from_envelope(body: &[u8], client: &DaemonClient) -> CliError {
    let envelope: Value = serde_json::from_slice(body).unwrap_or_default();
    let found = envelope[DETAILS_KEY]
        .as_array()
        .and_then(|details| details.first())
        .and_then(|detail| diagnostic_from_json(&detail[DEBUG_KEY]));
    match found {
        Some(refusal) => refusal_error(refusal),
        None => broken_answer(client, &String::from_utf8_lossy(body)),
    }
}

/// A refusal the daemon sent: a credential or scope one is `NotPermitted`, any other `Failed`.
pub fn refusal_error(refusal: Diagnostic) -> CliError {
    if exit_code_for_failure(std::slice::from_ref(&refusal)) == EXIT_NOT_PERMITTED {
        return CliError::NotPermitted(vec![refusal]);
    }
    CliError::Failed(vec![refusal])
}

/// A diagnostic written as JSON by rastai-diag's `render_json` or the daemon's Connect JSON. The severity comes
/// from the catalog entry of its code, as both writers spell it differently; an unknown code is an error.
pub fn diagnostic_from_json(value: &Value) -> Option<Diagnostic> {
    let code = value["code"].as_str()?;
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    let severity = match find_entry(code) {
        Some(entry) => entry.severity,
        None => Severity::Error,
    };
    Some(Diagnostic {
        code: code.to_string(),
        severity,
        message: text("message"),
        path: text("path"),
        fix_text: text("fix_text"),
        fix_command_id: text("fix_command_id"),
        fix_arguments_json: text("fix_arguments_json"),
    })
}

/// A transport failure as its catalog diagnostic, every placeholder filled.
pub fn transport_diagnostic(error: &TransportError, socket_path: &Path) -> Diagnostic {
    let path = socket_path.display().to_string();
    let reason = match error {
        TransportError::SocketNotReachable { reason, .. }
        | TransportError::SocketFolderInsecure { reason, .. }
        | TransportError::SocketNotCreated { reason, .. }
        | TransportError::ConnectionFailed { reason } => reason.clone(),
        TransportError::TimedOut { timeout_ms } => format!("no answer within {timeout_ms} ms"),
        TransportError::PeerIsAnotherUser { peer_user_id, .. } => format!("user {peer_user_id}"),
        TransportError::NotSupportedOnThisPlatform { what } => what.to_string(),
    };
    if error.code() == NOT_SUPPORTED_PLATFORM_CODE {
        return diagnostic(error.code(), &[("platform", std::env::consts::OS)]);
    }
    diagnostic(error.code(), &[("path", &path), ("reason", &reason)])
}

/// An answer the CLI can't read: the daemon is there but speaks something else (`RRS-N2008`).
fn broken_answer(client: &DaemonClient, reason: &str) -> CliError {
    let error = TransportError::ConnectionFailed {
        reason: reason.to_string(),
    };
    CliError::DaemonUnavailable(vec![transport_diagnostic(&error, &client.socket_path)])
}

fn not_listening(socket_path: &Path) -> Diagnostic {
    diagnostic(NOT_LISTENING_CODE, &[("path", &socket_path.display().to_string())])
}

fn client_header() -> (String, String) {
    (CLIENT_HEADER.to_string(), format!("{CLIENT_NAME}/{APP_VERSION}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_diag::render_json::render_json;
    use serde_json::json;

    fn health(version: &str, apis: &[&str]) -> DaemonHealth {
        DaemonHealth {
            version: version.to_string(),
            apis: apis.iter().map(|api| api.to_string()).collect(),
        }
    }

    #[test]
    fn a_daemon_of_another_version_is_used_with_a_warning() {
        let client = client_for(PathBuf::from("/s"), health("0.0.9", API_PACKAGES)).unwrap();
        assert_eq!(client.warnings[0].code, "RRS-C0006");
        let same = client_for(PathBuf::from("/s"), health(APP_VERSION, API_PACKAGES)).unwrap();
        assert!(same.warnings.is_empty());
    }

    #[test]
    fn a_daemon_without_an_api_this_cli_calls_is_refused() {
        let refusal = client_for(PathBuf::from("/s"), health("9.0.0", &["rastai.commands.v1"])).unwrap_err();
        assert_eq!(refusal.exit_code(), 3);
        assert_eq!(refusal.diagnostics()[0].code, "RRS-C0006");
    }

    #[test]
    fn a_rendered_diagnostic_reads_back() {
        let original = diagnostic(
            "RRS-C1005",
            &[("command_id", "x.y"), ("scope", "training"), ("principal", "p")],
        );
        let read = diagnostic_from_json(&render_json(&original)).unwrap();
        assert_eq!(read, original);
        assert_eq!(diagnostic_from_json(&json!({"message": "no code"})), None);
    }
}
