//! The plain-HTTP half of the client (layout: rust-node-gateway-proto.md section 4.7): a request over a Unix socket
//! round-trips (daemon.sock, for the CLI and the MCP bridge), a streamed body arrives chunk by chunk, a silent
//! server times out, and the same calls work over loopback TCP. HTTPS, egress tags and TLS refusals join this file
//! with M2-14.
//!
//! The test servers are a few lines of raw HTTP/1.1 rather than a server library, so they also check the exact
//! bytes the client sends.

#[allow(clippy::unwrap_used)]
mod test_server {
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    /// The blank line between an HTTP head and its body.
    const END_OF_HEAD: &[u8] = b"\r\n\r\n";
    const CONTENT_LENGTH: &str = "content-length:";

    /// One request as the server read it: the head as text (lower-cased header names untouched) and the body.
    pub struct ReceivedRequest {
        pub head: String,
        pub body: Vec<u8>,
    }

    /// Reads one request: the head up to the blank line, then Content-Length bytes of body.
    pub async fn read_request<Connection: AsyncRead + Unpin>(connection: &mut Connection) -> ReceivedRequest {
        let mut bytes = Vec::new();
        let head_length = loop {
            if let Some(position) = find(&bytes, END_OF_HEAD) {
                break position + END_OF_HEAD.len();
            }
            let mut buffer = [0; 1024];
            let count = connection.read(&mut buffer).await.unwrap();
            assert!(count > 0, "the client closed before the head ended");
            bytes.extend_from_slice(&buffer[..count]);
        };
        let head = String::from_utf8(bytes[..head_length].to_vec()).unwrap();
        let body_length = content_length(&head);
        while bytes.len() < head_length + body_length {
            let mut buffer = [0; 1024];
            let count = connection.read(&mut buffer).await.unwrap();
            bytes.extend_from_slice(&buffer[..count]);
        }
        ReceivedRequest {
            head,
            body: bytes[head_length..].to_vec(),
        }
    }

    /// Writes a whole response with a Content-Length.
    pub async fn write_response<Connection: AsyncWrite + Unpin>(connection: &mut Connection, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 200 OK\r\nx-served-by: test\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        connection.write_all(head.as_bytes()).await.unwrap();
        connection.write_all(body).await.unwrap();
    }

    fn content_length(head: &str) -> usize {
        for line in head.lines() {
            if line.to_ascii_lowercase().starts_with(CONTENT_LENGTH) {
                return line[CONTENT_LENGTH.len()..].trim().parse().unwrap();
            }
        }
        0
    }

    fn find(bytes: &[u8], wanted: &[u8]) -> Option<usize> {
        bytes.windows(wanted.len()).position(|window| window == wanted)
    }
}

#[allow(clippy::unwrap_used)]
mod over_loopback {
    use rastai_transport::http_client::{HttpRequest, HttpTarget, send_request};

    use crate::test_server::{read_request, write_response};

    #[tokio::test]
    async fn a_request_over_loopback_round_trips() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut connection, _address) = listener.accept().await.unwrap();
            let received = read_request(&mut connection).await;
            write_response(&mut connection, b"pong").await;
            received
        });

        let request = HttpRequest {
            method: "GET".to_string(),
            target: HttpTarget::LoopbackHttp { port },
            path: "/health".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 5000,
        };
        let response = send_request(request).await.unwrap();
        let received = server.await.unwrap();

        assert!(
            received.head.starts_with("GET /health HTTP/1.1\r\n"),
            "{}",
            received.head
        );
        assert!(
            received.head.contains(&format!("host: 127.0.0.1:{port}\r\n")),
            "{}",
            received.head
        );
        assert!(received.body.is_empty());
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"pong");
    }
}

#[cfg(unix)]
#[allow(clippy::unwrap_used)]
mod over_a_unix_socket {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use rastai_transport::http_client::{HttpRequest, HttpTarget, next_body_chunk, open_response_stream, send_request};
    use rastai_transport::local_socket::{LocalListener, LocalStream, accept_local, bind_local_socket};
    use tokio::io::AsyncWriteExt;

    use crate::test_server::{read_request, write_response};

    const SOCKET_FILE: &str = "daemon.sock";
    const SOCKET_FOLDER: &str = "run";
    const REQUEST_BODY: &[u8] = br#"{"command_id":"system.health"}"#;
    const RESPONSE_BODY: &[u8] = br#"{"ok":true}"#;
    /// Long enough for any test machine, short enough that the timeout test stays quick.
    const SILENT_SERVER_TIMEOUT_MS: u64 = 300;

    static NEXT_FOLDER_NUMBER: AtomicU32 = AtomicU32::new(0);

    fn new_socket_path() -> (PathBuf, PathBuf) {
        let number = NEXT_FOLDER_NUMBER.fetch_add(1, Ordering::SeqCst);
        let test_folder = std::env::temp_dir().join(format!("rth-{}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&test_folder);
        fs::create_dir_all(&test_folder).unwrap();
        let socket_path = test_folder.join(SOCKET_FOLDER).join(SOCKET_FILE);
        (test_folder, socket_path)
    }

    fn request_to(socket_path: &Path, method: &str, body: &[u8], timeout_ms: u64) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            target: HttpTarget::LocalSocket {
                socket_path: socket_path.to_path_buf(),
            },
            path: "/api/execute".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: body.to_vec(),
            timeout_ms,
        }
    }

    async fn accept_one(listener: &LocalListener) -> tokio::net::UnixStream {
        let (stream, _peer) = accept_local(listener).await.unwrap();
        match stream {
            LocalStream::Unix(unix_stream) => unix_stream,
        }
    }

    #[tokio::test]
    async fn a_request_over_a_unix_socket_round_trips() {
        let (test_folder, socket_path) = new_socket_path();
        let listener = bind_local_socket(&socket_path).unwrap();
        let server = tokio::spawn(async move {
            let mut connection = accept_one(&listener).await;
            let received = read_request(&mut connection).await;
            write_response(&mut connection, RESPONSE_BODY).await;
            received
        });

        let response = send_request(request_to(&socket_path, "POST", REQUEST_BODY, 5000))
            .await
            .unwrap();
        let received = server.await.unwrap();

        assert!(
            received.head.starts_with("POST /api/execute HTTP/1.1\r\n"),
            "{}",
            received.head
        );
        assert!(received.head.contains("host: localhost\r\n"), "{}", received.head);
        assert!(
            received.head.contains("content-type: application/json\r\n"),
            "{}",
            received.head
        );
        assert_eq!(received.body, REQUEST_BODY);
        assert_eq!(response.status, 200);
        assert!(
            response
                .headers
                .contains(&("x-served-by".to_string(), "test".to_string()))
        );
        assert_eq!(response.body, RESPONSE_BODY);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_chunked_body_arrives_chunk_by_chunk() {
        let (test_folder, socket_path) = new_socket_path();
        let listener = bind_local_socket(&socket_path).unwrap();
        let (second_chunk_sender, second_chunk_receiver) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let mut connection = accept_one(&listener).await;
            read_request(&mut connection).await;
            let head = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nfirst\r\n";
            connection.write_all(head.as_bytes()).await.unwrap();
            // The second chunk waits until the client has read the first, so they can't arrive together.
            second_chunk_receiver.await.unwrap();
            connection.write_all(b"6\r\nsecond\r\n0\r\n\r\n").await.unwrap();
        });

        let mut stream = open_response_stream(request_to(&socket_path, "GET", b"", 5000))
            .await
            .unwrap();
        let first = next_body_chunk(&mut stream).await.unwrap();
        second_chunk_sender.send(()).unwrap();
        let second = next_body_chunk(&mut stream).await.unwrap();
        let end = next_body_chunk(&mut stream).await.unwrap();

        assert_eq!(stream.status, 200);
        assert_eq!(first, Some(b"first".to_vec()));
        assert_eq!(second, Some(b"second".to_vec()));
        assert_eq!(end, None);
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn a_silent_server_times_out_with_rrs_n2008() {
        let (test_folder, socket_path) = new_socket_path();
        let listener = bind_local_socket(&socket_path).unwrap();
        tokio::spawn(async move {
            let mut connection = accept_one(&listener).await;
            read_request(&mut connection).await;
            // Never answers; holds the connection open until the test ends.
            std::future::pending::<()>().await;
        });

        let refusal = send_request(request_to(&socket_path, "GET", b"", SILENT_SERVER_TIMEOUT_MS)).await;
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2008"));
        fs::remove_dir_all(&test_folder).unwrap();
    }

    #[tokio::test]
    async fn nothing_listening_on_the_socket_is_rrs_n2004() {
        let (test_folder, socket_path) = new_socket_path();
        let refusal = send_request(request_to(&socket_path, "GET", b"", 5000)).await;
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2004"));
        fs::remove_dir_all(&test_folder).unwrap();
    }
}
