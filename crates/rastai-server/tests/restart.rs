//! A restart right after a stop in one process (issue #250). Two things can make it meet `RRS-S0001`:
//! - a task of the stopped daemon still holding the server, which must not happen: a stop returns only once the lock
//!   is free, and a Subscribe stream still open at the stop must not keep it (the first test);
//! - a child process of another test in this process holding the lock file it inherited between its fork and its exec
//!   for a moment, which the stop can't prevent. A start that expects a refusal must wait that out (the second test).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use prost::Message;
use rastai_core::paths::{daemon_lock_path, owner_token_path};
use rastai_proto::events::v1::SubscribeRequest;
use rastai_server::daemon::start_daemon;
use rastai_server::http::connect::stream_message_envelope;
use rastai_store::data_dir_lock::take_daemon_lock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use common::{
    options_on_any_port, owner_bearer, refused_start_in_test, remove_test_folder, start_test_daemon, stop_test_daemon,
    test_paths,
};

const SUBSCRIBE_PATH: &str = "/rastai.events.v1.EventService/Subscribe";
/// How long the test waits for the response head of the stream it opened.
const HEAD_WAIT: Duration = Duration::from_secs(5);
/// How long the stand-in for a child's inherited lock file keeps the lock: well inside the test's own wait.
const CHILD_HOLDS_LOCK_FOR: Duration = Duration::from_millis(300);
/// The mode a token must not have (startup.rs refuses it with RRS-C1008).
const OPEN_TOKEN_MODE: u32 = 0o644;

/// Opens a Subscribe stream and reads its response head, then keeps the socket open the way a live client does.
async fn open_stream_and_keep_it(port: u16, authorization: &str) -> TcpStream {
    let body = stream_message_envelope(&SubscribeRequest::default().encode_to_vec());
    let head = format!(
        "POST {SUBSCRIBE_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {authorization}\r\n\
         Content-Type: application/connect+proto\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    let mut received = Vec::new();
    let mut buffer = [0u8; 1024];
    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = tokio::time::timeout(HEAD_WAIT, socket.read(&mut buffer))
            .await
            .expect("the stream sent no response head")
            .unwrap();
        assert!(read > 0, "the stream was closed before its head");
        received.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&received).to_ascii_lowercase();
    assert!(text.starts_with("http/1.1 200"), "{text}");
    socket
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_with_a_subscribe_stream_open_gets_the_lock_back_at_once() {
    let paths = test_paths("restart-stream");
    let running = start_test_daemon(&paths).await;
    let stream = open_stream_and_keep_it(running.http_port, &owner_bearer(&paths)).await;
    stop_test_daemon(running).await;

    // The stop returned, so the lock is free now: no retry, no wait.
    let relocked = take_daemon_lock(&paths);
    assert!(relocked.is_ok(), "the lock was still held after the stop");
    drop(relocked);

    let restarted = start_daemon(paths.clone(), &options_on_any_port())
        .await
        .expect("the restart after a stop with an open stream");
    drop(stream);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

/// Holds the daemon lock from a second open file, the way a child process holds the lock file it inherited between
/// its fork and its exec, and lets go after `CHILD_HOLDS_LOCK_FOR`.
fn child_holds_the_lock_for_a_moment(paths: &rastai_core::paths::DataPaths) {
    let inherited: File = OpenOptions::new()
        .read(true)
        .write(true)
        .open(daemon_lock_path(paths))
        .unwrap();
    inherited.try_lock().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(CHILD_HOLDS_LOCK_FOR);
        drop(inherited);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_start_waits_out_a_lock_a_child_still_holds_from_the_stop() {
    let paths = test_paths("forked-holder");
    let running = start_test_daemon(&paths).await;
    stop_test_daemon(running).await;
    let token_path = owner_token_path(&paths);
    fs::set_permissions(&token_path, fs::Permissions::from_mode(OPEN_TOKEN_MODE)).unwrap();

    child_holds_the_lock_for_a_moment(&paths);
    // The refusal is the token's mode, not the lock a child held for a moment: RRS-C1008, never RRS-S0001.
    let refused = refused_start_in_test(&paths, &options_on_any_port()).await;
    assert_eq!(refused.code(), "RRS-C1008");
    remove_test_folder(&paths);
}
