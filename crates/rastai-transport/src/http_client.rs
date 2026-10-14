//! The one hyper and rustls wrapper (contracts 2.5; layout: rust-node-gateway-proto.md section 4.2). Plain HTTP/1.1 over
//! a local socket (daemon.sock, for the CLI and the MCP bridge) and over loopback TCP (llama-server, the daemon's own
//! port), and HTTPS through rustls, verified against webpki-roots (M2-14). Every HTTPS request carries an `EgressTag`,
//! so each connection is reported and the UI can show what leaves the machine.
//!
//! One connection per request and no pool: local connections cost almost nothing to open, and an HTTPS handshake per
//! request is the simple choice. No retries either; the caller decides (section 4.6).

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use hyper::header::{HOST, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, Uri};
use hyper_util::rt::TokioIo;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::error::{TransportError, connection_failed};
use crate::local_socket::{LocalStream, connect_local_socket};

/// Loopback targets are always this address: this crate never talks plain HTTP to another machine (section 4.6).
const LOOPBACK_ADDRESS: &str = "127.0.0.1";
/// The Host header over a local socket. The daemon's request gate accepts any loopback host name (contracts 3.3).
const LOCAL_SOCKET_HOST: &str = "localhost";
/// The port an HTTPS URL leaves out, so the Host header leaves it out too.
const HTTPS_DEFAULT_PORT: u16 = 443;
/// How often an HTTPS request reports itself while its body streams (section 4.6). The UI's egress badge expects a
/// refresh at least this often.
pub const EGRESS_REPORT_INTERVAL_SECONDS: u64 = 5;

/// Where a request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpTarget {
    /// HTTPS to a host name or address, verified against webpki-roots: Hugging Face, the API providers.
    Https { host: String, port: u16 },
    /// Plain HTTP to 127.0.0.1 on this port: llama-server, the daemon's own HTTP port.
    LoopbackHttp { port: u16 },
    /// Plain HTTP over a local socket: daemon.sock (`rastai_core::paths::daemon_socket_path`).
    LocalSocket { socket_path: PathBuf },
}

/// Who is sending data off the machine and why. Every Https request carries one; the owner of the channel (the
/// daemon's bus or the node's WatchLive) turns each report into an ephemeral egress.active event.
#[derive(Debug, Clone)]
pub struct EgressTag {
    pub purpose: String,
    pub subject_id: String,
    pub reports: mpsc::Sender<EgressReport>,
}

/// One connection's report: the purpose and subject from its tag, and the host it connects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressReport {
    pub purpose: String,
    pub host: String,
    pub subject_id: String,
}

#[derive(Debug, Clone)]
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
    /// Required for an Https target (RRS-N3008 without it). Local targets ignore it.
    pub egress: Option<EgressTag>,
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
    egress: Option<EgressReporting>,
}

/// What an HTTPS body keeps after its connection opens: the tag to report through, the host, and when the last report
/// went out.
#[derive(Debug)]
struct EgressReporting {
    tag: EgressTag,
    host: String,
    last_report_at: Instant,
}

/// Sends the request and reads the whole response. A slow exchange is `TimedOut` (RRS-N2008).
pub async fn send_request(request: HttpRequest) -> Result<HttpResponse, TransportError> {
    send_request_with_roots(request, public_roots()).await
}

/// Sends the request and returns once the response head has arrived; the body follows through `next_body_chunk`.
pub async fn open_response_stream(request: HttpRequest) -> Result<HttpStream, TransportError> {
    open_response_stream_with_roots(request, public_roots()).await
}

/// The next piece of the body, or None at its end. Waits at most the request's `timeout_ms` (RRS-N2008).
pub async fn next_body_chunk(stream: &mut HttpStream) -> Result<Option<Vec<u8>>, TransportError> {
    let timeout_ms = stream.timeout_ms;
    with_timeout(timeout_ms, read_next_chunk(stream)).await
}

/// `send_request` against the given root store. Production passes the public roots; tests pass a test CA.
async fn send_request_with_roots(request: HttpRequest, roots: RootCertStore) -> Result<HttpResponse, TransportError> {
    let timeout_ms = request.timeout_ms;
    with_timeout(timeout_ms, read_whole_response(request, roots)).await
}

/// `open_response_stream` against the given root store, as `send_request_with_roots` is.
async fn open_response_stream_with_roots(
    request: HttpRequest,
    roots: RootCertStore,
) -> Result<HttpStream, TransportError> {
    let timeout_ms = request.timeout_ms;
    with_timeout(timeout_ms, open_stream(request, roots)).await
}

async fn read_whole_response(request: HttpRequest, roots: RootCertStore) -> Result<HttpResponse, TransportError> {
    let mut stream = open_stream(request, roots).await?;
    let mut body = Vec::new();
    while let Some(chunk) = read_next_chunk(&mut stream).await? {
        body.extend_from_slice(&chunk);
    }
    Ok(HttpResponse {
        status: stream.status,
        headers: stream.headers,
        body,
    })
}

async fn open_stream(request: HttpRequest, roots: RootCertStore) -> Result<HttpStream, TransportError> {
    let header_host = host_header(&request.target);
    let http_request = build_http_request(&request, &header_host)?;
    let timeout_ms = request.timeout_ms;
    let (response, egress) = match &request.target {
        HttpTarget::Https { host, port } => {
            open_https(host, *port, request.egress.as_ref(), roots, http_request).await?
        }
        HttpTarget::LoopbackHttp { port } => {
            let tcp_stream = connect_tcp(LOOPBACK_ADDRESS, *port).await?;
            (exchange(tcp_stream, http_request).await?, None)
        }
        HttpTarget::LocalSocket { socket_path } => {
            let local_stream = connect_local_socket(socket_path).await?;
            (exchange_on_local_stream(local_stream, http_request).await?, None)
        }
    };
    Ok(stream_from_response(response, timeout_ms, egress))
}

/// An HTTPS request: the tag is checked first, so an untagged request connects to nothing. The tag gets its first
/// report as soon as the TCP connection opens, before the handshake (section 4.6).
async fn open_https(
    host: &str,
    port: u16,
    egress: Option<&EgressTag>,
    roots: RootCertStore,
    http_request: Request<RequestBody>,
) -> Result<(Response<Incoming>, Option<EgressReporting>), TransportError> {
    let tag = egress.ok_or_else(|| TransportError::EgressTagMissing { host: host.to_string() })?;
    let tcp_stream = connect_tcp(host, port).await?;
    send_egress_report(tag, host);
    let tls_stream = start_tls(tcp_stream, host, port, roots).await?;
    let response = exchange(tls_stream, http_request).await?;
    let reporting = EgressReporting {
        tag: tag.clone(),
        host: host.to_string(),
        last_report_at: Instant::now(),
    };
    Ok((response, Some(reporting)))
}

/// TCP with TCP_NODELAY: without it, small streamed chunks stalled ~40 ms each (spike 07).
async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, TransportError> {
    let stream = TcpStream::connect((host, port))
        .await
        .map_err(|error| connection_failed(format!("connecting to {host}:{port}: {error}")))?;
    stream
        .set_nodelay(true)
        .map_err(|error| connection_failed(format!("setting TCP_NODELAY: {error}")))?;
    Ok(stream)
}

/// The TLS handshake. The certificate must be trusted by `roots` and name `host`; anything else is refused here, before
/// a request is sent.
async fn start_tls(
    tcp_stream: TcpStream,
    host: &str,
    port: u16,
    roots: RootCertStore,
) -> Result<TlsStream<TcpStream>, TransportError> {
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|error| connection_failed(format!("invalid host name {host:?}: {error}")))?;
    let connector = TlsConnector::from(Arc::new(client_config(roots)?));
    connector.connect(server_name, tcp_stream).await.map_err(|error| {
        // A certificate that isn't trusted or doesn't name this host lands here too, and is refused.
        connection_failed(format!("TLS handshake with {host}:{port} failed: {error}"))
    })
}

/// The client settings: TLS 1.2 and 1.3 with the ring provider, verifying against `roots` and sending no client
/// certificate. The provider is named here so that aws-lc-rs never enters the build (Cargo.toml).
fn client_config(roots: RootCertStore) -> Result<rustls::ClientConfig, TransportError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| connection_failed(format!("TLS setup: {error}")))?;
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

/// The public trust store every production HTTPS request verifies against: Mozilla's roots (webpki-roots).
fn public_roots() -> RootCertStore {
    RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
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

/// One HTTP/1.1 exchange on an open connection: loopback TCP, a Unix socket and TLS are all just byte streams here.
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
        HttpTarget::Https { host, port } => {
            if *port == HTTPS_DEFAULT_PORT {
                host.clone()
            } else {
                format!("{host}:{port}")
            }
        }
        HttpTarget::LoopbackHttp { port } => format!("{LOOPBACK_ADDRESS}:{port}"),
        HttpTarget::LocalSocket { .. } => LOCAL_SOCKET_HOST.to_string(),
    }
}

/// The hyper request: method, path, Host, the caller's headers, the body. A Host the caller gives replaces ours,
/// since two Host headers make a request every HTTP/1.1 server must refuse. A bad method, path or header is a bug
/// in the caller, reported as RRS-N2008 with the part that is wrong.
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
        let value = header_value(value)?;
        if header_name == HOST {
            headers.insert(header_name, value);
        } else {
            headers.append(header_name, value);
        }
    }
    Ok(http_request)
}

fn header_value(value: &str) -> Result<HeaderValue, TransportError> {
    HeaderValue::from_str(value).map_err(|_| connection_failed(format!("invalid header value {value:?}")))
}

fn stream_from_response(response: Response<Incoming>, timeout_ms: u64, egress: Option<EgressReporting>) -> HttpStream {
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
        egress,
    }
}

/// The next chunk, reporting the connection first when the interval is up. The timeout is the caller's to add.
async fn read_next_chunk(stream: &mut HttpStream) -> Result<Option<Vec<u8>>, TransportError> {
    if let Some(reporting) = stream.egress.as_mut() {
        reporting.report_if_due(Instant::now());
    }
    next_data_frame(&mut stream.body).await
}

/// The next data frame of the body; trailers, which nothing here uses, are skipped.
async fn next_data_frame(body: &mut Incoming) -> Result<Option<Vec<u8>>, TransportError> {
    loop {
        // hyper's Body only has poll_frame; the awaitable `frame()` lives in http-body-util, which isn't on the §27
        // list, so poll_fn turns the poll into a future here.
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

impl EgressReporting {
    /// Sends a report when the interval since the last one has passed. Called before each body chunk is read.
    fn report_if_due(&mut self, now: Instant) {
        if now.duration_since(self.last_report_at) >= Duration::from_secs(EGRESS_REPORT_INTERVAL_SECONDS) {
            send_egress_report(&self.tag, &self.host);
            self.last_report_at = now;
        }
    }
}

/// One report on the tag's channel. The channel is bounded and a report is plain data, so a full channel drops this
/// one: the badge refreshes again within the interval, and a slow reporter must never stall a download (section 4.6).
/// A closed channel means nobody listens any more, which doesn't fail the request either.
fn send_egress_report(tag: &EgressTag, host: &str) {
    let report = EgressReport {
        purpose: tag.purpose.clone(),
        host: host.to_string(),
        subject_id: tag.subject_id.clone(),
    };
    let _ = tag.reports.try_send(report);
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

    /// How many reports the test channels hold. A report is small and the test reads them as they come.
    const REPORT_CHANNEL_CAPACITY: usize = 8;

    fn request(method: &str, path: &str, headers: Vec<(String, String)>) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            target: HttpTarget::LoopbackHttp { port: 8080 },
            path: path.to_string(),
            headers,
            body: b"{}".to_vec(),
            timeout_ms: 1000,
            egress: None,
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
    fn a_host_header_from_the_caller_replaces_ours() {
        let headers = vec![("Host".to_string(), "localhost:47820".to_string())];
        let built = build_http_request(&request("GET", "/", headers), "127.0.0.1:8080").unwrap();
        let hosts: Vec<_> = built.headers().get_all(HOST).iter().collect();
        assert_eq!(hosts, vec!["localhost:47820"]);
    }

    #[test]
    fn repeated_headers_other_than_host_are_all_kept() {
        let headers = vec![
            ("Accept".to_string(), "text/plain".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
        ];
        let built = build_http_request(&request("GET", "/", headers), "127.0.0.1:8080").unwrap();
        assert_eq!(built.headers().get_all("accept").iter().count(), 2);
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
    fn the_host_of_an_https_target_leaves_out_the_default_port() {
        let default_port = HttpTarget::Https {
            host: "huggingface.co".to_string(),
            port: 443,
        };
        let other_port = HttpTarget::Https {
            host: "localhost".to_string(),
            port: 8443,
        };
        assert_eq!(host_header(&default_port), "huggingface.co");
        assert_eq!(host_header(&other_port), "localhost:8443");
    }

    #[test]
    fn an_empty_body_has_no_frame() {
        let body = RequestBody::new(Vec::new());
        assert!(body.is_end_stream());
        assert_eq!(body.size_hint().exact(), Some(0));
    }

    #[test]
    fn a_report_is_sent_once_the_interval_has_passed() {
        let (sender, mut reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);
        let tag = EgressTag {
            purpose: "hf_download".to_string(),
            subject_id: "download-1".to_string(),
            reports: sender,
        };
        let start = Instant::now();
        let mut reporting = EgressReporting {
            tag,
            host: "huggingface.co".to_string(),
            last_report_at: start,
        };
        let interval = Duration::from_secs(EGRESS_REPORT_INTERVAL_SECONDS);

        reporting.report_if_due(start + interval - Duration::from_millis(1));
        assert!(reports.try_recv().is_err(), "a report before the interval is due");

        reporting.report_if_due(start + interval);
        let report = reports.try_recv().unwrap();
        assert_eq!(report.host, "huggingface.co");
        assert_eq!(report.purpose, "hf_download");

        reporting.report_if_due(start + interval + Duration::from_millis(1));
        assert!(
            reports.try_recv().is_err(),
            "the next report waits a full interval from the last one"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tls_tests {
    //! The HTTPS tests. They sit here, not in tests/, because rustls and tokio-rustls may be imported only by this
    //! file (contracts 2.5, check-imports). The test server presents a certificate for `localhost` that a test CA
    //! signed; the fixtures are PEM files in tests/fixtures/tls, made with `openssl req -x509` and `openssl x509
    //! -req` (EC P-256, 20 years). The test CA's private key is not kept: nothing here signs a new certificate.

    use super::*;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    const TEST_CA_PEM: &str = include_str!("../tests/fixtures/tls/test-ca-cert.pem");
    const TEST_SERVER_CERT_PEM: &str = include_str!("../tests/fixtures/tls/test-server-cert.pem");
    const TEST_SERVER_KEY_PEM: &str = include_str!("../tests/fixtures/tls/test-server-key.pem");
    /// The one name the test certificate covers.
    const TEST_HOST: &str = "localhost";
    const REPORT_CHANNEL_CAPACITY: usize = 8;
    /// Long enough for any test machine, short enough that the timeout test stays quick.
    const SILENT_SERVER_TIMEOUT_MS: u64 = 300;
    const REQUEST_TIMEOUT_MS: u64 = 5000;

    fn test_ca_roots() -> RootCertStore {
        let mut roots = RootCertStore::empty();
        for certificate in CertificateDer::pem_slice_iter(TEST_CA_PEM.as_bytes()) {
            roots.add(certificate.unwrap()).unwrap();
        }
        roots
    }

    fn test_server_acceptor() -> TlsAcceptor {
        let certificates: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(TEST_SERVER_CERT_PEM.as_bytes())
                .collect::<Result<_, _>>()
                .unwrap();
        let key = PrivateKeyDer::from_pem_slice(TEST_SERVER_KEY_PEM.as_bytes()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certificates, key)
            .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }

    /// Reads a request head up to its blank line. These tests send only GET requests, which have no body.
    async fn read_head<Connection: AsyncRead + Unpin>(connection: &mut Connection) -> String {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0_u8; 1];
            let count = connection.read(&mut byte).await.unwrap();
            if count == 0 {
                break;
            }
            bytes.push(byte[0]);
        }
        String::from_utf8(bytes).unwrap()
    }

    fn tag_reporting_to(reports: mpsc::Sender<EgressReport>) -> EgressTag {
        EgressTag {
            purpose: "hf_download".to_string(),
            subject_id: "download-1".to_string(),
            reports,
        }
    }

    fn https_request(host: &str, port: u16, egress: Option<EgressTag>, timeout_ms: u64) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            target: HttpTarget::Https {
                host: host.to_string(),
                port,
            },
            path: "/health".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms,
            egress,
        }
    }

    /// A TCP listener on loopback, and its port. The server runs its own task, so each test spawns one.
    async fn listen() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    #[tokio::test]
    async fn a_https_request_round_trips_with_a_trusted_test_ca() {
        let (listener, port) = listen().await;
        let server = tokio::spawn(async move {
            let (tcp, _peer) = listener.accept().await.unwrap();
            let mut tls = test_server_acceptor().accept(tcp).await.unwrap();
            let head = read_head(&mut tls).await;
            tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\npong")
                .await
                .unwrap();
            tls.flush().await.unwrap();
            head
        });
        let (reports_to, mut reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);

        let response = send_request_with_roots(
            https_request(TEST_HOST, port, Some(tag_reporting_to(reports_to)), REQUEST_TIMEOUT_MS),
            test_ca_roots(),
        )
        .await
        .unwrap();
        let head = server.await.unwrap();

        assert!(head.starts_with("GET /health HTTP/1.1\r\n"), "{head}");
        assert!(head.contains(&format!("host: localhost:{port}\r\n")), "{head}");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"pong");
        // The first report went out when the TCP connection opened, before the response came back.
        let report = reports.try_recv().unwrap();
        assert_eq!(
            report,
            EgressReport {
                purpose: "hf_download".to_string(),
                host: TEST_HOST.to_string(),
                subject_id: "download-1".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn a_chunked_https_body_arrives_chunk_by_chunk() {
        let (listener, port) = listen().await;
        let (second_chunk_sender, second_chunk_receiver) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let (tcp, _peer) = listener.accept().await.unwrap();
            let mut tls = test_server_acceptor().accept(tcp).await.unwrap();
            read_head(&mut tls).await;
            let head = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nfirst\r\n";
            tls.write_all(head.as_bytes()).await.unwrap();
            tls.flush().await.unwrap();
            // The second chunk waits until the client has read the first, so they can't arrive together.
            second_chunk_receiver.await.unwrap();
            tls.write_all(b"6\r\nsecond\r\n0\r\n\r\n").await.unwrap();
            tls.flush().await.unwrap();
        });
        let (reports_to, _reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);

        let mut stream = open_response_stream_with_roots(
            https_request(TEST_HOST, port, Some(tag_reporting_to(reports_to)), REQUEST_TIMEOUT_MS),
            test_ca_roots(),
        )
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
    }

    #[tokio::test]
    async fn a_silent_https_server_times_out() {
        let (listener, port) = listen().await;
        tokio::spawn(async move {
            // Accepts the TCP connection and never answers the handshake.
            let (_tcp, _peer) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let (reports_to, _reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);

        let refusal = send_request_with_roots(
            https_request(
                TEST_HOST,
                port,
                Some(tag_reporting_to(reports_to)),
                SILENT_SERVER_TIMEOUT_MS,
            ),
            test_ca_roots(),
        )
        .await;

        assert_eq!(
            refusal,
            Err(TransportError::TimedOut {
                timeout_ms: SILENT_SERVER_TIMEOUT_MS
            })
        );
    }

    #[tokio::test]
    async fn a_certificate_from_an_untrusted_authority_is_refused() {
        // The public roots don't include the test CA, so the handshake must fail before any request is sent.
        let (listener, port) = listen().await;
        let server = tokio::spawn(async move {
            let (tcp, _peer) = listener.accept().await.unwrap();
            let Ok(mut tls) = test_server_acceptor().accept(tcp).await else {
                return false;
            };
            let _ = read_head(&mut tls).await;
            true
        });
        let (reports_to, _reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);

        let refusal = send_request(https_request(
            TEST_HOST,
            port,
            Some(tag_reporting_to(reports_to)),
            REQUEST_TIMEOUT_MS,
        ))
        .await;

        assert!(!server.await.unwrap(), "the server should never see a request");
        let error = refusal.unwrap_err();
        assert_eq!(error.code(), "RRS-N2008");
        assert!(format!("{error:?}").contains("TLS handshake"), "{error:?}");
    }

    #[tokio::test]
    async fn a_certificate_that_does_not_name_the_host_is_refused() {
        // The test CA is trusted, but the certificate is for localhost only, so connecting to 127.0.0.1 is refused.
        let (listener, port) = listen().await;
        tokio::spawn(async move {
            let (tcp, _peer) = listener.accept().await.unwrap();
            let _ = test_server_acceptor().accept(tcp).await;
        });
        let (reports_to, _reports) = mpsc::channel(REPORT_CHANNEL_CAPACITY);

        let refusal = send_request_with_roots(
            https_request(
                "127.0.0.1",
                port,
                Some(tag_reporting_to(reports_to)),
                REQUEST_TIMEOUT_MS,
            ),
            test_ca_roots(),
        )
        .await;

        assert_eq!(refusal.map_err(|error| error.code()), Err("RRS-N2008"));
    }
}
