//! The one hyper wrapper (contracts 2.5; layout: rust-node-gateway-proto.md section 4.2). M1 has its plain-HTTP
//! half: HTTP/1.1 over a local socket (daemon.sock, for the CLI and the MCP bridge) and over loopback TCP
//! (llama-server, the daemon's own port). HTTPS through rustls, the `Https` target and its `EgressTag` arrive with
//! M2-14 in this same file.
//!
//! One connection per request and no pool: local connections cost almost nothing to open. No retries either; the
//! caller decides (section 4.6).

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use hyper::header::{HOST, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, Uri};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{TransportError, connection_failed};
use crate::local_socket::{LocalStream, connect_local_socket};

/// Loopback targets are always this address: this crate never talks plain HTTP to another machine (section 4.6).
const LOOPBACK_ADDRESS: &str = "127.0.0.1";
/// The Host header over a local socket. The daemon's request gate accepts any loopback host name (contracts 3.3).
const LOCAL_SOCKET_HOST: &str = "localhost";

/// Where a request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpTarget {
    /// Plain HTTP to 127.0.0.1 on this port: llama-server, the daemon's own HTTP port.
    LoopbackHttp { port: u16 },
    /// Plain HTTP over a local socket: daemon.sock (`rastai_core::paths::daemon_socket_path`).
    LocalSocket { socket_path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub target: HttpTarget,
    /// The path and query, "/api/health".
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// `send_request`: the whole exchange. `open_response_stream`: connecting and the response head; then each
    /// `next_body_chunk` waits at most this long for the next chunk.
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A response whose body is read chunk by chunk as it arrives (`next_body_chunk`).
#[derive(Debug)]
pub struct HttpStream {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    body: Incoming,
    timeout_ms: u64,
}

/// Sends the request and reads the whole response. A slow exchange is `TimedOut` (RRS-N2008).
pub async fn send_request(request: HttpRequest) -> Result<HttpResponse, TransportError> {
    let timeout_ms = request.timeout_ms;
    with_timeout(timeout_ms, read_whole_response(request)).await
}

/// Sends the request and returns once the response head has arrived; the body follows through `next_body_chunk`.
pub async fn open_response_stream(request: HttpRequest) -> Result<HttpStream, TransportError> {
    let timeout_ms = request.timeout_ms;
    with_timeout(timeout_ms, open_stream(request)).await
}

/// The next piece of the body, or None at its end. Waits at most the request's `timeout_ms` (RRS-N2008).
pub async fn next_body_chunk(stream: &mut HttpStream) -> Result<Option<Vec<u8>>, TransportError> {
    let timeout_ms = stream.timeout_ms;
    with_timeout(timeout_ms, next_data_frame(&mut stream.body)).await
}

async fn read_whole_response(request: HttpRequest) -> Result<HttpResponse, TransportError> {
    let mut stream = open_stream(request).await?;
    let mut body = Vec::new();
    while let Some(chunk) = next_data_frame(&mut stream.body).await? {
        body.extend_from_slice(&chunk);
    }
    Ok(HttpResponse {
        status: stream.status,
        headers: stream.headers,
        body,
    })
}

async fn open_stream(request: HttpRequest) -> Result<HttpStream, TransportError> {
    let host = host_header(&request.target);
    let http_request = build_http_request(&request, &host)?;
    let response = match &request.target {
        HttpTarget::LoopbackHttp { port } => {
            let tcp_stream = connect_loopback(*port).await?;
            exchange(tcp_stream, http_request).await?
        }
        HttpTarget::LocalSocket { socket_path } => {
            let local_stream = connect_local_socket(socket_path).await?;
            exchange_on_local_stream(local_stream, http_request).await?
        }
    };
    Ok(stream_from_response(response, request.timeout_ms))
}

/// Loopback TCP with TCP_NODELAY: without it, small streamed chunks stalled ~40 ms each (spike 07).
async fn connect_loopback(port: u16) -> Result<tokio::net::TcpStream, TransportError> {
    let stream = tokio::net::TcpStream::connect((LOOPBACK_ADDRESS, port))
        .await
        .map_err(|error| connection_failed(format!("connecting to {LOOPBACK_ADDRESS}:{port}: {error}")))?;
    stream
        .set_nodelay(true)
        .map_err(|error| connection_failed(format!("setting TCP_NODELAY: {error}")))?;
    Ok(stream)
}

#[cfg(unix)]
async fn exchange_on_local_stream(
    local_stream: LocalStream,
    http_request: Request<RequestBody>,
) -> Result<Response<Incoming>, TransportError> {
    match local_stream {
        LocalStream::Unix(unix_stream) => exchange(unix_stream, http_request).await,
    }
}

#[cfg(not(unix))]
async fn exchange_on_local_stream(
    local_stream: LocalStream,
    _http_request: Request<RequestBody>,
) -> Result<Response<Incoming>, TransportError> {
    // No LocalStream can exist here yet (see the enum in local_socket.rs), so there is nothing to match.
    match local_stream {}
}

/// One HTTP/1.1 exchange on an open connection: loopback TCP and a Unix socket are both just byte streams here.
async fn exchange<Connection>(
    connection: Connection,
    http_request: Request<RequestBody>,
) -> Result<Response<Incoming>, TransportError>
where
    Connection: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, driver) = hyper::client::conn::http1::handshake(TokioIo::new(connection))
        .await
        .map_err(|error| connection_failed(format!("HTTP handshake: {error}")))?;
    // hyper reads and writes the socket in this task; it ends when the response body is read or dropped. Its own
    // errors also reach the caller through send_request or the body, so its result is not needed here.
    tokio::spawn(driver);
    sender
        .send_request(http_request)
        .await
        .map_err(|error| connection_failed(format!("sending the request: {error}")))
}

fn host_header(target: &HttpTarget) -> String {
    match target {
        HttpTarget::LoopbackHttp { port } => format!("{LOOPBACK_ADDRESS}:{port}"),
        HttpTarget::LocalSocket { .. } => LOCAL_SOCKET_HOST.to_string(),
    }
}

/// The hyper request: method, path, Host, the caller's headers, the body. A bad method, path or header is a bug in
/// the caller, reported as RRS-N2008 with the part that is wrong.
fn build_http_request(request: &HttpRequest, host: &str) -> Result<Request<RequestBody>, TransportError> {
    let mut http_request = Request::new(RequestBody::new(request.body.clone()));
    *http_request.method_mut() = Method::from_bytes(request.method.as_bytes())
        .map_err(|_| connection_failed(format!("invalid HTTP method {:?}", request.method)))?;
    *http_request.uri_mut() = request
        .path
        .parse::<Uri>()
        .map_err(|_| connection_failed(format!("invalid request path {:?}", request.path)))?;
    let headers = http_request.headers_mut();
    headers.insert(HOST, header_value(host)?);
    for (name, value) in &request.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| connection_failed(format!("invalid header name {name:?}")))?;
        headers.append(header_name, header_value(value)?);
    }
    Ok(http_request)
}

fn header_value(value: &str) -> Result<HeaderValue, TransportError> {
    HeaderValue::from_str(value).map_err(|_| connection_failed(format!("invalid header value {value:?}")))
}

fn stream_from_response(response: Response<Incoming>, timeout_ms: u64) -> HttpStream {
    let status = response.status().as_u16();
    let mut headers = Vec::new();
    for (name, value) in response.headers() {
        // A header that isn't UTF-8 keeps its readable part rather than failing the whole response.
        headers.push((
            name.as_str().to_string(),
            String::from_utf8_lossy(value.as_bytes()).into_owned(),
        ));
    }
    HttpStream {
        status,
        headers,
        body: response.into_body(),
        timeout_ms,
    }
}

/// The next data frame of the body; trailers, which nothing here uses, are skipped.
async fn next_data_frame(body: &mut Incoming) -> Result<Option<Vec<u8>>, TransportError> {
    loop {
        let frame = std::future::poll_fn(|context| Pin::new(&mut *body).poll_frame(context)).await;
        match frame {
            None => return Ok(None),
            Some(Err(error)) => return Err(connection_failed(format!("reading the response body: {error}"))),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    return Ok(Some(data.to_vec()));
                }
            }
        }
    }
}

async fn with_timeout<Value>(
    timeout_ms: u64,
    work: impl Future<Output = Result<Value, TransportError>>,
) -> Result<Value, TransportError> {
    match tokio::time::timeout(Duration::from_millis(timeout_ms), work).await {
        Ok(result) => result,
        Err(_elapsed) => Err(TransportError::TimedOut { timeout_ms }),
    }
}

/// A request body sent as one frame. hyper needs a type that implements its `Body`; the crate that has a ready one
/// (http-body-util) isn't on the §27 list, and this is all of it we need.
struct RequestBody {
    bytes: Option<Bytes>,
    length: u64,
}

impl RequestBody {
    fn new(body: Vec<u8>) -> RequestBody {
        let length = body.len() as u64;
        // An empty body has no frame at all, so a GET carries no Content-Length.
        let bytes = if body.is_empty() { None } else { Some(Bytes::from(body)) };
        RequestBody { bytes, length }
    }
}

impl Body for RequestBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let next = self.bytes.take().map(|bytes| Ok(Frame::data(bytes)));
        Poll::Ready(next)
    }

    fn is_end_stream(&self) -> bool {
        self.bytes.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.length)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, headers: Vec<(String, String)>) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            target: HttpTarget::LoopbackHttp { port: 8080 },
            path: path.to_string(),
            headers,
            body: b"{}".to_vec(),
            timeout_ms: 1000,
        }
    }

    #[test]
    fn the_request_carries_method_path_host_and_headers() {
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        let built = build_http_request(&request("POST", "/api/health?verbose=1", headers), "127.0.0.1:8080").unwrap();
        assert_eq!(built.method(), Method::POST);
        assert_eq!(built.uri().to_string(), "/api/health?verbose=1");
        assert_eq!(built.headers()["host"], "127.0.0.1:8080");
        assert_eq!(built.headers()["content-type"], "application/json");
        assert_eq!(built.body().size_hint().exact(), Some(2));
    }

    #[test]
    fn a_bad_header_name_is_refused_with_rrs_n2008() {
        let headers = vec![("bad header".to_string(), "x".to_string())];
        let refusal = build_http_request(&request("GET", "/", headers), "localhost").map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2008"));
    }

    #[test]
    fn a_bad_method_is_refused_with_rrs_n2008() {
        let refusal = build_http_request(&request("GE T", "/", Vec::new()), "localhost").map(|_| ());
        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2008"));
    }

    #[test]
    fn the_host_is_the_loopback_address_or_localhost() {
        assert_eq!(
            host_header(&HttpTarget::LoopbackHttp { port: 47820 }),
            "127.0.0.1:47820"
        );
        let socket = HttpTarget::LocalSocket {
            socket_path: PathBuf::from("/run/user/1000/rastai/daemon.sock"),
        };
        assert_eq!(host_header(&socket), "localhost");
    }

    #[test]
    fn an_empty_body_has_no_frame() {
        let body = RequestBody::new(Vec::new());
        assert!(body.is_end_stream());
        assert_eq!(body.size_hint().exact(), Some(0));
    }
}
