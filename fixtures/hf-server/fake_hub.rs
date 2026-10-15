//! A local fake of the Hugging Face Hub endpoints the node's downloader uses (layout rust-node-gateway-proto.md 7.9),
//! served over plain HTTP on 127.0.0.1 so no test reaches the network. It answers the file listing
//! (`GET /api/models/<repo>` and `GET /api/models/<repo>/revision/<revision>`) and file bytes
//! (`GET /<repo>/resolve/<commit>/<file>`, with a Range request resumed as 206). Every request is recorded, so a test
//! can check what was asked for and what was not. A file can answer with a redirect (a CDN, no Location, or a loop), as
//! the Hub does for an LFS file; the CDN a redirect names is a second fake that serves the same files under `/fake-cdn/`.
//!
//! The tests include this file with `#[path]` (see tests/downloads.rs): it is test code, not part of the node. Names
//! are plain ASCII, so there is no percent-decoding here.
#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use rastai_transport::http_client::sha256_digest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The most request-head bytes the fake reads; a real request is far smaller.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// The length of a git blob id as the listing gives it for a plain file.
const GIT_BLOB_ID_HEX_DIGITS: usize = 40;
const MAIN_BRANCH: &str = "main";
/// The path prefix a redirected file request lands on: the CDN fake serves the same files under it.
const CDN_PATH_PREFIX: &str = "/fake-cdn";

/// How a file answers instead of with its bytes: with a 302, as the Hub does for an LFS file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Redirect {
    /// The file answers itself.
    None,
    /// A 302 to the same file on the CDN fake. Only a fake built with `FakeHub::start_with_cdn` redirects this way;
    /// otherwise the file answers itself.
    ToCdn,
    /// A 302 with no Location header, which the client can't follow.
    NoLocation,
    /// A 302 whose Location is the file's own address again, so a chain of redirects never ends.
    LoopsToItself,
}

/// One file a repository serves.
#[derive(Clone)]
pub struct FakeFile {
    pub name: String,
    pub content: Vec<u8>,
    /// The SHA-256 the listing gives an LFS file; None lists a plain git file. A test can list a wrong one.
    pub listed_sha256: Option<String>,
    /// Body bytes sent before the connection closes early, a dropped connection. None sends the whole body.
    pub drop_after: Option<usize>,
    /// Whether every request drops (true), or only the first (false).
    pub drop_every_time: bool,
    /// Whether the file answers a Range request with the whole file (200) instead of the rest (206).
    pub ignores_range: bool,
    /// Whether a 206 names a Content-Range that starts one byte after the one it sent. The body is still the rest of the
    /// file, so a client that trusts the range appends the wrong bytes.
    pub wrong_content_range: bool,
    /// Whether the file redirects instead of answering with its bytes.
    pub redirect: Redirect,
}

impl FakeFile {
    /// An LFS file: its listing carries the SHA-256 of its content.
    pub fn lfs(name: &str, content: Vec<u8>) -> FakeFile {
        let digest = sha256_hex(&content);
        FakeFile {
            name: name.to_string(),
            content,
            listed_sha256: Some(digest),
            drop_after: None,
            drop_every_time: false,
            ignores_range: false,
            wrong_content_range: false,
            redirect: Redirect::None,
        }
    }

    /// A plain git file: its listing carries a blob id and no SHA-256.
    pub fn plain(name: &str, content: Vec<u8>) -> FakeFile {
        FakeFile {
            listed_sha256: None,
            ..FakeFile::lfs(name, content)
        }
    }
}

/// How a gated repository answers an account without access: `status` (401 or 403) unless the request carries
/// `Authorization: Bearer <granted_token>`. The real Hub lists a gated repository to anyone, so `resolve_only` gates
/// the file requests alone and the listing answers 200.
#[derive(Clone)]
pub struct GatedAccess {
    pub status: u16,
    pub granted_token: String,
    pub resolve_only: bool,
}

/// One repository the fake serves at one commit. `main` names the commit as well.
#[derive(Clone)]
pub struct FakeRepo {
    pub name: String,
    pub commit: String,
    pub files: Vec<FakeFile>,
    pub gated: Option<GatedAccess>,
}

/// One request as the fake received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub authorization: Option<String>,
}

struct FakeState {
    repos: Vec<FakeRepo>,
    /// This fake's own port, which a looping redirect names.
    port: u16,
    /// The port of the CDN fake that redirected files go to, when this fake redirects at all.
    cdn_port: Option<u16>,
    requests: Mutex<Vec<RecordedRequest>>,
    /// The files whose one dropped answer has already been sent, keyed by repository and file name.
    dropped_once: Mutex<HashSet<String>>,
}

/// A running fake Hub. It stops when the test ends and the runtime shuts down.
pub struct FakeHub {
    address: SocketAddr,
    state: Arc<FakeState>,
}

impl FakeHub {
    /// Starts the fake on a free loopback port.
    pub async fn start(repos: Vec<FakeRepo>) -> FakeHub {
        FakeHub::start_with(repos, None).await
    }

    /// Starts a fake whose files marked `Redirect::ToCdn` redirect to `cdn`, on its own port. The CDN is an
    /// ordinary fake that serves the same files; a request to it is a different origin, so a test can check that the
    /// token and the Range travel there as the client should send them.
    pub async fn start_with_cdn(repos: Vec<FakeRepo>, cdn: &FakeHub) -> FakeHub {
        FakeHub::start_with(repos, Some(cdn.port())).await
    }

    async fn start_with(repos: Vec<FakeRepo>, cdn_port: Option<u16>) -> FakeHub {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(FakeState {
            repos,
            port: address.port(),
            cdn_port,
            requests: Mutex::new(Vec::new()),
            dropped_once: Mutex::new(HashSet::new()),
        });
        let accepting_state = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(handle_connection(socket, accepting_state.clone()));
            }
        });
        FakeHub { address, state }
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Every request so far, in the order the fake received them.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.requests.lock().unwrap().clone()
    }

    /// The file requests (the resolve path) for one file name, in order.
    pub fn file_requests(&self, file_name: &str) -> Vec<RecordedRequest> {
        let suffix = format!("/{file_name}");
        self.requests()
            .into_iter()
            .filter(|request| request.path.contains("/resolve/") && request.path.ends_with(&suffix))
            .collect()
    }
}

/// The SHA-256 of some bytes as lowercase hex, from the same helper the node hashes with.
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256_digest(bytes)
}

/// A fixed-length filler the tests can build a file from: byte `i` is `(i * 31 + seed) mod 256`, so two files of one
/// size differ and the content is the same on every run.
pub fn patterned_bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| ((index * 31 + seed as usize) % 256) as u8)
        .collect()
}

async fn handle_connection(mut socket: TcpStream, state: Arc<FakeState>) {
    let Some(head) = read_head(&mut socket).await else {
        return;
    };
    let request = parse_head(&head);
    state.requests.lock().unwrap().push(request.clone());
    let answer = answer_for(&request, &state);
    let _ = write_answer(&mut socket, answer).await;
}

/// Reads up to the blank line that ends the request head.
async fn read_head(socket: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_HEAD_BYTES {
            return None;
        }
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(head).ok()
}

fn parse_head(head: &str) -> RecordedRequest {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut range = None;
    let mut authorization = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("range") {
                range = Some(value);
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = Some(value);
            }
        }
    }
    RecordedRequest {
        method,
        path,
        range,
        authorization,
    }
}

/// What the fake sends back: a status, its headers, the whole body and how much of the body goes out before the
/// connection closes.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    send_body_bytes: usize,
}

fn answer_for(request: &RecordedRequest, state: &FakeState) -> Answer {
    if request.method != "GET" {
        return status_only(405);
    }
    let path = request.path.split('?').next().unwrap_or("");
    let path = match path.strip_prefix(CDN_PATH_PREFIX) {
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    };
    if let Some(listing) = path.strip_prefix("/api/models/") {
        return listing_answer(listing, request, state);
    }
    match path.split_once("/resolve/") {
        Some((repo_path, rest)) => file_answer(repo_path.trim_start_matches('/'), rest, request, state),
        None => status_only(404),
    }
}

fn listing_answer(listing: &str, request: &RecordedRequest, state: &FakeState) -> Answer {
    let (repo_name, revision) = match listing.split_once("/revision/") {
        Some((repo, revision)) => (repo, revision),
        None => (listing, ""),
    };
    let Some(repo) = find_repo(state, repo_name) else {
        return status_only(404);
    };
    if !revision_is_known(repo, revision) {
        return status_only(404);
    }
    if let Some(refused) = gate_status(repo, request, GateScope::Listing) {
        return status_only(refused);
    }
    let body = listing_json(repo).to_string().into_bytes();
    json_answer(body)
}

fn revision_is_known(repo: &FakeRepo, revision: &str) -> bool {
    revision.is_empty() || revision == MAIN_BRANCH || revision == repo.commit
}

fn listing_json(repo: &FakeRepo) -> Value {
    let siblings: Vec<Value> = repo.files.iter().map(sibling_json).collect();
    json!({ "sha": repo.commit, "siblings": siblings })
}

fn sibling_json(file: &FakeFile) -> Value {
    let size = file.content.len();
    match &file.listed_sha256 {
        Some(digest) => json!({
            "rfilename": file.name,
            "size": size,
            "lfs": { "sha256": digest, "size": size },
        }),
        None => json!({
            "rfilename": file.name,
            "blobId": git_blob_id(&file.content),
            "size": size,
        }),
    }
}

/// A stand-in git blob id: the first 40 hex digits of the content's SHA-256. The node doesn't check it.
fn git_blob_id(content: &[u8]) -> String {
    sha256_hex(content)[..GIT_BLOB_ID_HEX_DIGITS].to_string()
}

fn file_answer(repo_name: &str, rest: &str, request: &RecordedRequest, state: &FakeState) -> Answer {
    let Some((commit, file_name)) = rest.split_once('/') else {
        return status_only(404);
    };
    let Some(repo) = find_repo(state, repo_name) else {
        return status_only(404);
    };
    if commit != repo.commit {
        return status_only(404);
    }
    if let Some(refused) = gate_status(repo, request, GateScope::File) {
        return status_only(refused);
    }
    let Some(file) = repo.files.iter().find(|file| file.name == file_name) else {
        return status_only(404);
    };
    if let Some(redirect) = redirect_for(file, repo_name, commit, state) {
        return redirect;
    }
    file_body_answer(repo_name, file, request, state)
}

/// The 302 a file answers with when it redirects, or None when it answers itself. A file that redirects to the CDN
/// answers itself when this fake has no CDN.
fn redirect_for(file: &FakeFile, repo_name: &str, commit: &str, state: &FakeState) -> Option<Answer> {
    match file.redirect {
        Redirect::None => None,
        Redirect::ToCdn => state
            .cdn_port
            .map(|cdn_port| redirect_with_location(cdn_location(cdn_port, repo_name, commit, file))),
        Redirect::NoLocation => Some(redirect_answer(None)),
        Redirect::LoopsToItself => Some(redirect_with_location(own_location(
            state.port, repo_name, commit, file,
        ))),
    }
}

/// The CDN fake's address for the file, on the CDN's own port.
fn cdn_location(cdn_port: u16, repo_name: &str, commit: &str, file: &FakeFile) -> String {
    format!(
        "http://127.0.0.1:{cdn_port}{CDN_PATH_PREFIX}/{repo_name}/resolve/{commit}/{}",
        file.name
    )
}

/// The file's own address on this fake, which a looping redirect names again.
fn own_location(port: u16, repo_name: &str, commit: &str, file: &FakeFile) -> String {
    format!("http://127.0.0.1:{port}/{repo_name}/resolve/{commit}/{}", file.name)
}

fn redirect_with_location(location: String) -> Answer {
    redirect_answer(Some(location))
}

/// A 302, with a Location header when one is given.
fn redirect_answer(location: Option<String>) -> Answer {
    let mut headers = Vec::new();
    if let Some(location) = location {
        headers.push(("Location".to_string(), location));
    }
    headers.push(("Content-Length".to_string(), "0".to_string()));
    Answer {
        status: 302,
        headers,
        body: Vec::new(),
        send_body_bytes: 0,
    }
}

fn file_body_answer(repo_name: &str, file: &FakeFile, request: &RecordedRequest, state: &FakeState) -> Answer {
    let total = file.content.len();
    let offset = match (&request.range, file.ignores_range) {
        (Some(range), false) => match parse_range_start(range) {
            Some(start) if start <= total => start,
            _ => return status_only(416),
        },
        _ => 0,
    };
    let status = if offset > 0 { 206 } else { 200 };
    let body = file.content[offset..].to_vec();
    let mut headers = vec![
        ("Content-Length".to_string(), body.len().to_string()),
        ("ETag".to_string(), format!("\"{}\"", etag_for(file))),
        ("Accept-Ranges".to_string(), "bytes".to_string()),
    ];
    if offset > 0 {
        let last = total - 1;
        let start = if file.wrong_content_range { offset + 1 } else { offset };
        headers.push(("Content-Range".to_string(), format!("bytes {start}-{last}/{total}")));
    }
    let send_body_bytes = drop_point(repo_name, file, state, body.len());
    Answer {
        status,
        headers,
        body,
        send_body_bytes,
    }
}

/// How many body bytes go out. A file set to drop sends that many and closes; the drop happens on the first answer
/// only, unless the file drops every time.
fn drop_point(repo_name: &str, file: &FakeFile, state: &FakeState, body_length: usize) -> usize {
    let Some(after) = file.drop_after else {
        return body_length;
    };
    if file.drop_every_time {
        return after.min(body_length);
    }
    let key = format!("{repo_name}/{}", file.name);
    let first_time = state.dropped_once.lock().unwrap().insert(key);
    if first_time {
        after.min(body_length)
    } else {
        body_length
    }
}

fn etag_for(file: &FakeFile) -> String {
    match &file.listed_sha256 {
        Some(digest) => digest.clone(),
        None => git_blob_id(&file.content),
    }
}

/// `bytes=N-` gives N; any other form is refused (the node only asks for the rest of a file).
fn parse_range_start(range: &str) -> Option<usize> {
    let rest = range.strip_prefix("bytes=")?;
    let start = rest.strip_suffix('-')?;
    start.parse().ok()
}

/// Which requests a gate covers: the listing, or the file requests (`resolve_only` gates the files alone).
#[derive(Clone, Copy, PartialEq, Eq)]
enum GateScope {
    Listing,
    File,
}

/// 401 or 403 when the repository is gated and the request doesn't carry the granted token; None otherwise.
fn gate_status(repo: &FakeRepo, request: &RecordedRequest, scope: GateScope) -> Option<u16> {
    let access = repo.gated.as_ref()?;
    if scope == GateScope::Listing && access.resolve_only {
        return None;
    }
    let granted = format!("Bearer {}", access.granted_token);
    if request.authorization.as_deref() == Some(granted.as_str()) {
        None
    } else {
        Some(access.status)
    }
}

fn find_repo<'a>(state: &'a FakeState, name: &str) -> Option<&'a FakeRepo> {
    state.repos.iter().find(|repo| repo.name == name)
}

fn json_answer(body: Vec<u8>) -> Answer {
    let headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Content-Length".to_string(), body.len().to_string()),
    ];
    let send_body_bytes = body.len();
    Answer {
        status: 200,
        headers,
        body,
        send_body_bytes,
    }
}

fn status_only(status: u16) -> Answer {
    Answer {
        status,
        headers: vec![("Content-Length".to_string(), "0".to_string())],
        body: Vec::new(),
        send_body_bytes: 0,
    }
}

async fn write_answer(socket: &mut TcpStream, answer: Answer) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", answer.status, reason_phrase(answer.status));
    for (name, value) in &answer.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&answer.body[..answer.send_body_bytes]).await?;
    socket.shutdown().await
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Status",
    }
}
