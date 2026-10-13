//! Each outcome maps to its exit code (layout: rust-control-plane.md 9.5), the real `rastai` binary against a fake
//! daemon on a temporary daemon.sock. The fake speaks the daemon's wire: `/api/health` JSON, binary Connect for
//! `ListCommands` and `Execute`, the Connect error envelope and the `Subscribe` stream envelopes, the last two built
//! by rastai-server's own functions. Unix only, like daemon.sock.
//!
//! | Outcome | Exit code |
//! |---|---|
//! | a sync command that succeeded; a stream command's session id (followed from M3); a job that `succeeded`; `--detach` | 0 `EXIT_OK` |
//! | `ok: false`; a job that `failed` while running, or was `cancelled` | 1 `EXIT_FAILED` |
//! | an unknown verb; a flag the command doesn't have; `--set` on a verb | 2 `EXIT_USAGE` |
//! | no daemon and `--no-start`; a daemon without an API package the CLI calls | 3 `EXIT_DAEMON_UNAVAILABLE` |
//! | a scope refusal in the answer; a credential refusal by the gate | 4 `EXIT_NOT_PERMITTED` |
//! | a job that failed before it started | 5 `EXIT_ERRORED` |

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use prost::Message;
use rastai_core::versions::{API_PACKAGES, APP_VERSION};
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::diagnostic;
use rastai_proto::commands::v1::{
    CommandDescriptor, CommandKind, ExecuteRequest, ExecuteResponse, ListCommandsResponse,
};
use rastai_proto::common::v1::JobState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, EventPayload, JobLogLine, JobProgress, JobStateChanged};
use rastai_server::http::connect::{connect_error_json, end_of_stream_envelope};
use rastai_transport::local_socket::{LocalStream, accept_local, bind_local_socket};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;

use common::{final_codes, final_line, run_rastai, test_home};

const END_OF_HEAD: &[u8] = b"\r\n\r\n";
const HEALTH_PATH: &str = "/api/health";
const LIST_COMMANDS_PATH: &str = "/rastai.commands.v1.CommandService/ListCommands";
const EXECUTE_PATH: &str = "/rastai.commands.v1.CommandService/Execute";
const SUBSCRIBE_PATH: &str = "/rastai.events.v1.EventService/Subscribe";
/// The fake's train.start names its job after the recipe, which picks how the job ends.
const JOB_ID_PREFIX: &str = "job_";
/// Another job's events reach the stream too; a follower must ignore them.
const OTHER_JOB_ID: &str = "job_other";

/// What the fake daemon says about itself.
struct FakeHealth {
    version: String,
    apis: Vec<String>,
}

/// A running fake daemon: its runtime, and every request line it received. Dropping it stops the runtime and
/// removes the test's home.
struct FakeDaemon {
    runtime: Option<Runtime>,
    requests: Arc<Mutex<Vec<String>>>,
    home: PathBuf,
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

// ---- The outcomes ----

#[test]
fn a_sync_command_that_succeeded_exits_0_and_prints_its_result() {
    let home = test_home("sync-ok");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["system", "status", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let first_line: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first_line, json!({"node_count": 1}));
    assert_eq!(final_line(&result), json!({"exit_code": 0, "diagnostics": []}));
    // The help cache keeps this version's commands.
    assert!(help_cache(&home, APP_VERSION).exists());
}

#[test]
fn a_stream_command_prints_its_session_id_and_subscribes_to_nothing_until_m3() {
    let home = test_home("stream");
    let daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["chat", "send", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let first_line: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first_line, json!({"session_id": "ses_a"}));
    assert!(!requests_of(&daemon).iter().any(|line| line.contains(SUBSCRIBE_PATH)));
}

#[test]
fn a_command_that_failed_exits_1() {
    let home = test_home("sync-failed");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["plugin", "list", "--json"]);
    assert_eq!(result.exit_code, 1, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0010"]);
}

#[test]
fn a_scope_refusal_exits_4() {
    let home = test_home("scope");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["token", "list", "--json"]);
    assert_eq!(result.exit_code, 4, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C1005"]);
}

#[test]
fn a_credential_refused_by_the_gate_exits_4() {
    let home = test_home("gate");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["node", "list", "--json"]);
    assert_eq!(result.exit_code, 4, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C1002"]);
}

#[test]
fn an_unknown_verb_exits_2_with_the_closest_verbs() {
    let home = test_home("unknown-verb");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["system", "statu", "--json"]);
    assert_eq!(result.exit_code, 2, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0001"]);
    let fix = final_line(&result)["diagnostics"][0]["fix_text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(fix.contains("rastai system status"), "{fix}");
}

#[test]
fn a_flag_the_command_does_not_have_exits_2_before_anything_is_sent() {
    let home = test_home("bad-flag");
    let daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["system", "status", "--colour", "red", "--json"]);
    assert_eq!(result.exit_code, 2, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0002", "RRS-C0002"]);
    let message = final_line(&result)["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("this command takes none"), "{message}");
    assert!(!requests_of(&daemon).iter().any(|line| line.contains(EXECUTE_PATH)));
}

#[test]
fn a_settings_flag_on_a_verb_exits_2() {
    let home = test_home("set-on-verb");
    let daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["system", "status", "--set", "ui.theme=dark", "--json"]);
    assert_eq!(result.exit_code, 2, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0002"]);
    assert!(!requests_of(&daemon).iter().any(|line| line.contains(EXECUTE_PATH)));
}

#[test]
fn no_daemon_with_no_start_exits_3() {
    let home = test_home("no-daemon");
    let result = run_rastai(&home, &["system", "status", "--no-start", "--json"]);
    assert_eq!(result.exit_code, 3, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-N2004"]);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_daemon_without_an_api_this_cli_calls_exits_3() {
    let home = test_home("no-api");
    let health = FakeHealth {
        version: "9.0.0".to_string(),
        apis: vec!["rastai.commands.v2".to_string()],
    };
    let _daemon = start_fake_daemon(&home, health);
    let result = run_rastai(&home, &["system", "status", "--json"]);
    assert_eq!(result.exit_code, 3, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0006"]);
}

#[test]
fn a_daemon_of_another_version_is_used_with_warning_rrs_c0006() {
    let home = test_home("skew");
    let health = FakeHealth {
        version: "0.0.9".to_string(),
        apis: API_PACKAGES.iter().map(|api| api.to_string()).collect(),
    };
    let _daemon = start_fake_daemon(&home, health);
    let result = run_rastai(&home, &["system", "status", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C0006"]);
    assert_eq!(final_line(&result)["diagnostics"][0]["severity"], "warning");
    // Another version's commands aren't this version's help.
    assert!(!help_cache(&home, "0.0.9").exists());
    assert!(!help_cache(&home, APP_VERSION).exists());
}

#[test]
fn a_job_that_succeeded_exits_0_after_its_events() {
    let home = test_home("job-ok");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["train", "start", "succeeds"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let lines: Vec<&str> = result.stdout.lines().collect();
    assert_eq!(
        lines,
        [
            "job_succeeds",
            "job_succeeds starting",
            "job_succeeds running",
            "epoch 1",
            "50% halfway",
            "job_succeeds succeeded"
        ]
    );
}

#[test]
fn a_job_that_failed_while_running_exits_1() {
    let home = test_home("job-failed");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["train", "start", "fails", "--json"]);
    assert_eq!(result.exit_code, 1, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-J1004"]);
}

#[test]
fn a_job_that_was_cancelled_exits_1() {
    let home = test_home("job-cancelled");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["train", "start", "--recipe", "cancelled", "--json"]);
    assert_eq!(result.exit_code, 1, "{result:?}");
}

#[test]
fn a_job_that_failed_before_it_started_exits_5() {
    let home = test_home("job-errored");
    let _daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["train", "start", "errors", "--json"]);
    assert_eq!(result.exit_code, 5, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-J3006"]);
}

#[test]
fn a_detached_job_exits_0_with_its_id_and_is_not_followed() {
    let home = test_home("job-detached");
    let daemon = start_fake_daemon(&home, healthy());
    let result = run_rastai(&home, &["train", "start", "fails", "--detach", "--json"]);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let first_line: Value = serde_json::from_str(result.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first_line, json!({"job_id": "job_fails"}));
    assert!(!requests_of(&daemon).iter().any(|line| line.contains(SUBSCRIBE_PATH)));
}

#[test]
fn the_request_carries_the_arguments_and_a_fresh_idempotency_key() {
    let home = test_home("request");
    let daemon = start_fake_daemon(&home, healthy());
    let first = run_rastai(&home, &["train", "start", "succeeds", "--json"]);
    let second = run_rastai(&home, &["train", "start", "--recipe=succeeds", "--json"]);
    assert_eq!((first.exit_code, second.exit_code), (0, 0));
    let executed: Vec<String> = requests_of(&daemon)
        .into_iter()
        .filter(|line| line.starts_with(EXECUTE_PATH))
        .collect();
    assert_eq!(executed.len(), 2);
    let keys: Vec<&str> = executed.iter().map(|line| line.rsplit(' ').next().unwrap()).collect();
    assert_eq!(keys[0].len(), 32);
    assert_ne!(keys[0], keys[1]);
    assert!(executed[0].contains(r#"{"recipe":"succeeds"}"#), "{executed:?}");
}

// ---- The fake daemon ----

/// `<cache>/commands-<version>.json` under the test's `RASTAI_HOME` (contracts 8.1).
fn help_cache(home: &Path, version: &str) -> PathBuf {
    home.join("cache").join(format!("commands-{version}.json"))
}

fn healthy() -> FakeHealth {
    FakeHealth {
        version: APP_VERSION.to_string(),
        apis: API_PACKAGES.iter().map(|api| api.to_string()).collect(),
    }
}

fn requests_of(daemon: &FakeDaemon) -> Vec<String> {
    daemon.requests.lock().unwrap().clone()
}

/// Binds `<home>/runtime/daemon.sock` and answers on its own runtime until the returned value is dropped.
fn start_fake_daemon(home: &Path, health: FakeHealth) -> FakeDaemon {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let socket_path = common::daemon_socket(home);
    let listener = {
        let _entered = runtime.enter();
        bind_local_socket(&socket_path).unwrap()
    };
    let (jobs, _) = broadcast::channel::<String>(16);
    let health = Arc::new(health);
    let log = Arc::clone(&requests);
    runtime.spawn(async move {
        loop {
            let (LocalStream::Unix(stream), _peer) = accept_local(&listener).await.unwrap();
            tokio::spawn(answer_connection(
                stream,
                Arc::clone(&health),
                Arc::clone(&log),
                jobs.clone(),
            ));
        }
    });
    FakeDaemon {
        runtime: Some(runtime),
        requests,
        home: home.to_path_buf(),
    }
}

/// One request read and answered. A Subscribe stream stays open until the CLI hangs up.
async fn answer_connection(
    mut stream: UnixStream,
    health: Arc<FakeHealth>,
    log: Arc<Mutex<Vec<String>>>,
    jobs: broadcast::Sender<String>,
) {
    let (path, body) = read_request(&mut stream).await;
    match path.as_str() {
        HEALTH_PATH => {
            let answer = json!({"status": "ok", "version": health.version, "apis": health.apis});
            write_answer(&mut stream, 200, "application/json", answer.to_string().as_bytes()).await;
        }
        LIST_COMMANDS_PATH => {
            let answer = ListCommandsResponse {
                commands: fake_commands(),
            };
            write_answer(&mut stream, 200, "application/proto", &answer.encode_to_vec()).await;
        }
        EXECUTE_PATH => {
            let request = ExecuteRequest::decode(body.as_slice()).unwrap();
            log.lock().unwrap().push(format!(
                "{EXECUTE_PATH} {} {} {}",
                request.command_id, request.arguments_json, request.idempotency_key
            ));
            answer_execute(&mut stream, &request, &jobs).await;
        }
        SUBSCRIBE_PATH => {
            log.lock().unwrap().push(SUBSCRIBE_PATH.to_string());
            stream_job_events(&mut stream, jobs.subscribe()).await;
        }
        _ => write_answer(&mut stream, 404, "text/plain", b"").await,
    }
}

fn fake_commands() -> Vec<CommandDescriptor> {
    let no_arguments = json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false});
    let recipe = json!({
        "type": "object",
        "properties": {"recipe": {"type": "string", "x-rastai-cli-position": 0}},
        "required": ["recipe"],
        "additionalProperties": false
    });
    let command = |command_id: &str, kind: CommandKind, schema: &Value| CommandDescriptor {
        command_id: command_id.to_string(),
        kind: kind as i32,
        arguments_schema_json: schema.to_string(),
        ..CommandDescriptor::default()
    };
    vec![
        command("system.status", CommandKind::Sync, &no_arguments),
        command("system.shutdown", CommandKind::Sync, &no_arguments),
        command("plugin.list", CommandKind::Sync, &no_arguments),
        command("token.list", CommandKind::Sync, &no_arguments),
        command("node.list", CommandKind::Sync, &no_arguments),
        command("train.start", CommandKind::Job, &recipe),
        command("chat.send", CommandKind::Stream, &no_arguments),
    ]
}

/// What each fake command answers.
async fn answer_execute(stream: &mut UnixStream, request: &ExecuteRequest, jobs: &broadcast::Sender<String>) {
    let response = match request.command_id.as_str() {
        "system.status" | "system.shutdown" => ExecuteResponse {
            ok: true,
            result_json: json!({"node_count": 1}).to_string(),
            ..ExecuteResponse::default()
        },
        "chat.send" => ExecuteResponse {
            ok: true,
            result_json: json!({"session_id": "ses_a"}).to_string(),
            ..ExecuteResponse::default()
        },
        "plugin.list" => refused("RRS-C0010", &[("command_id", "plugin.list")]),
        "token.list" => refused(
            "RRS-C1005",
            &[
                ("command_id", "token.list"),
                ("scope", "read"),
                ("principal", "token:t"),
            ],
        ),
        "node.list" => {
            let refusal = connect_error_json(&diagnostic("RRS-C1002", &[]));
            write_answer(stream, 401, "application/json", refusal.to_string().as_bytes()).await;
            return;
        }
        _ => {
            let arguments: Value = serde_json::from_str(&request.arguments_json).unwrap();
            let recipe = arguments["recipe"].as_str().unwrap().to_string();
            let _ = jobs.send(recipe.clone());
            ExecuteResponse {
                ok: true,
                job_id: format!("{JOB_ID_PREFIX}{recipe}"),
                ..ExecuteResponse::default()
            }
        }
    };
    write_answer(stream, 200, "application/proto", &response.encode_to_vec()).await;
}

fn refused(code: &str, values: &[(&str, &str)]) -> ExecuteResponse {
    ExecuteResponse {
        ok: false,
        diagnostics: vec![diagnostic_to_proto(&diagnostic(code, values))],
        ..ExecuteResponse::default()
    }
}

/// A Subscribe stream: the head at once, then, when a job starts, its events (after another job's), then the
/// clean end of the stream.
async fn stream_job_events(stream: &mut UnixStream, mut jobs: broadcast::Receiver<String>) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: application/connect+proto\r\nTransfer-Encoding: chunked\r\n\r\n";
    stream.write_all(head.as_bytes()).await.unwrap();
    let Ok(recipe) = jobs.recv().await else {
        return;
    };
    let job_id = format!("{JOB_ID_PREFIX}{recipe}");
    let mut events = vec![state_event(OTHER_JOB_ID, JobState::Running, JobState::Succeeded, None)];
    events.extend(job_events(&job_id, &recipe));
    for (sequence, mut event) in events.into_iter().enumerate() {
        event.bus_sequence = sequence as u64 + 1;
        write_chunk(stream, &envelope(&event.encode_to_vec())).await;
    }
    write_chunk(stream, &end_of_stream_envelope(None)).await;
    let _ = stream.write_all(b"0\r\n\r\n").await;
}

/// The events of each fake recipe's job.
fn job_events(job_id: &str, recipe: &str) -> Vec<BusEvent> {
    let started = vec![
        state_event(job_id, JobState::Queued, JobState::Starting, None),
        state_event(job_id, JobState::Starting, JobState::Running, None),
    ];
    let mut events = Vec::new();
    match recipe {
        "succeeds" => {
            events.extend(started);
            events.push(payload_event(
                job_id,
                Payload::JobLogLine(JobLogLine {
                    stream: 1,
                    text: "epoch 1".to_string(),
                }),
            ));
            events.push(payload_event(
                job_id,
                Payload::JobProgress(JobProgress {
                    fraction: 0.5,
                    message: "halfway".to_string(),
                }),
            ));
            events.push(state_event(job_id, JobState::Running, JobState::Succeeded, None));
        }
        "fails" => {
            events.extend(started);
            events.push(state_event(
                job_id,
                JobState::Running,
                JobState::Failed,
                Some("RRS-J1004"),
            ));
        }
        "cancelled" => {
            events.extend(started);
            events.push(state_event(job_id, JobState::Running, JobState::Cancelled, None));
        }
        _ => events.push(state_event(
            job_id,
            JobState::Queued,
            JobState::Failed,
            Some("RRS-J3006"),
        )),
    }
    events
}

fn state_event(job_id: &str, from: JobState, to: JobState, code: Option<&str>) -> BusEvent {
    let change = JobStateChanged {
        job_id: job_id.to_string(),
        from_state: from as i32,
        to_state: to as i32,
        diagnostic: code.map(|code| diagnostic_to_proto(&diagnostic(code, &[("run_id", "run_a")]))),
        ..JobStateChanged::default()
    };
    BusEvent {
        event_type: "job.state_changed".to_string(),
        subject_id: job_id.to_string(),
        source: "daemon".to_string(),
        payload: Some(EventPayload {
            payload: Some(Payload::JobStateChanged(change)),
        }),
        ..BusEvent::default()
    }
}

/// A log line or progress event: its subject is the run, its source the job (contracts 7.2).
fn payload_event(job_id: &str, payload: Payload) -> BusEvent {
    BusEvent {
        subject_id: "run_a".to_string(),
        source: format!("job:{job_id}"),
        payload: Some(EventPayload { payload: Some(payload) }),
        ..BusEvent::default()
    }
}

fn envelope(message: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend_from_slice(&(message.len() as u32).to_be_bytes());
    bytes.extend_from_slice(message);
    bytes
}

async fn write_chunk(stream: &mut UnixStream, bytes: &[u8]) {
    let mut chunk = format!("{:x}\r\n", bytes.len()).into_bytes();
    chunk.extend_from_slice(bytes);
    chunk.extend_from_slice(b"\r\n");
    let _ = stream.write_all(&chunk).await;
}

/// Reads one request: the head up to the blank line, then Content-Length bytes of body.
async fn read_request(stream: &mut UnixStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let head_length = loop {
        if let Some(position) = bytes
            .windows(END_OF_HEAD.len())
            .position(|window| window == END_OF_HEAD)
        {
            break position + END_OF_HEAD.len();
        }
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "the CLI closed the connection mid-request");
        bytes.extend_from_slice(&buffer[..count]);
    };
    let head = String::from_utf8(bytes[..head_length].to_vec()).unwrap();
    let path = head.split(' ').nth(1).unwrap().to_string();
    let body_length: usize = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, value)| value.trim().parse().unwrap());
    while bytes.len() < head_length + body_length {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "the CLI closed the connection mid-body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    (path, bytes[head_length..head_length + body_length].to_vec())
}

async fn write_answer(stream: &mut UnixStream, status: u16, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}
