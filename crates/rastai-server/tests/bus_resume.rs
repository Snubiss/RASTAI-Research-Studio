//! The bus and `EventService.Subscribe` through a running daemon (contracts 7.2, flows/platform.md 6.3, layout 8.6):
//! a gap inside the buffer is replayed and live events follow; a gap outside it, or a sequence from before a restart,
//! gets `RRS-C0401`; the first sequence after a start is the start time in ms × 1,000; a journaled command publishes
//! `command.finished`; a token needs `read`, and a revoked token's open stream ends with `RRS-C1006`.
//!
//! The stream is read envelope by envelope over a plain TCP socket, the way a Connect client reads it: HTTP/1.1
//! chunks carrying 5-byte envelopes, the last one the end-of-stream envelope.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use prost::Message;
use rastai_core::principal::owner_principal;
use rastai_core::time::now_unix_ms;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, SubscribeRequest};
use rastai_server::daemon::{DaemonOptions, RunningDaemon, start_daemon};
use rastai_server::events::bus::{publish, subscribe};
use rastai_server::http::connect::stream_message_envelope;
use rastai_server::shutdown::request_shutdown;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use common::{
    ANY_FREE_PORT, execute_request, header_value, ok_result, owner_bearer, remove_test_folder, run_command,
    send_to_port, start_test_daemon, stop_test_daemon, test_paths,
};

const SUBSCRIBE_PATH: &str = "/rastai.events.v1.EventService/Subscribe";
const STREAM_CONTENT_TYPE: &str = "application/connect+proto";
/// A small buffer, so a test can push events out of it.
const SMALL_BUFFER_FLAG: &str = "daemon.bus_buffer_events=4";
const READ_TIMEOUT_SECONDS: u64 = 5;
const END_OF_STREAM_FLAGS: u8 = 0b10;
const ENVELOPE_HEADER_BYTES: usize = 5;
const HEAD_END: &[u8] = b"\r\n\r\n";
const LINE_END: &[u8] = b"\r\n";

/// One envelope of the stream: an event, or the end with its JSON (`{}` or `{"error": ...}`).
#[derive(Debug)]
enum Envelope {
    Event(Box<BusEvent>),
    End(Value),
}

/// An open Subscribe stream: the socket, bytes read but not parsed, and the body's bytes not yet made envelopes. A
/// stream refused before its first event is one short body with a Content-Length; any other is chunked.
struct EventStream {
    socket: TcpStream,
    unread: Vec<u8>,
    body: Vec<u8>,
    chunked: bool,
}

async fn open_event_stream(port: u16, authorization: &str, request: &SubscribeRequest) -> EventStream {
    let body = stream_message_envelope(&request.encode_to_vec());
    let head = format!(
        "POST {SUBSCRIBE_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {authorization}\r\n\
         Content-Type: {STREAM_CONTENT_TYPE}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    let mut stream = EventStream {
        socket,
        unread: Vec::new(),
        body: Vec::new(),
        chunked: false,
    };
    let head = read_response_head(&mut stream).await.to_ascii_lowercase();
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(head.contains("content-type: application/connect+proto"), "{head}");
    stream.chunked = head.contains("transfer-encoding: chunked");
    stream
}

async fn read_more(stream: &mut EventStream) {
    let mut buffer = [0u8; 4096];
    let read = tokio::time::timeout(
        Duration::from_secs(READ_TIMEOUT_SECONDS),
        stream.socket.read(&mut buffer),
    )
    .await
    .expect("the stream sent nothing in time")
    .unwrap();
    assert!(read > 0, "the connection closed before the stream ended");
    stream.unread.extend_from_slice(&buffer[..read]);
}

async fn read_response_head(stream: &mut EventStream) -> String {
    loop {
        if let Some(end) = find(&stream.unread, HEAD_END) {
            let head = String::from_utf8(stream.unread[..end].to_vec()).unwrap();
            stream.unread.drain(..end + HEAD_END.len());
            return head;
        }
        read_more(stream).await;
    }
}

/// Moves one whole HTTP chunk from `unread` to `body`, reading more as needed. A body with a Content-Length is
/// moved as it arrives.
async fn read_chunk(stream: &mut EventStream) {
    if !stream.chunked {
        if stream.unread.is_empty() {
            read_more(stream).await;
        }
        let arrived = std::mem::take(&mut stream.unread);
        stream.body.extend_from_slice(&arrived);
        return;
    }
    loop {
        if let Some(line_end) = find(&stream.unread, LINE_END) {
            let size_text = String::from_utf8(stream.unread[..line_end].to_vec()).unwrap();
            let size = usize::from_str_radix(size_text.trim(), 16).unwrap();
            let chunk_start = line_end + LINE_END.len();
            if stream.unread.len() >= chunk_start + size + LINE_END.len() {
                assert!(size > 0, "the body ended without an end-of-stream envelope");
                let chunk: Vec<u8> = stream.unread[chunk_start..chunk_start + size].to_vec();
                stream.body.extend_from_slice(&chunk);
                stream.unread.drain(..chunk_start + size + LINE_END.len());
                return;
            }
        }
        read_more(stream).await;
    }
}

async fn next_envelope(stream: &mut EventStream) -> Envelope {
    loop {
        if stream.body.len() >= ENVELOPE_HEADER_BYTES {
            let length = u32::from_be_bytes(stream.body[1..5].try_into().unwrap()) as usize;
            if stream.body.len() >= ENVELOPE_HEADER_BYTES + length {
                let flags = stream.body[0];
                let message: Vec<u8> = stream.body[ENVELOPE_HEADER_BYTES..ENVELOPE_HEADER_BYTES + length].to_vec();
                stream.body.drain(..ENVELOPE_HEADER_BYTES + length);
                if flags == END_OF_STREAM_FLAGS {
                    return Envelope::End(serde_json::from_slice(&message).unwrap());
                }
                return Envelope::Event(Box::new(BusEvent::decode(message.as_slice()).unwrap()));
            }
        }
        read_chunk(stream).await;
    }
}

async fn next_event(stream: &mut EventStream) -> BusEvent {
    match next_envelope(stream).await {
        Envelope::Event(event) => *event,
        Envelope::End(end) => panic!("the stream ended: {end}"),
    }
}

/// The JSON of the end-of-stream envelope, skipping any events before it.
async fn stream_end(stream: &mut EventStream) -> Value {
    loop {
        if let Envelope::End(end) = next_envelope(stream).await {
            return end;
        }
    }
}

/// The catalog code in an end-of-stream error: its one detail is the `Diagnostic`.
fn end_code(end: &Value) -> String {
    end["error"]["details"][0]["debug"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn find(bytes: &[u8], pattern: &[u8]) -> Option<usize> {
    bytes.windows(pattern.len()).position(|window| window == pattern)
}

fn heartbeat(node_id: &str) -> BusEvent {
    BusEvent {
        event_type: "node.heartbeat".to_string(),
        subject_id: node_id.to_string(),
        source: format!("node:{node_id}"),
        ..BusEvent::default()
    }
}

fn after(sequence: u64) -> SubscribeRequest {
    SubscribeRequest {
        after_bus_sequence: sequence,
        ..SubscribeRequest::default()
    }
}

async fn start_with_small_buffer(paths: &rastai_core::paths::DataPaths) -> RunningDaemon {
    let options = DaemonOptions {
        port: Some(ANY_FREE_PORT),
        set_flags: vec![SMALL_BUFFER_FLAG.to_string()],
        ..DaemonOptions::default()
    };
    start_daemon(paths.clone(), &options).await.unwrap()
}

/// Publishes `count` heartbeats and returns their sequences.
fn publish_heartbeats(running: &RunningDaemon, count: usize) -> Vec<u64> {
    let mut sequences = Vec::new();
    for _ in 0..count {
        sequences.push(publish(&running.server.bus, heartbeat("node_local")));
    }
    sequences
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gap_inside_the_buffer_is_replayed_then_live_events_follow() {
    let paths = test_paths("bus-replay");
    let running = start_with_small_buffer(&paths).await;
    // Six published, four kept: the gap after the fourth is the fifth and sixth.
    let sequences = publish_heartbeats(&running, 6);
    let mut stream = open_event_stream(running.http_port, &owner_bearer(&paths), &after(sequences[3])).await;
    assert_eq!(next_event(&mut stream).await.bus_sequence, sequences[4]);
    let sixth = next_event(&mut stream).await;
    assert_eq!(sixth.bus_sequence, sequences[5]);
    assert_eq!(sixth.event_type, "node.heartbeat");
    assert_eq!(sixth.subject_id, "node_local");

    let live = publish(&running.server.bus, heartbeat("node_local"));
    assert_eq!(next_event(&mut stream).await.bus_sequence, live);

    // A stopping daemon ends the stream cleanly, so its listeners can finish.
    request_shutdown(&running.server);
    assert_eq!(stream_end(&mut stream).await, json!({}));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_filtered_stream_gets_only_its_types_and_subjects() {
    let paths = test_paths("bus-filter");
    let running = start_test_daemon(&paths).await;
    let request = SubscribeRequest {
        after_bus_sequence: 0,
        event_types: vec!["node.heartbeat".to_string()],
        subject_ids: vec!["node_b".to_string()],
    };
    let mut stream = open_event_stream(running.http_port, &owner_bearer(&paths), &request).await;
    publish(&running.server.bus, heartbeat("node_a"));
    let wanted = publish(&running.server.bus, heartbeat("node_b"));
    let received = next_event(&mut stream).await;
    assert_eq!(received.bus_sequence, wanted);
    assert_eq!(received.subject_id, "node_b");
    request_shutdown(&running.server);
    assert_eq!(stream_end(&mut stream).await, json!({}));
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gap_outside_the_buffer_or_from_before_a_restart_is_rrs_c0401() {
    let paths = test_paths("bus-missed");
    let running = start_with_small_buffer(&paths).await;
    let sequences = publish_heartbeats(&running, 6);
    let bearer = owner_bearer(&paths);
    // The second event is gone from the buffer: the client after the first can't be caught up.
    let mut too_old = open_event_stream(running.http_port, &bearer, &after(sequences[0])).await;
    let end = stream_end(&mut too_old).await;
    assert_eq!(end_code(&end), "RRS-C0401");
    let sequence = sequences[0].to_string();
    assert!(end["error"]["message"].as_str().unwrap().contains(&sequence), "{end}");
    // One after the second still is: its gap starts with the third, the oldest kept.
    let mut kept = open_event_stream(running.http_port, &bearer, &after(sequences[1])).await;
    assert_eq!(next_event(&mut kept).await.bus_sequence, sequences[2]);
    request_shutdown(&running.server);
    stream_end(&mut kept).await;
    stop_test_daemon(running).await;

    // A new daemon's first sequence is later than any the old one gave out; the old one's are refused.
    let restarted = start_with_small_buffer(&paths).await;
    let mut from_before = open_event_stream(restarted.http_port, &bearer, &after(sequences[5])).await;
    assert_eq!(end_code(&stream_end(&mut from_before).await), "RRS-C0401");
    let first_after_restart = publish(&restarted.server.bus, heartbeat("node_local"));
    assert!(first_after_restart > sequences[5]);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_sequence_is_the_start_time_times_1000_and_commands_publish_command_finished() {
    let paths = test_paths("bus-first");
    let before_start_ms = u64::try_from(now_unix_ms()).unwrap();
    let running = start_test_daemon(&paths).await;
    let after_start_ms = u64::try_from(now_unix_ms()).unwrap();
    let mut live = subscribe(&running.server.bus);

    let request = execute_request("token.create", r#"{"name": "ci", "preset": "chat-only"}"#);
    let response = run_command(&running.server, &owner_principal(), request).await;
    ok_result(&response);

    let event = tokio::time::timeout(Duration::from_secs(READ_TIMEOUT_SECONDS), live.recv())
        .await
        .expect("no command.finished was published")
        .unwrap();
    assert_eq!(event.event_type, "command.finished");
    assert_eq!(event.subject_id, response.journal_id);
    assert_eq!(event.source, "daemon");
    assert!(event.bus_sequence >= before_start_ms * 1_000, "{}", event.bus_sequence);
    assert!(event.bus_sequence <= after_start_ms * 1_000, "{}", event.bus_sequence);
    let Some(Payload::CommandFinished(finished)) = event.payload.unwrap().payload else {
        panic!("not a CommandFinished payload");
    };
    assert_eq!(finished.command_id, "token.create");
    assert_eq!(finished.principal, "owner");
    assert!(finished.ok);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_needs_read_and_a_revoked_token_s_stream_ends_with_rrs_c1006() {
    let paths = test_paths("bus-tokens");
    let running = start_test_daemon(&paths).await;
    let owner = owner_principal();
    let create = |arguments: &str| execute_request("token.create", arguments);
    let chat_only = ok_result(
        &run_command(
            &running.server,
            &owner,
            create(r#"{"name": "c", "preset": "chat-only"}"#),
        )
        .await,
    );
    let reader = ok_result(&run_command(&running.server, &owner, create(r#"{"name": "r", "scopes": ["read"]}"#)).await);

    let chat_bearer = format!("Bearer {}", chat_only["token"].as_str().unwrap());
    let mut refused = open_event_stream(running.http_port, &chat_bearer, &after(0)).await;
    let end = stream_end(&mut refused).await;
    assert_eq!(end_code(&end), "RRS-C1005");

    let reader_bearer = format!("Bearer {}", reader["token"].as_str().unwrap());
    let mut stream = open_event_stream(running.http_port, &reader_bearer, &after(0)).await;
    let live = publish(&running.server.bus, heartbeat("node_local"));
    assert_eq!(next_event(&mut stream).await.bus_sequence, live);

    let revoke = execute_request("token.revoke", &json!({"token_id": reader["token_id"]}).to_string());
    ok_result(&run_command(&running.server, &owner, revoke).await);
    assert_eq!(end_code(&stream_end(&mut stream).await), "RRS-C1006");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_takes_only_connect_proto() {
    let paths = test_paths("bus-content-type");
    let running = start_test_daemon(&paths).await;
    let headers = [
        ("authorization", owner_bearer(&paths)),
        ("content-type", "application/json".to_string()),
    ];
    let header_pairs: Vec<(&str, &str)> = headers.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let response = send_to_port(running.http_port, "POST", SUBSCRIBE_PATH, &header_pairs, b"{}").await;
    assert_eq!(response.status, 415);
    assert_eq!(header_value(&response, "accept-post"), Some(STREAM_CONTENT_TYPE));

    // A body that isn't one enveloped message ends the stream at once with RRS-C0002.
    let headers = [
        ("authorization", owner_bearer(&paths)),
        ("content-type", STREAM_CONTENT_TYPE.to_string()),
    ];
    let header_pairs: Vec<(&str, &str)> = headers.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let response = send_to_port(running.http_port, "POST", SUBSCRIBE_PATH, &header_pairs, b"\x01\x00").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body[0], END_OF_STREAM_FLAGS);
    let end: Value = serde_json::from_slice(&response.body[ENVELOPE_HEADER_BYTES..]).unwrap();
    assert_eq!(end_code(&end), "RRS-C0002");
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
