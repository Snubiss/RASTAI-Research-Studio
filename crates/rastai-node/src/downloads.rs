//! The in-house Hugging Face Hub client (contracts 2.5; layout rust-node-gateway-proto.md 7.9): the node-native
//! `download` job. It lists a repository at a revision, then fetches each file into the cache's `blobs/` as
//! `<name>.incomplete`, resuming from that file's length with a Range request after an interruption. An LFS file is
//! checked against its SHA-256 and renamed to its blob name, and the snapshot link is made. The per-file lock is
//! huggingface_hub's, so a Rastai job and a notebook downloading the same file take turns instead of writing one
//! `.incomplete` together.
//!
//! The Hub's HTTP comes through rastai-transport's `http_client`, the one HTTP wrapper; an HTTPS request there carries
//! its egress tag, so the UI shows the download. Nothing here reads the token from a store: `HfTokenSource` is the seam
//! M2-18 fills from `NodeHubService.GetSecret`.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rastai_core::time::now_unix_ms;
use rastai_proto::events::v1::DownloadProgress;
use rastai_proto::node::v1::{LiveEvent, live_event};
use rastai_store::disk_space::check_free_space;
use rastai_store::error::StoreError;
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{
    EgressTag, HttpRequest, HttpTarget, Sha256, next_body_chunk, open_response_stream, send_request,
};
use serde::Deserialize;

use crate::error::NodeError;
use crate::hf_cache;
use crate::live::{LiveHub, publish_live_event};

/// How many tries one file gets before its download stops for good; the partial file stays for a later job (layout
/// 7.9, contracts 15.10).
pub const DOWNLOAD_RETRY_LIMIT: u32 = 10;
/// The wait before the second try; each later wait doubles, up to `DOWNLOAD_RETRY_MAX_SECONDS`.
pub const DOWNLOAD_RETRY_FIRST_SECONDS: u64 = 1;
pub const DOWNLOAD_RETRY_MAX_SECONDS: u64 = 60;
/// How long a download waits for another program's lock on a file, and how often it looks (contracts 15.10).
pub const HF_LOCK_WAIT_SECONDS: u64 = 600;
pub const HF_LOCK_POLL_SECONDS: u64 = 1;
/// The Hub's production address: the one host a node's Hugging Face download reaches.
pub const HF_HUB_HOST: &str = "huggingface.co";
pub const HF_HUB_PORT: u16 = 443;
/// The purpose every Hub request reports its egress under; the node's `GetSecret` purpose for the token is the same.
pub const HF_DOWNLOAD_EGRESS_PURPOSE: &str = "hf_download";
/// The environment variable huggingface_hub reads for offline mode.
pub const HF_HUB_OFFLINE_VARIABLE: &str = "HF_HUB_OFFLINE";
/// The words huggingface_hub accepts as "on" in HF_HUB_OFFLINE.
const OFFLINE_WORDS: &[&str] = &["1", "on", "yes", "true"];
/// The listing is small, so a slow answer within this time is a failed connection.
const HUB_LISTING_TIMEOUT_MS: u64 = 30_000;
/// The longest wait for the next piece of a file before its connection counts as failed.
const HUB_CHUNK_TIMEOUT_MS: u64 = 60_000;
/// Read size when a finished file is hashed: large enough to keep the reads few, small enough to keep memory flat.
const HASH_READ_BYTES: usize = 1024 * 1024;
/// The branch a download without a revision means (huggingface_hub's default ref name).
const MAIN_BRANCH: &str = "main";
const BEARER_PREFIX: &str = "Bearer ";
const HTTP_OK: u16 = 200;
const HTTP_PARTIAL_CONTENT: u16 = 206;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;

/// What a download asks for: the repository, the revision, the files and the job it runs for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRequest {
    pub job_id: String,
    /// A Hugging Face repository id, `<org>/<name>` (or `<name>` alone).
    pub model_ref: String,
    /// A branch, tag or commit; empty for the repository's default branch.
    pub revision: String,
    /// The files to take, by name; empty for every file of the repository.
    pub only_files: Vec<String>,
}

/// How a file's failed connection is retried: `limit` tries in all, each wait `first_delay` and doubling up to
/// `max_delay`. Tests give a zero delay; production uses the constants above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub limit: u32,
    pub first_delay: Duration,
    pub max_delay: Duration,
}

/// The production retry policy: DOWNLOAD_RETRY_LIMIT tries, 1 s doubling to 60 s.
pub fn production_retry_policy() -> RetryPolicy {
    RetryPolicy {
        limit: DOWNLOAD_RETRY_LIMIT,
        first_delay: Duration::from_secs(DOWNLOAD_RETRY_FIRST_SECONDS),
        max_delay: Duration::from_secs(DOWNLOAD_RETRY_MAX_SECONDS),
    }
}

/// Everything a download needs that is not the request: where the Hub is, where the cache is, the token, the settings
/// and the retry and lock waits.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Where the Hub answers: `production_hub()` in a running node, a loopback fake in tests.
    pub hub: HttpTarget,
    /// Required when `hub` is HTTPS (RRS-N3008 without it); the reports go to the job's live channel.
    pub egress: Option<EgressTag>,
    /// The hub cache folder (`hf_cache_dir`, or the `hf.cache_dir` setting).
    pub cache_root: PathBuf,
    /// The Hugging Face token for this download, when the source has one.
    pub token: Option<String>,
    /// Offline mode: refuse before any request (`offline_requested` reads HF_HUB_OFFLINE and the setting).
    pub offline: bool,
    /// `storage.free_space_margin_bytes`: the disk must keep this much free after the download.
    pub free_space_margin_bytes: u64,
    /// How long to wait for another program's lock, and the gap between looks.
    pub lock_wait: Duration,
    pub lock_poll: Duration,
    pub retry: RetryPolicy,
}

/// What a finished download reports: the commit it resolved to, its files, which of them came over the network, which
/// waited for another program's lock (RRS-N3010 is the note for those), and the bytes of the whole download.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DownloadOutcome {
    pub model_ref: String,
    pub commit: String,
    pub files: Vec<String>,
    pub fetched_files: Vec<String>,
    pub waited_files: Vec<String>,
    pub bytes_total: u64,
}

/// Hears how far a download has got. The job wiring turns each call into an ephemeral `download.progress`; a waiting
/// file is announced once, so the job can add the RRS-N3010 note.
pub trait DownloadObserver: Send + Sync {
    fn progress(&self, model_ref: &str, bytes_done: u64, bytes_total: u64);
    fn waiting_for_lock(&self, file_name: &str);
}

/// An observer for callers that only want the outcome.
pub struct SilentObserver;

impl DownloadObserver for SilentObserver {
    fn progress(&self, _model_ref: &str, _bytes_done: u64, _bytes_total: u64) {}
    fn waiting_for_lock(&self, _file_name: &str) {}
}

/// Puts each progress call on the node's live queue for the job, which WatchLive streams (contracts 7.3).
pub struct LiveDownloadObserver {
    pub hub: Arc<LiveHub>,
    pub job_id: String,
}

impl DownloadObserver for LiveDownloadObserver {
    fn progress(&self, model_ref: &str, bytes_done: u64, bytes_total: u64) {
        let event = live_download_progress(&self.job_id, model_ref, bytes_done, bytes_total);
        publish_live_event(&self.hub, event);
    }

    fn waiting_for_lock(&self, _file_name: &str) {}
}

/// One `download.progress` as a live event of the job.
pub fn live_download_progress(job_id: &str, model_ref: &str, bytes_done: u64, bytes_total: u64) -> LiveEvent {
    let progress = DownloadProgress {
        model_ref: model_ref.to_string(),
        bytes_done,
        bytes_total,
    };
    LiveEvent {
        job_id: job_id.to_string(),
        session_id: String::new(),
        emitted_unix_ms: now_unix_ms(),
        payload: Some(live_event::Payload::DownloadProgress(progress)),
    }
}

/// Where the Hugging Face token comes from. M2-18 fills this from `NodeHubService.GetSecret` (purpose `hf_download`) for
/// one job, and the value never reaches job.json, a log or an event. Until then `NoHfToken` answers none, so only
/// public repositories download.
pub trait HfTokenSource: Send + Sync {
    fn hf_token(&self) -> Option<String>;
}

/// The token source until M2-18: no token.
pub struct NoHfToken;

impl HfTokenSource for NoHfToken {
    fn hf_token(&self) -> Option<String> {
        None
    }
}

/// Offline mode is on when the setting `hf.offline` is, or when HF_HUB_OFFLINE holds one of huggingface_hub's words.
pub fn offline_requested(environment_value: Option<&str>, setting_on: bool) -> bool {
    if setting_on {
        return true;
    }
    match environment_value {
        Some(value) => OFFLINE_WORDS.iter().any(|word| value.trim().eq_ignore_ascii_case(word)),
        None => false,
    }
}

/// The Hub the node downloads from in production.
pub fn production_hub() -> HttpTarget {
    HttpTarget::Https {
        host: HF_HUB_HOST.to_string(),
        port: HF_HUB_PORT,
    }
}

/// One file of the repository as the Hub lists it. `blob_name` is the name the file has in `blobs/`: the SHA-256 for an
/// LFS file, the git blob id otherwise. `sha256` is empty for a file the Hub doesn't hash in its listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFile {
    pub name: String,
    pub blob_name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepoListing {
    commit: String,
    files: Vec<RepoFile>,
}

#[derive(Deserialize)]
struct ListingJson {
    sha: String,
    #[serde(default)]
    siblings: Vec<SiblingJson>,
}

#[derive(Deserialize)]
struct SiblingJson {
    rfilename: String,
    #[serde(default, rename = "blobId")]
    blob_id: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<LfsJson>,
}

#[derive(Deserialize)]
struct LfsJson {
    sha256: String,
    #[serde(default)]
    size: u64,
}

/// Downloads a model into the hub cache: lists it, checks the disk, then fetches each file under its lock and links
/// the snapshot. Each file already in `blobs/` is only linked. An error keeps what was downloaded, so a later job resumes.
pub async fn download_model(
    request: &DownloadRequest,
    options: &DownloadOptions,
    observer: &dyn DownloadObserver,
) -> Result<DownloadOutcome, NodeError> {
    check_request_names(request)?;
    if options.offline {
        return Err(NodeError::ModelOffline {
            repository: request.model_ref.clone(),
        });
    }
    let listing = fetch_listing(request, options).await?;
    let files = select_files(&listing, &request.only_files, &request.model_ref)?;
    let repo_dir = hf_cache::repo_dir(&options.cache_root, &request.model_ref);
    fs::create_dir_all(&repo_dir).map_err(|error| cache_error(&repo_dir, error))?;
    check_disk(&repo_dir, &files, options)?;
    download_files(request, options, observer, &listing, &files, &repo_dir).await
}

/// Refuses a repository id or revision that would leave the cache folder. The Hub's names are checked when listed.
fn check_request_names(request: &DownloadRequest) -> Result<(), NodeError> {
    let folders = request.model_ref.split('/').count();
    if !hf_cache::is_safe_relative_name(&request.model_ref) || folders > 2 {
        return Err(NodeError::UnsafeHubPath {
            path: request.model_ref.clone(),
        });
    }
    if !request.revision.is_empty() && !hf_cache::is_safe_relative_name(&request.revision) {
        return Err(NodeError::UnsafeHubPath {
            path: request.revision.clone(),
        });
    }
    Ok(())
}

/// The files the download takes, in the listing's order, or the chosen ones in the order they were asked for. A chosen
/// name the repository doesn't have is RRS-N3004.
fn select_files(listing: &RepoListing, only_files: &[String], repository: &str) -> Result<Vec<RepoFile>, NodeError> {
    if only_files.is_empty() {
        return Ok(listing.files.clone());
    }
    let mut chosen = Vec::new();
    for name in only_files {
        match listing.files.iter().find(|file| &file.name == name) {
            Some(file) => chosen.push(file.clone()),
            None => {
                return Err(NodeError::ModelNotFound {
                    repository: repository.to_string(),
                    file_name: name.clone(),
                });
            }
        }
    }
    Ok(chosen)
}

/// The disk check comes first (RRS-S3001): the bytes still to come, plus the margin, must be free.
fn check_disk(repo_dir: &Path, files: &[RepoFile], options: &DownloadOptions) -> Result<(), NodeError> {
    let needed = bytes_still_needed(repo_dir, files).map_err(|error| cache_error(repo_dir, error))?;
    check_free_space(&options.cache_root, needed, options.free_space_margin_bytes)?;
    Ok(())
}

/// The bytes the files need that aren't in `blobs/` yet; a partial file counts only for the part it lacks.
fn bytes_still_needed(repo_dir: &Path, files: &[RepoFile]) -> io::Result<u64> {
    let mut needed: u64 = 0;
    for file in files {
        if hf_cache::blob_path(repo_dir, &file.blob_name).exists() {
            continue;
        }
        let have = file_length(&hf_cache::incomplete_path(repo_dir, &file.blob_name))?;
        needed = needed.saturating_add(file.size.saturating_sub(have));
    }
    Ok(needed)
}

/// Fetches each file in turn and links its snapshot, then writes the revision's ref.
async fn download_files(
    request: &DownloadRequest,
    options: &DownloadOptions,
    observer: &dyn DownloadObserver,
    listing: &RepoListing,
    files: &[RepoFile],
    repo_dir: &Path,
) -> Result<DownloadOutcome, NodeError> {
    let bytes_total = files.iter().fold(0u64, |total, file| total.saturating_add(file.size));
    let mut outcome = DownloadOutcome {
        model_ref: request.model_ref.clone(),
        commit: listing.commit.clone(),
        bytes_total,
        ..DownloadOutcome::default()
    };
    observer.progress(&request.model_ref, 0, bytes_total);
    let mut bytes_before: u64 = 0;
    for file in files {
        let job = FileJob {
            options,
            observer,
            model_ref: &request.model_ref,
            commit: &listing.commit,
            file,
            repo_dir,
            bytes_before,
            bytes_total,
        };
        let status = ensure_blob(&job).await?;
        hf_cache::link_snapshot(repo_dir, &listing.commit, &file.name, &file.blob_name)
            .map_err(|error| cache_error(repo_dir, error))?;
        record_file(&mut outcome, file, status);
        bytes_before = bytes_before.saturating_add(file.size);
        observer.progress(&request.model_ref, bytes_before, bytes_total);
    }
    write_revision_ref(repo_dir, request, &listing.commit)?;
    Ok(outcome)
}

fn record_file(outcome: &mut DownloadOutcome, file: &RepoFile, status: BlobStatus) {
    outcome.files.push(file.name.clone());
    if status.fetched {
        outcome.fetched_files.push(file.name.clone());
    }
    if status.waited {
        outcome.waited_files.push(file.name.clone());
    }
}

/// Records the commit under the revision's name, unless the revision is the commit itself.
fn write_revision_ref(repo_dir: &Path, request: &DownloadRequest, commit: &str) -> Result<(), NodeError> {
    let revision = if request.revision.is_empty() {
        MAIN_BRANCH
    } else {
        request.revision.as_str()
    };
    if revision == commit {
        return Ok(());
    }
    hf_cache::write_ref(repo_dir, revision, commit).map_err(|error| cache_error(repo_dir, error))
}

/// What one file's download needs, in one value so the helpers take two arguments, not nine.
struct FileJob<'a> {
    options: &'a DownloadOptions,
    observer: &'a dyn DownloadObserver,
    model_ref: &'a str,
    commit: &'a str,
    file: &'a RepoFile,
    repo_dir: &'a Path,
    /// The bytes of the files before this one, for the progress count.
    bytes_before: u64,
    bytes_total: u64,
}

/// Whether a file came over the network in this download, and whether it waited for another program's lock first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlobStatus {
    fetched: bool,
    waited: bool,
}

/// Makes sure `blobs/<name>` exists: if it does, nothing is fetched; otherwise the file is fetched under the lock. The
/// blob is checked again once the lock is held, since another program may have finished it while this one waited.
async fn ensure_blob(job: &FileJob<'_>) -> Result<BlobStatus, NodeError> {
    let blob = hf_cache::blob_path(job.repo_dir, &job.file.blob_name);
    if blob.exists() {
        return Ok(BlobStatus {
            fetched: false,
            waited: false,
        });
    }
    let lock_file = hf_cache::lock_path(&job.options.cache_root, job.model_ref, &job.file.blob_name);
    let (lock, waited) = take_file_lock(&lock_file, &job.file.name, job.options, job.observer).await?;
    let fetched = if blob.exists() {
        false
    } else {
        fetch_blob(job).await?;
        true
    };
    drop(lock);
    Ok(BlobStatus { fetched, waited })
}

/// Takes the exclusive lock of one file, polling while another program holds it. Returns the open lock file, which
/// holds the lock until it is dropped, and whether the wait happened (the observer hears of it once).
async fn take_file_lock(
    lock_file: &Path,
    file_name: &str,
    options: &DownloadOptions,
    observer: &dyn DownloadObserver,
) -> Result<(File, bool), NodeError> {
    if let Some(folder) = lock_file.parent() {
        fs::create_dir_all(folder).map_err(|error| cache_error(folder, error))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_file)
        .map_err(|error| cache_error(lock_file, error))?;
    let deadline = Instant::now() + options.lock_wait;
    let mut waited = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok((file, waited)),
            Err(TryLockError::WouldBlock) => {
                if !waited {
                    waited = true;
                    observer.waiting_for_lock(file_name);
                }
                if Instant::now() >= deadline {
                    return Err(NodeError::DownloadLockTimeout {
                        file_name: file_name.to_string(),
                    });
                }
                tokio::time::sleep(options.lock_poll).await;
            }
            Err(TryLockError::Error(error)) => return Err(cache_error(lock_file, error)),
        }
    }
}

/// Fetches one file into `blobs/`: retries the connection, checks an LFS file's SHA-256, and renames the partial file
/// to its blob name. The caller holds the file's lock throughout.
async fn fetch_blob(job: &FileJob<'_>) -> Result<(), NodeError> {
    let incomplete = hf_cache::incomplete_path(job.repo_dir, &job.file.blob_name);
    if let Some(folder) = incomplete.parent() {
        fs::create_dir_all(folder).map_err(|error| cache_error(folder, error))?;
    }
    fetch_with_retries(job, &incomplete).await?;
    verify_sha256(&incomplete, job.file).await?;
    let blob = hf_cache::blob_path(job.repo_dir, &job.file.blob_name);
    fs::rename(&incomplete, &blob).map_err(|error| cache_error(&blob, error))
}

/// Tries the fetch again after a failed connection, waiting longer each time, until the retry limit. Any other failure
/// (gated, not found, a refused egress tag) stops at once.
async fn fetch_with_retries(job: &FileJob<'_>, incomplete: &Path) -> Result<(), NodeError> {
    let retry = &job.options.retry;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match fetch_attempt(job, incomplete).await {
            Ok(()) => return Ok(()),
            Err(error) if is_interruption(&error) => {
                if attempt >= retry.limit {
                    return Err(NodeError::DownloadInterrupted {
                        file_name: job.file.name.clone(),
                        reason: interruption_reason(&error),
                    });
                }
                tokio::time::sleep(backoff_delay(retry, attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// One request for the rest of a file: from the partial file's length when there is one, else the whole file. Writes
/// what arrives to the partial file, so a failure keeps every byte that came.
async fn fetch_attempt(job: &FileJob<'_>, incomplete: &Path) -> Result<(), NodeError> {
    let have = file_length(incomplete).map_err(|error| cache_error(incomplete, error))?;
    if job.file.size > 0 && have >= job.file.size {
        return Ok(());
    }
    let offset = if have > 0 { Some(have) } else { None };
    let path = resolve_path(job.model_ref, job.commit, &job.file.name);
    let mut stream = open_response_stream(hub_request(job.options, path, offset, HUB_CHUNK_TIMEOUT_MS)).await?;
    check_file_status(job, incomplete, stream.status)?;
    // A server that ignored the Range header sends the whole file again: the partial file starts over.
    let restart = stream.status == HTTP_OK && have > 0;
    let mut output = open_output(incomplete, restart).map_err(|error| cache_error(incomplete, error))?;
    let mut written = if restart { 0 } else { have };
    while let Some(chunk) = next_body_chunk(&mut stream).await? {
        output
            .write_all(&chunk)
            .map_err(|error| cache_error(incomplete, error))?;
        written = written.saturating_add(chunk.len() as u64);
        job.observer
            .progress(job.model_ref, job.bytes_before.saturating_add(written), job.bytes_total);
    }
    drop(output);
    check_body_length(job, incomplete)
}

/// A body that ends before the file's listed size is a dropped connection, so it is retried like one.
fn check_body_length(job: &FileJob<'_>, incomplete: &Path) -> Result<(), NodeError> {
    let length = file_length(incomplete).map_err(|error| cache_error(incomplete, error))?;
    if job.file.size > 0 && length < job.file.size {
        return Err(connection_refused(format!(
            "the body ended after {length} of {} bytes",
            job.file.size
        )));
    }
    Ok(())
}

/// The answer's status decides the fetch: 200 or 206 are the file, 401 or 403 is a gated repository, 404 a missing one.
/// A 416 means the partial file is longer than the Hub's file; it is removed so the next try starts over.
fn check_file_status(job: &FileJob<'_>, incomplete: &Path, status: u16) -> Result<(), NodeError> {
    match status {
        HTTP_OK | HTTP_PARTIAL_CONTENT => Ok(()),
        HTTP_UNAUTHORIZED | HTTP_FORBIDDEN => Err(NodeError::ModelGated {
            repository: job.model_ref.to_string(),
        }),
        HTTP_NOT_FOUND => Err(NodeError::ModelNotFound {
            repository: job.model_ref.to_string(),
            file_name: job.file.name.clone(),
        }),
        HTTP_RANGE_NOT_SATISFIABLE => {
            let _ = fs::remove_file(incomplete);
            Err(connection_refused(
                "the Hub refused the resumed range; the file starts again".to_string(),
            ))
        }
        other => Err(hub_status_refused(other, "the file")),
    }
}

/// Checks an LFS file's SHA-256 against the listing and removes the partial file when it doesn't match. A file the
/// listing doesn't hash is not checked: rustls's SHA-256 is the only hash the node has, and git's SHA-1 blob ids are
/// not checked here.
async fn verify_sha256(incomplete: &Path, file: &RepoFile) -> Result<(), NodeError> {
    if file.sha256.is_empty() {
        return Ok(());
    }
    let path = incomplete.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || hash_file(&path))
        .await
        .map_err(|error| cache_error(incomplete, io::Error::other(error.to_string())))?
        .map_err(|error| cache_error(incomplete, error))?;
    if digest.eq_ignore_ascii_case(&file.sha256) {
        return Ok(());
    }
    let _ = fs::remove_file(incomplete);
    Err(NodeError::DownloadHashMismatch {
        path: incomplete.to_path_buf(),
    })
}

/// The SHA-256 of a file's bytes as lowercase hex, read in pieces.
fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_READ_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(hasher.finish_hex());
        }
        hasher.update(&buffer[..read]);
    }
}

/// The request for a file or the listing: the token, the Range for a resumed file and the egress tag.
fn hub_request(options: &DownloadOptions, path: String, range_from: Option<u64>, timeout_ms: u64) -> HttpRequest {
    let mut headers = Vec::new();
    if let Some(token) = &options.token {
        headers.push(("Authorization".to_string(), format!("{BEARER_PREFIX}{token}")));
    }
    if let Some(offset) = range_from {
        headers.push(("Range".to_string(), format!("bytes={offset}-")));
    }
    HttpRequest {
        method: "GET".to_string(),
        target: options.hub.clone(),
        path,
        headers,
        body: Vec::new(),
        timeout_ms,
        egress: options.egress.clone(),
    }
}

async fn fetch_listing(request: &DownloadRequest, options: &DownloadOptions) -> Result<RepoListing, NodeError> {
    let path = listing_path(&request.model_ref, &request.revision);
    let response = send_request(hub_request(options, path, None, HUB_LISTING_TIMEOUT_MS)).await?;
    match response.status {
        HTTP_OK => parse_listing(&response.body),
        HTTP_UNAUTHORIZED | HTTP_FORBIDDEN => Err(NodeError::ModelGated {
            repository: request.model_ref.clone(),
        }),
        HTTP_NOT_FOUND => Err(NodeError::ModelNotFound {
            repository: request.model_ref.clone(),
            file_name: String::new(),
        }),
        other => Err(hub_status_refused(other, "the file list")),
    }
}

/// Reads the Hub's listing. Every name and blob id must be plain, because each becomes part of a path.
fn parse_listing(body: &[u8]) -> Result<RepoListing, NodeError> {
    let listing: ListingJson = serde_json::from_slice(body)
        .map_err(|error| connection_refused(format!("the file list didn't parse: {error}")))?;
    if !is_plain_id(&listing.sha) {
        return Err(NodeError::UnsafeHubPath { path: listing.sha });
    }
    let mut files = Vec::new();
    for sibling in listing.siblings {
        files.push(repo_file_from(sibling)?);
    }
    Ok(RepoListing {
        commit: listing.sha,
        files,
    })
}

fn repo_file_from(sibling: SiblingJson) -> Result<RepoFile, NodeError> {
    if !hf_cache::is_safe_relative_name(&sibling.rfilename) {
        return Err(NodeError::UnsafeHubPath {
            path: sibling.rfilename,
        });
    }
    let (blob_name, sha256, size) = match sibling.lfs {
        Some(lfs) => (lfs.sha256.clone(), lfs.sha256, lfs.size),
        None => (sibling.blob_id, String::new(), sibling.size),
    };
    if blob_name.is_empty() {
        return Err(connection_refused(format!(
            "the file list gives {} no blob id",
            sibling.rfilename
        )));
    }
    if !is_plain_id(&blob_name) {
        return Err(NodeError::UnsafeHubPath { path: blob_name });
    }
    Ok(RepoFile {
        name: sibling.rfilename,
        blob_name,
        sha256,
        size,
    })
}

/// A commit or blob id is hex text: letters and digits only, so it is one path part.
fn is_plain_id(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|character| character.is_ascii_alphanumeric())
}

/// `/api/models/<repo>?blobs=true` for the default branch, else the same at `/revision/<revision>`.
fn listing_path(repo_id: &str, revision: &str) -> String {
    if revision.is_empty() {
        return format!("/api/models/{}?blobs=true", percent_encode(repo_id, true));
    }
    format!(
        "/api/models/{}/revision/{}?blobs=true",
        percent_encode(repo_id, true),
        percent_encode(revision, false)
    )
}

/// `/<repo>/resolve/<commit>/<file>`: the Hub's address of one file at one commit.
fn resolve_path(repo_id: &str, commit: &str, file_name: &str) -> String {
    format!(
        "/{}/resolve/{commit}/{}",
        percent_encode(repo_id, true),
        percent_encode(file_name, true)
    )
}

/// Percent-encodes everything but letters, digits and `-._~`, and `/` when `keep_slash` is set, so a revision such as
/// `refs/pr/1` becomes one path part.
fn percent_encode(text: &str, keep_slash: bool) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        let plain =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') || (keep_slash && byte == b'/');
        if plain {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Opens a partial file for writing: appended to when the request resumed it, emptied when the server sent the whole
/// file again.
fn open_output(path: &Path, restart: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if restart {
        options.truncate(true);
    } else {
        options.append(true);
    }
    options.open(path)
}

fn file_length(path: &Path) -> io::Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

/// Only a connection that dropped or stalled is tried again; the Hub's answers and a refused tag are not.
fn is_interruption(error: &NodeError) -> bool {
    matches!(
        error,
        NodeError::Transport(TransportError::ConnectionFailed { .. } | TransportError::TimedOut { .. })
    )
}

fn interruption_reason(error: &NodeError) -> String {
    match error {
        NodeError::Transport(TransportError::ConnectionFailed { reason }) => reason.clone(),
        NodeError::Transport(TransportError::TimedOut { timeout_ms }) => format!("no answer within {timeout_ms} ms"),
        other => other.code().to_string(),
    }
}

/// The wait before try `attempt + 1`: the first delay, doubled for each try before, and never more than the maximum.
fn backoff_delay(retry: &RetryPolicy, attempt: u32) -> Duration {
    let doubling = 1u32.checked_shl(attempt - 1).unwrap_or(u32::MAX);
    retry.first_delay.saturating_mul(doubling).min(retry.max_delay)
}

fn connection_refused(reason: String) -> NodeError {
    NodeError::Transport(TransportError::ConnectionFailed { reason })
}

fn hub_status_refused(status: u16, what: &str) -> NodeError {
    connection_refused(format!("the Hub answered {status} to {what}"))
}

fn cache_error(path: &Path, error: io::Error) -> NodeError {
    NodeError::Store(StoreError::DataDirectoryNotWritable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn each_wait_doubles_up_to_the_maximum() {
        let retry = production_retry_policy();
        assert_eq!(backoff_delay(&retry, 1), Duration::from_secs(1));
        assert_eq!(backoff_delay(&retry, 2), Duration::from_secs(2));
        assert_eq!(backoff_delay(&retry, 6), Duration::from_secs(32));
        assert_eq!(backoff_delay(&retry, 7), Duration::from_secs(60));
        assert_eq!(
            backoff_delay(&retry, 40),
            Duration::from_secs(60),
            "a long run of tries never waits longer than the maximum"
        );
    }

    #[test]
    fn percent_encoding_keeps_a_revision_one_path_part() {
        assert_eq!(percent_encode("refs/pr/1", false), "refs%2Fpr%2F1");
        assert_eq!(
            percent_encode("layers/0/sae.safetensors", true),
            "layers/0/sae.safetensors"
        );
        assert_eq!(percent_encode("a b", true), "a%20b");
    }
}
