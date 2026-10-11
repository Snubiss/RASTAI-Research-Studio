//! The stdio bridge against an in-process fake `/mcp` on a Unix socket (layout: rust-control-plane.md 7.6): lines in
//! order with the token and the session headers; a 404 after a daemon restart replays `initialize` once and retries;
//! no token answers `RRS-C3002` without contacting the daemon; malformed lines are answered by the bridge itself; an
//! unreachable daemon and a refusal from the request gate come back as coded errors.
//!
//! The fake daemon is a few lines of raw HTTP/1.1, like rastai-transport's tests, so the tests also see the exact
//! headers the bridge sends. Unix only, like daemon.sock.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rastai_mcp::bridge::{DaemonEndpoint, run_bridge};
use rastai_transport::local_socket::{LocalStream, accept_local, bind_local_socket};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

const TOKEN: &str = "tok_01JA2B3C4D5E6F7G8H9J0K1M2N";
const FIRST_SESSION: &str = "SESSION-ONE";
const SECOND_SESSION: &str = "SESSION-TWO";
const PROTOCOL_VERSION: &str = "2025-06-18";
/// The blank line between an HTTP head and its body.
const END_OF_HEAD: &[u8] = b"\r\n\r\n";
/// A short prefix keeps the socket path within the 107 bytes a Unix socket address holds.
const TEST_FOLDER_PREFIX: &str = "rmb";
static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
const TOOLS_CALL: &str =
    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system__status","arguments":{}}}"#;

/// One request as the fake daemon read it.
#[derive(Debug, Clone)]
struct ReceivedRequest {
    request_line: String,
    /// Lower-cased names, as HTTP/1.1 headers are case-insensitive.
    headers: Vec<(String, String)>,
    body: Value,
}

/// What the fake daemon answers.
struct FakeAnswer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

/// Chooses the answer to the request numbered from 0.
type AnswerFunction = fn(usize, &ReceivedRequest) -> FakeAnswer;

struct FakeDaemon {
    endpoint: DaemonEndpoint,
    test_folder: PathBuf,
    requests: Arc<Mutex<Vec<ReceivedRequest>>>,
    task: JoinHandle<()>,
}

#[tokio::test]
async fn lines_are_forwarded_in_order_with_the_token_and_the_session() {
    let daemon = start_fake_daemon(answer_normally);
    let output = run_on_lines(&daemon, TOKEN, &[INITIALIZE, INITIALIZED, TOOLS_LIST, TOOLS_CALL]).await;
    let requests = stop_fake_daemon(daemon);

    assert_eq!(reply_ids(&output), [json!(1), json!(2), json!(3)]);
    assert_eq!(output[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(output[2]["result"]["structuredContent"]["node_count"], 1);
    assert_eq!(
        methods(&requests),
        ["initialize", "notifications/initialized", "tools/list", "tools/call"]
    );
    for request in &requests {
        assert_eq!(request.request_line, "POST /mcp HTTP/1.1");
        assert_eq!(header(request, "authorization"), Some(format!("Bearer {TOKEN}")));
        assert_eq!(header(request, "content-type"), Some("application/json".to_string()));
    }
    assert_eq!(header(&requests[0], "mcp-session-id"), None);
    for request in &requests[1..] {
        assert_eq!(header(request, "mcp-session-id"), Some(FIRST_SESSION.to_string()));
        assert_eq!(
            header(request, "mcp-protocol-version"),
            Some(PROTOCOL_VERSION.to_string())
        );
    }
    assert_eq!(requests[3].body, serde_json::from_str::<Value>(TOOLS_CALL).unwrap());
}

#[tokio::test]
async fn a_lost_session_after_a_daemon_restart_replays_initialize_once_and_retries() {
    let daemon = start_fake_daemon(answer_as_a_restarted_daemon);
    let output = run_on_lines(&daemon, TOKEN, &[INITIALIZE, INITIALIZED, TOOLS_LIST]).await;
    let requests = stop_fake_daemon(daemon);

    // The client sees one reply per request: the replayed initialize is the bridge's business.
    assert_eq!(reply_ids(&output), [json!(1), json!(2)]);
    assert_eq!(output[1]["result"]["tools"], json!([]));
    let expected_methods = [
        "initialize",
        "notifications/initialized",
        "tools/list",
        "initialize",
        "notifications/initialized",
        "tools/list",
    ];
    assert_eq!(methods(&requests), expected_methods);
    let sessions: Vec<Option<String>> = requests
        .iter()
        .map(|request| header(request, "mcp-session-id"))
        .collect();
    let first = Some(FIRST_SESSION.to_string());
    let second = Some(SECOND_SESSION.to_string());
    assert_eq!(sessions, [None, first.clone(), first, None, second.clone(), second]);
}

#[tokio::test]
async fn a_session_lost_again_after_the_replay_is_not_retried_twice() {
    let daemon = start_fake_daemon(answer_as_a_daemon_that_keeps_losing_sessions);
    let output = run_on_lines(&daemon, TOKEN, &[INITIALIZE, TOOLS_LIST]).await;
    let requests = stop_fake_daemon(daemon);

    assert_eq!(reply_ids(&output), [json!(1), json!(2)]);
    assert_eq!(output[1]["error"]["data"]["code"], "RRS-N2008");
    assert_eq!(
        methods(&requests),
        ["initialize", "tools/list", "initialize", "tools/list"]
    );
}

#[tokio::test]
async fn without_a_token_every_request_gets_rrs_c3002_and_the_daemon_is_never_contacted() {
    for token in ["", "  "] {
        let daemon = start_fake_daemon(answer_normally);
        let output = run_on_lines(&daemon, token, &[INITIALIZE, INITIALIZED, TOOLS_LIST]).await;
        let requests = stop_fake_daemon(daemon);

        assert_eq!(reply_ids(&output), [json!(1), json!(2)], "{token:?}");
        for reply in &output {
            assert_eq!(reply["error"]["data"]["code"], "RRS-C3002", "{token:?}");
            assert_eq!(reply["error"]["message"], "The MCP bridge has no token.");
        }
        assert!(requests.is_empty(), "the bridge contacted the daemon without a token");
    }
}

#[tokio::test]
async fn malformed_lines_are_answered_by_the_bridge_with_rrs_c3001() {
    let daemon = start_fake_daemon(answer_normally);
    let batch = format!("[{TOOLS_LIST}]");
    let lines = ["this is not json", batch.as_str(), "", INITIALIZE];
    let mut input: Vec<u8> = Vec::new();
    for line in lines {
        input.extend_from_slice(line.as_bytes());
        input.push(b'\n');
    }
    input.extend_from_slice(b"\xff\xfe\n");
    let output = run_on_bytes(&daemon, TOKEN, &input).await;
    let requests = stop_fake_daemon(daemon);

    assert_eq!(output.len(), 4);
    let codes: Vec<&Value> = output.iter().map(|reply| &reply["error"]["data"]["code"]).collect();
    assert_eq!(codes[0], "RRS-C3001");
    assert_eq!(codes[1], "RRS-C3001");
    assert_eq!(output[2]["id"], 1);
    assert_eq!(codes[3], "RRS-C3001");
    assert_eq!(output[0]["error"]["code"], -32700);
    assert_eq!(output[1]["error"]["code"], -32600);
    assert_eq!(
        output[3]["error"]["message"],
        "Malformed MCP request: the line isn't UTF-8."
    );
    assert_eq!(methods(&requests), ["initialize"]);
}

#[tokio::test]
async fn a_daemon_that_is_not_running_gives_rrs_n2004_and_notifications_stay_silent() {
    let daemon = start_fake_daemon(answer_normally);
    let stopped = stop_fake_daemon_and_socket(&daemon);
    let output = run_on_lines(&daemon, TOKEN, &[INITIALIZED, TOOLS_LIST]).await;
    let _ = fs::remove_dir_all(&daemon.test_folder);

    assert!(stopped.is_empty());
    assert_eq!(reply_ids(&output), [json!(2)]);
    assert_eq!(output[0]["error"]["code"], -32000);
    assert_eq!(output[0]["error"]["data"]["code"], "RRS-N2004");
}

#[tokio::test]
async fn a_refusal_from_the_request_gate_keeps_its_code() {
    let daemon = start_fake_daemon(answer_with_a_revoked_token);
    let output = run_on_lines(&daemon, TOKEN, &[INITIALIZE]).await;
    stop_fake_daemon(daemon);

    assert_eq!(reply_ids(&output), [json!(1)]);
    assert_eq!(output[0]["error"]["data"]["code"], "RRS-C1006");
    assert_eq!(output[0]["error"]["message"], "This token was revoked.");
}

// ---- The fake daemon's answers ----

/// Every request answered as a healthy daemon would: initialize opens the first session.
fn answer_normally(_number: usize, request: &ReceivedRequest) -> FakeAnswer {
    match request.body["method"].as_str().unwrap_or_default() {
        "initialize" => initialize_answer(request, FIRST_SESSION),
        "tools/list" => reply_answer(request, json!({ "tools": [] })),
        "tools/call" => reply_answer(
            request,
            json!({ "structuredContent": { "node_count": 1 }, "isError": false }),
        ),
        _ => accepted_answer(),
    }
}

/// The daemon restarts after the first two requests: it no longer knows the first session, and a new initialize
/// opens the second.
fn answer_as_a_restarted_daemon(number: usize, request: &ReceivedRequest) -> FakeAnswer {
    let method = request.body["method"].as_str().unwrap_or_default();
    if method == "initialize" {
        let session = if number == 0 { FIRST_SESSION } else { SECOND_SESSION };
        return initialize_answer(request, session);
    }
    if number >= 2 && header(request, "mcp-session-id").as_deref() == Some(FIRST_SESSION) {
        return not_found_answer();
    }
    answer_normally(number, request)
}

/// Opens sessions and then forgets each one at once.
fn answer_as_a_daemon_that_keeps_losing_sessions(number: usize, request: &ReceivedRequest) -> FakeAnswer {
    if request.body["method"] == "initialize" {
        let session = if number == 0 { FIRST_SESSION } else { SECOND_SESSION };
        return initialize_answer(request, session);
    }
    not_found_answer()
}

/// The request gate refuses a revoked token before MCP, with the diagnostic as JSON (`render_json`).
fn answer_with_a_revoked_token(_number: usize, _request: &ReceivedRequest) -> FakeAnswer {
    let body = json!({
        "code": "RRS-C1006", "severity": "error", "message": "This token was revoked.", "path": "",
        "fix_text": "Create a new token.", "fix_command_id": "", "fix_arguments_json": "",
    });
    FakeAnswer {
        status: 401,
        headers: Vec::new(),
        body: body.to_string(),
    }
}

fn initialize_answer(request: &ReceivedRequest, session: &str) -> FakeAnswer {
    let mut answer = reply_answer(
        request,
        json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {} }),
    );
    answer.headers.push(("Mcp-Session-Id", session.to_string()));
    answer
}

fn reply_answer(request: &ReceivedRequest, result: Value) -> FakeAnswer {
    let reply = json!({ "jsonrpc": "2.0", "id": request.body["id"], "result": result });
    FakeAnswer {
        status: 200,
        headers: vec![("Content-Type", "application/json".to_string())],
        body: reply.to_string(),
    }
}

fn accepted_answer() -> FakeAnswer {
    FakeAnswer {
        status: 202,
        headers: Vec::new(),
        body: String::new(),
    }
}

fn not_found_answer() -> FakeAnswer {
    FakeAnswer {
        status: 404,
        headers: Vec::new(),
        body: String::new(),
    }
}

// ---- Running the bridge ----

async fn run_on_lines(daemon: &FakeDaemon, token: &str, lines: &[&str]) -> Vec<Value> {
    let mut input = Vec::new();
    for line in lines {
        input.extend_from_slice(line.as_bytes());
        input.push(b'\n');
    }
    run_on_bytes(daemon, token, &input).await
}

/// Runs the bridge to the end of `input` and returns each output line as JSON.
async fn run_on_bytes(daemon: &FakeDaemon, token: &str, input: &[u8]) -> Vec<Value> {
    let mut output: Vec<u8> = Vec::new();
    run_bridge(&daemon.endpoint, token, input, &mut output).await;
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "every reply ends its line: {text:?}"
    );
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn reply_ids(output: &[Value]) -> Vec<Value> {
    output.iter().map(|reply| reply["id"].clone()).collect()
}

fn methods(requests: &[ReceivedRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| request.body["method"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn header(request: &ReceivedRequest, name: &str) -> Option<String> {
    request
        .headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.clone())
}

// ---- The fake daemon ----

/// Binds a fake daemon.sock in a fresh test folder and answers each connection with `answer`.
fn start_fake_daemon(answer: AnswerFunction) -> FakeDaemon {
    let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
    let test_folder = std::env::temp_dir().join(format!("{TEST_FOLDER_PREFIX}-{}-{number}", std::process::id()));
    let _ = fs::remove_dir_all(&test_folder);
    let socket_path = test_folder.join("run").join("daemon.sock");
    let listener = bind_local_socket(&socket_path).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        loop {
            let (LocalStream::Unix(mut stream), _peer) = accept_local(&listener).await.unwrap();
            let request = read_request(&mut stream).await;
            let number = push_request(&log, request.clone());
            write_answer(&mut stream, &answer(number, &request)).await;
        }
    });
    FakeDaemon {
        endpoint: DaemonEndpoint { socket_path },
        test_folder,
        requests,
        task,
    }
}

/// Stops the fake daemon, removes its folder and returns what it received.
fn stop_fake_daemon(daemon: FakeDaemon) -> Vec<ReceivedRequest> {
    let requests = stop_fake_daemon_and_socket(&daemon);
    let _ = fs::remove_dir_all(&daemon.test_folder);
    requests
}

/// Stops the fake daemon and removes its socket file, keeping the 0700 folder: the next connect finds nothing
/// listening, as after a daemon crash.
fn stop_fake_daemon_and_socket(daemon: &FakeDaemon) -> Vec<ReceivedRequest> {
    daemon.task.abort();
    let _ = fs::remove_file(&daemon.endpoint.socket_path);
    daemon.requests.lock().unwrap().clone()
}

fn push_request(log: &Mutex<Vec<ReceivedRequest>>, request: ReceivedRequest) -> usize {
    let mut requests = log.lock().unwrap();
    requests.push(request);
    requests.len() - 1
}

/// Reads one request: the head up to the blank line, then Content-Length bytes of body.
async fn read_request(stream: &mut UnixStream) -> ReceivedRequest {
    let mut bytes = Vec::new();
    let head_length = loop {
        if let Some(position) = bytes
            .windows(END_OF_HEAD.len())
            .position(|window| window == END_OF_HEAD)
        {
            break position + END_OF_HEAD.len();
        }
        read_more(stream, &mut bytes).await;
    };
    let head = String::from_utf8(bytes[..head_length].to_vec()).unwrap();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap().to_string();
    let headers: Vec<(String, String)> = lines.filter_map(split_header).collect();
    let body_length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map_or(0, |(_, value)| value.parse().unwrap());
    while bytes.len() < head_length + body_length {
        read_more(stream, &mut bytes).await;
    }
    ReceivedRequest {
        request_line,
        headers,
        body: serde_json::from_slice(&bytes[head_length..]).unwrap(),
    }
}

async fn read_more(stream: &mut UnixStream, bytes: &mut Vec<u8>) {
    let mut buffer = [0; 4096];
    let count = stream.read(&mut buffer).await.unwrap();
    assert!(count > 0, "the bridge closed the connection mid-request");
    bytes.extend_from_slice(&buffer[..count]);
}

fn split_header(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once(':')?;
    Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
}

async fn write_answer(stream: &mut UnixStream, answer: &FakeAnswer) {
    let mut head = format!(
        "HTTP/1.1 {} Fake\r\ncontent-length: {}\r\n",
        answer.status,
        answer.body.len()
    );
    for (name, value) in &answer.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(answer.body.as_bytes()).await.unwrap();
}
