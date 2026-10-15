//! The in-house Hugging Face Hub client (contracts 2.5; layout rust-node-gateway-proto.md 7.9): the downloader core of
//! M2-17, part 1. `download_model` lists a repository at a revision, then fetches each file into the cache's `blobs/` as
//! `<name>.incomplete`, resuming from that file's length with a Range request after an interruption. An LFS file is
//! checked against its SHA-256 and renamed to its blob name, and the snapshot link is made. The per-file lock is
//! huggingface_hub's, so a Rastai job and a notebook downloading the same file take turns instead of writing one
//! `.incomplete` together.
//!
//! The Hub's HTTP comes through rastai-transport's `http_client`, the one HTTP wrapper. A file's answer may redirect to a
//! CDN host (the Hub does for LFS files); the CDN request has its own egress tag and never carries the token. The token
//! comes in through `HfTokenSource`; this module never stores it.
//!
//! Not in this module yet (M2-17 part 2, after M2-18): the node-native `download` job (`JobKind::Download`), the
//! production `HF_HUB_OFFLINE` reader that job uses, the `download.progress` the job publishes on WatchLive (the observer
//! below is built and not yet driven by a job), the RRS-N3010 note on a job, `download.finished`, and `GetSecret` filling
//! `HfTokenSource` (until then `NoHfToken` answers none).

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rastai_core::paths::hf_lock_path;
use rastai_core::time::now_unix_ms;
use rastai_proto::events::v1::DownloadProgress;
use rastai_proto::node::v1::{LiveEvent, live_event};
use rastai_store::disk_space::check_free_space;
use rastai_store::error::StoreError;
use rastai_transport::error::TransportError;
use rastai_transport::http_client::{
    EgressTag, HttpRequest, HttpStream, HttpTarget, Sha256, next_body_chunk, open_response_stream, send_request,
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
/// The purpose every request to the Hub reports its egress under; the node's `GetSecret` purpose for the token is the
/// same.
pub const HF_DOWNLOAD_EGRESS_PURPOSE: &str = "hf_download";
/// The purpose a redirected file request reports under: the CDN host a file's answer names, which never gets the token.
pub const HF_CDN_EGRESS_PURPOSE: &str = "hf_cdn_download";
/// The environment variable huggingface_hub reads for offline mode.
pub const HF_HUB_OFFLINE_VARIABLE: &str = "HF_HUB_OFFLINE";
/// The words huggingface_hub accepts as "on" in HF_HUB_OFFLINE.
const OFFLINE_WORDS: &[&str] = &["1", "on", "yes", "true"];
/// The most redirects one request follows. The Hub answers a file with one redirect to its CDN, so this is generous.
const HF_REDIRECT_LIMIT: u32 = 5;
/// Progress is published at the start, at the end, and once per this many bytes between them. One event per HTTP
/// chunk would be hundreds of thousands for a multi-gigabyte file.
const HF_PROGRESS_STEP_BYTES: u64 = 1024 * 1024;
/// The listing is small, so a slow answer within this time is a failed connection.
const HUB_LISTING_TIMEOUT_MS: u64 = 30_000;
/// The longest wait for the next piece of a file before its connection counts as failed.
const HUB_CHUNK_TIMEOUT_MS: u64 = 60_000;
/// Read size when a finished file is hashed: large enough to keep the reads few, small enough to keep memory flat.
const HASH_READ_BYTES: usize = 1024 * 1024;
/// The branch a download without a revision means (huggingface_hub's default ref name). The listing asks for it by
/// name too, so the listing and the ref name the same revision.
const MAIN_BRANCH: &str = "main";
const BEARER_PREFIX: &str = "Bearer ";
const HTTPS_PREFIX: &str = "https://";
/// A loopback redirect target, which only the tests' fake Hub and its CDN use.
const LOOPBACK_HTTP_PREFIX: &str = "http://127.0.0.1:";
const HTTPS_DEFAULT_PORT: u16 = 443;
const HTTP_OK: u16 = 200;
const HTTP_PARTIAL_CONTENT: u16 = 206;
const HTTP_MOVED_PERMANENTLY: u16 = 301;
const HTTP_FOUND: u16 = 302;
const HTTP_SEE_OTHER: u16 = 303;
const HTTP_TEMPORARY_REDIRECT: u16 = 307;
const HTTP_PERMANENT_REDIRECT: u16 = 308;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;
/// How many readers `check_hub_answer` picks from: the file list, the Content-Range, the redirect and the plain id.
const HUB_ANSWER_READERS: u8 = 4;

/// What a download asks for: the repository, the revision and the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRequest {
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

/// Everything a download needs that is not the request: where the Hub is, where the cache is, the settings and the retry
/// and lock waits. The token is not here: it comes from the `HfTokenSource` each download is given.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Where the Hub answers: `production_hub()` in a running node, a loopback fake in tests.
    pub hub: HttpTarget,
    /// Required when `hub` is HTTPS (RRS-N3008 without it); the reports go to the job's live channel.
    pub egress: Option<EgressTag>,
    /// The hub cache folder (`hf_cache_dir`, or the `hf.cache_dir` setting).
    pub cache_root: PathBuf,
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

/// Puts progress on the node's live queue for the job, which WatchLive streams (contracts 7.3). It publishes the first
/// report, the last one, and one per `HF_PROGRESS_STEP_BYTES` between them.
pub struct LiveDownloadObserver {
    hub: Arc<LiveHub>,
    job_id: String,
    /// The bytes of the last report published; None before the first one.
    last_published: Mutex<Option<u64>>,
}

impl LiveDownloadObserver {
    pub fn new(hub: Arc<LiveHub>, job_id: &str) -> LiveDownloadObserver {
        LiveDownloadObserver {
            hub,
            job_id: job_id.to_string(),
            last_published: Mutex::new(None),
        }
    }
}

impl DownloadObserver for LiveDownloadObserver {
    fn progress(&self, model_ref: &str, bytes_done: u64, bytes_total: u64) {
        let mut last = self
            .last_published
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !progress_due(*last, bytes_done, bytes_total) {
            return;
        }
        *last = Some(bytes_done);
        drop(last);
        let event = live_download_progress(&self.job_id, model_ref, bytes_done, bytes_total);
        publish_live_event(&self.hub, event);
    }

    fn waiting_for_lock(&self, _file_name: &str) {}
}

/// Whether a progress report goes out: the first one, the last one, and one per step of bytes in between. A value
/// already published is never sent again: the last chunk and the end of its file reach the same total.
fn progress_due(last_published: Option<u64>, bytes_done: u64, bytes_total: u64) -> bool {
    match last_published {
        None => true,
        Some(previous) if previous == bytes_done => false,
        Some(previous) => bytes_done >= bytes_total || bytes_done.saturating_sub(previous) >= HF_PROGRESS_STEP_BYTES,
    }
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

/// The readers of the Hub's and the CDN's answers, as one entry point for the fuzz target (contracts 12.5, ADR-0037).
/// The first byte picks a reader and the rest is its input: a file list, a Content-Range value, a redirect's Location,
/// or a name that becomes part of a path. Whatever arrives must never panic a reader; what a reader accepts must be
/// safe to use, and a redirect from the production Hub never reaches loopback. Returns the first broken promise.
pub fn check_hub_answer(input: &[u8]) -> Result<(), String> {
    let Some((reader, body)) = input.split_first() else {
        return Ok(());
    };
    let text = String::from_utf8_lossy(body);
    match reader % HUB_ANSWER_READERS {
        0 => check_listing_answer(body),
        // The readers below return a value the caller checks; here only that they return at all matters.
        1 => {
            let _ = content_range_start(&text);
            Ok(())
        }
        2 => check_location_answer(&text),
        _ => {
            let _ = is_plain_id(&text);
            Ok(())
        }
    }
}

/// A file list the reader accepts names only safe files and plain ids, so each can become a path part.
fn check_listing_answer(body: &[u8]) -> Result<(), String> {
    let Ok(listing) = parse_listing(body) else {
        return Ok(());
    };
    if !is_plain_id(&listing.commit) {
        return Err(format!("accepted the commit {:?}", listing.commit));
    }
    for file in &listing.files {
        if !hf_cache::is_safe_relative_name(&file.name) || !is_plain_id(&file.blob_name) {
            return Err(format!("accepted the file {:?}", file.name));
        }
    }
    Ok(())
}

/// A redirect from the production Hub that Rastai follows never names loopback, and its path starts at the root.
fn check_location_answer(location: &str) -> Result<(), String> {
    let access = HubAccess {
        hub: production_hub(),
        egress: None,
        token: None,
    };
    let current = Route {
        target: production_hub(),
        path: "/".to_string(),
        egress: None,
    };
    let Ok(route) = route_for_location(&access, &current, location) else {
        return Ok(());
    };
    if matches!(route.target, HttpTarget::LoopbackHttp { .. }) {
        return Err(format!("followed {location:?} to loopback"));
    }
    if !route.path.starts_with('/') {
        return Err(format!("followed {location:?} to the path {:?}", route.path));
    }
    Ok(())
}

/// Downloads a model into the hub cache: lists it, checks the disk, then fetches each file under its lock and links
/// the snapshot. Each file already in `blobs/` is only linked. An error keeps what was downloaded, so a later job resumes.
pub async fn download_model(
    request: &DownloadRequest,
    options: &DownloadOptions,
    observer: &dyn DownloadObserver,
    tokens: &dyn HfTokenSource,
) -> Result<DownloadOutcome, NodeError> {
    check_request_names(request)?;
    if options.offline {
        return Err(NodeError::ModelOffline {
            repository: request.model_ref.clone(),
        });
    }
    let access = HubAccess {
        hub: options.hub.clone(),
        egress: options.egress.clone(),
        token: tokens.hf_token(),
    };
    let listing = fetch_listing(request, &access).await?;
    let files = select_files(&listing, &request.only_files, &request.model_ref)?;
    let repo_dir = hf_cache::repo_dir(&options.cache_root, &request.model_ref);
    fs::create_dir_all(&repo_dir).map_err(|error| cache_error(&repo_dir, error))?;
    check_disk(&repo_dir, &files, options)?;
    download_files(request, options, &access, observer, &listing, &files).await
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
    access: &HubAccess,
    observer: &dyn DownloadObserver,
    listing: &RepoListing,
    files: &[RepoFile],
) -> Result<DownloadOutcome, NodeError> {
    let repo_dir = hf_cache::repo_dir(&options.cache_root, &request.model_ref);
    let repo_dir = repo_dir.as_path();
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
            access,
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
    let revision = revision_or_main(&request.revision);
    if revision == commit {
        return Ok(());
    }
    hf_cache::write_ref(repo_dir, revision, commit).map_err(|error| cache_error(repo_dir, error))
}

/// The revision a request names, or the default branch when it names none.
fn revision_or_main(revision: &str) -> &str {
    if revision.is_empty() { MAIN_BRANCH } else { revision }
}

/// Who the Hub is for this download: its address, the egress tag its requests report under, and the token. The token
/// is sent to the Hub only, never to another host a redirect names. The struct has no Debug, so a log can't print it.
struct HubAccess {
    hub: HttpTarget,
    egress: Option<EgressTag>,
    token: Option<String>,
}

/// One request's destination: where it goes, its path, and the egress tag that reports it.
#[derive(Clone)]
struct Route {
    target: HttpTarget,
    path: String,
    egress: Option<EgressTag>,
}

/// What one file's download needs, in one value so the helpers take two arguments, not nine.
struct FileJob<'a> {
    options: &'a DownloadOptions,
    access: &'a HubAccess,
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
    let repository_folder = hf_cache::repo_folder_name(job.model_ref);
    let lock_file = hf_lock_path(&job.options.cache_root, &repository_folder, &job.file.blob_name);
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
/// to its blob name. A hash mismatch removes the partial file, so the file is fetched once more from zero; a second
/// mismatch is RRS-N3003 (layout flow 1.5). The caller holds the file's lock throughout.
async fn fetch_blob(job: &FileJob<'_>) -> Result<(), NodeError> {
    let incomplete = hf_cache::incomplete_path(job.repo_dir, &job.file.blob_name);
    if let Some(folder) = incomplete.parent() {
        fs::create_dir_all(folder).map_err(|error| cache_error(folder, error))?;
    }
    let checked = match fetch_and_check(job, &incomplete).await {
        Err(NodeError::DownloadHashMismatch { .. }) => fetch_and_check(job, &incomplete).await,
        result => result,
    };
    checked?;
    let blob = hf_cache::blob_path(job.repo_dir, &job.file.blob_name);
    fs::rename(&incomplete, &blob).map_err(|error| cache_error(&blob, error))
}

/// One fetch of the file's rest, with its retries, then the SHA-256 check.
async fn fetch_and_check(job: &FileJob<'_>, incomplete: &Path) -> Result<(), NodeError> {
    fetch_with_retries(job, incomplete).await?;
    verify_sha256(incomplete, job.file).await
}

/// Tries the fetch again after a failed connection, waiting longer each time, until the retry limit. Any other failure
/// (gated, not found, a refused egress tag, a full disk) stops at once.
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
    let route = hub_route(job.access, path);
    let mut stream = open_hub_stream(job.access, route, offset, HUB_CHUNK_TIMEOUT_MS, &job.file.name).await?;
    check_file_status(job, incomplete, stream.status)?;
    check_range_start(stream.status, &stream.headers, offset)?;
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

/// A 206 answer must start where the request asked. Appending from another byte would write the wrong bytes into the
/// partial file, and only the SHA-256 check would notice, which a file the listing doesn't hash never gets.
fn check_range_start(status: u16, headers: &[(String, String)], offset: Option<u64>) -> Result<(), NodeError> {
    if status != HTTP_PARTIAL_CONTENT {
        return Ok(());
    }
    let requested = offset.unwrap_or(0);
    match header_value(headers, "content-range").and_then(content_range_start) {
        Some(start) if start == requested => Ok(()),
        _ => Err(connection_refused(format!(
            "the Hub's range doesn't start at byte {requested}"
        ))),
    }
}

/// The first byte of a `Content-Range: bytes <start>-<end>/<total>` value, or None when it has another form.
fn content_range_start(value: &str) -> Option<u64> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (start, _) = rest.split_once('-')?;
    start.trim().parse().ok()
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

/// The route to a path on the Hub.
fn hub_route(access: &HubAccess, path: String) -> Route {
    Route {
        target: access.hub.clone(),
        path,
        egress: access.egress.clone(),
    }
}

/// The request for a route: the token only when the route is the Hub itself, the Range for a resumed file, and the
/// route's egress tag.
fn request_for_route(access: &HubAccess, route: &Route, range_from: Option<u64>, timeout_ms: u64) -> HttpRequest {
    let mut headers = Vec::new();
    let token = if route.target == access.hub {
        access.token.as_ref()
    } else {
        None
    };
    if let Some(token) = token {
        headers.push(("Authorization".to_string(), format!("{BEARER_PREFIX}{token}")));
    }
    if let Some(offset) = range_from {
        headers.push(("Range".to_string(), format!("bytes={offset}-")));
    }
    HttpRequest {
        method: "GET".to_string(),
        target: route.target.clone(),
        path: route.path.clone(),
        headers,
        body: Vec::new(),
        timeout_ms,
        egress: route.egress.clone(),
    }
}

/// Opens a file's answer and follows its redirects (the Hub redirects an LFS file to its CDN), up to HF_REDIRECT_LIMIT.
/// Each hop keeps the Range. The stream returned is the first answer that isn't a redirect.
async fn open_hub_stream(
    access: &HubAccess,
    first: Route,
    range_from: Option<u64>,
    timeout_ms: u64,
    file_name: &str,
) -> Result<HttpStream, NodeError> {
    let mut route = first;
    for _ in 0..=HF_REDIRECT_LIMIT {
        let stream = open_response_stream(request_for_route(access, &route, range_from, timeout_ms)).await?;
        if !is_redirect(stream.status) {
            return Ok(stream);
        }
        let Some(location) = header_value(&stream.headers, "location") else {
            return Err(redirect_refused(
                file_name,
                format!("the answer redirected with status {} and no location", stream.status),
            ));
        };
        route = route_for_location(access, &route, location)?;
    }
    Err(redirect_refused(
        file_name,
        format!("the answer redirected more than {HF_REDIRECT_LIMIT} times"),
    ))
}

/// A redirect that can't be followed. It is not an interruption, so the retry loop stops at once (RRS-N3013).
fn redirect_refused(file_name: &str, reason: String) -> NodeError {
    NodeError::RedirectRefused {
        file_name: file_name.to_string(),
        reason,
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(
        status,
        HTTP_MOVED_PERMANENTLY | HTTP_FOUND | HTTP_SEE_OTHER | HTTP_TEMPORARY_REDIRECT | HTTP_PERMANENT_REDIRECT
    )
}

/// The route a redirect names. A path alone stays on the same host; an absolute https URL goes to its host, with the CDN
/// egress tag when that host isn't the Hub. An address Rastai doesn't follow is refused as RRS-N3001, which the retry
/// loop doesn't try again: the Hub would name the same address next time.
fn route_for_location(access: &HubAccess, current: &Route, location: &str) -> Result<Route, NodeError> {
    if location.starts_with('/') {
        return Ok(Route {
            target: current.target.clone(),
            path: location.to_string(),
            egress: current.egress.clone(),
        });
    }
    let Some((target, path)) = absolute_location(location) else {
        return Err(NodeError::UnsafeHubPath {
            path: location.to_string(),
        });
    };
    // A plain-HTTP loopback address is the tests' fake Hub. A production Hub that names one could make the node fetch a
    // local service and take its bytes as the file, so loopback is followed only when the Hub itself is loopback.
    if matches!(target, HttpTarget::LoopbackHttp { .. }) && !matches!(access.hub, HttpTarget::LoopbackHttp { .. }) {
        return Err(NodeError::UnsafeHubPath {
            path: location.to_string(),
        });
    }
    let egress = if target == access.hub {
        access.egress.clone()
    } else {
        cdn_egress(access)
    };
    Ok(Route { target, path, egress })
}

/// The egress tag for a host other than the Hub: the same job and subject, under the CDN's own purpose, so the egress
/// badge shows the CDN transfer as its own line.
fn cdn_egress(access: &HubAccess) -> Option<EgressTag> {
    access.egress.as_ref().map(|tag| EgressTag {
        purpose: HF_CDN_EGRESS_PURPOSE.to_string(),
        subject_id: tag.subject_id.clone(),
        reports: tag.reports.clone(),
    })
}

/// The target and path of an absolute URL: `https://host[:port]/path`, or `http://127.0.0.1:port/path` for the tests'
/// loopback fake. Any other form is None, which the caller refuses.
fn absolute_location(location: &str) -> Option<(HttpTarget, String)> {
    if let Some(rest) = location.strip_prefix(HTTPS_PREFIX) {
        let (authority, path) = split_authority(rest);
        if authority.contains('@') {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().ok()?),
            None => (authority, HTTPS_DEFAULT_PORT),
        };
        if host.is_empty() {
            return None;
        }
        let target = HttpTarget::Https {
            host: host.to_string(),
            port,
        };
        return Some((target, path));
    }
    let rest = location.strip_prefix(LOOPBACK_HTTP_PREFIX)?;
    let (port_text, path) = split_authority(rest);
    let port = port_text.parse::<u16>().ok()?;
    Some((HttpTarget::LoopbackHttp { port }, path))
}

/// Splits `authority/path` at the first slash; a URL with no path is the root.
fn split_authority(rest: &str) -> (&str, String) {
    match rest.find('/') {
        Some(index) => (&rest[..index], rest[index..].to_string()),
        None => (rest, "/".to_string()),
    }
}

/// The value of the first header with this name, whatever the case of the name.
fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
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

/// The listing: the commit and the file list at the revision, with the default branch named as `main`.
async fn fetch_listing(request: &DownloadRequest, access: &HubAccess) -> Result<RepoListing, NodeError> {
    let path = listing_path(&request.model_ref, &request.revision);
    let route = hub_route(access, path);
    let response = send_request(request_for_route(access, &route, None, HUB_LISTING_TIMEOUT_MS)).await?;
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

/// `/api/models/<repo>/revision/<revision>?blobs=true`, where an empty revision is the default branch, `main`.
fn listing_path(repo_id: &str, revision: &str) -> String {
    format!(
        "/api/models/{}/revision/{}?blobs=true",
        percent_encode(repo_id, true),
        percent_encode(revision_or_main(revision), false)
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

/// A full disk (the operating system's StorageFull) gets its own code, RRS-S3002; the partial file stays, so a later job
/// resumes it once space is freed. Any other failure keeps the cache folder's code.
fn cache_error(path: &Path, error: io::Error) -> NodeError {
    if error.kind() == io::ErrorKind::StorageFull {
        return NodeError::Store(StoreError::DiskFull {
            reason: format!("{}: {error}", path.display()),
        });
    }
    NodeError::Store(StoreError::DataDirectoryNotWritable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn access_to(hub: HttpTarget) -> HubAccess {
        HubAccess {
            hub,
            egress: None,
            token: None,
        }
    }

    fn route_at(target: HttpTarget) -> Route {
        Route {
            target,
            path: "/first".to_string(),
            egress: None,
        }
    }

    /// A production Hub that redirects to a loopback address would make the node fetch a local service. The redirect is
    /// refused, and the refusal is not retried.
    #[test]
    fn a_production_hub_never_follows_a_redirect_to_loopback() {
        let access = access_to(production_hub());
        let current = route_at(production_hub());
        let error = route_for_location(&access, &current, "http://127.0.0.1:9/secret")
            .err()
            .unwrap();
        assert!(matches!(error, NodeError::UnsafeHubPath { .. }), "{error:?}");
        assert!(!is_interruption(&error));
    }

    /// The test fake is loopback itself, so its redirect to its loopback CDN is followed.
    #[test]
    fn a_loopback_hub_follows_a_redirect_to_loopback() {
        let hub = HttpTarget::LoopbackHttp { port: 8000 };
        let route = route_for_location(&access_to(hub.clone()), &route_at(hub), "http://127.0.0.1:8001/x").unwrap();
        assert_eq!(route.target, HttpTarget::LoopbackHttp { port: 8001 });
        assert_eq!(route.path, "/x");
    }

    /// An address that is neither a path nor https or loopback http is refused, and a refusal is never retried: the
    /// Hub gives the same answer each time.
    #[test]
    fn an_address_rastai_doesnt_follow_is_refused_without_a_retry() {
        let access = access_to(production_hub());
        let current = route_at(production_hub());
        let error = route_for_location(&access, &current, "ftp://files.example/model")
            .err()
            .unwrap();
        assert!(matches!(error, NodeError::UnsafeHubPath { .. }), "{error:?}");
        assert!(!is_interruption(&error));
    }

    /// The fuzz target's entry point accepts an honest file list and refuses nothing it should follow.
    #[test]
    fn the_hub_answer_check_passes_an_honest_listing_and_a_refused_redirect() {
        let listing = br#"{"sha":"0123abcd","siblings":[{"rfilename":"config.json","blobId":"ab12"}]}"#;
        let mut honest = vec![0u8];
        honest.extend_from_slice(listing);
        assert_eq!(check_hub_answer(&honest), Ok(()));
        let mut loopback = vec![2u8];
        loopback.extend_from_slice(b"http://127.0.0.1:9/secret");
        assert_eq!(check_hub_answer(&loopback), Ok(()));
    }

    /// A dropped or stalled connection is still tried again.
    #[test]
    fn a_dropped_connection_is_still_retried() {
        let error = connection_refused("reset by peer".to_string());
        assert!(is_interruption(&error));
    }

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

    #[test]
    fn the_listing_names_the_default_branch_as_main() {
        assert_eq!(
            listing_path("org/name", ""),
            "/api/models/org/name/revision/main?blobs=true"
        );
        assert_eq!(
            listing_path("org/name", "refs/pr/1"),
            "/api/models/org/name/revision/refs%2Fpr%2F1?blobs=true"
        );
    }

    #[test]
    fn progress_is_published_first_last_and_once_per_step() {
        let step = HF_PROGRESS_STEP_BYTES;
        assert!(progress_due(None, 0, 10 * step));
        assert!(!progress_due(Some(0), step / 2, 10 * step));
        assert!(progress_due(Some(0), step, 10 * step));
        assert!(
            progress_due(Some(step), 10 * step, 10 * step),
            "the last report always goes out"
        );
        assert!(
            !progress_due(Some(10 * step), 10 * step, 10 * step),
            "a value already published is not sent twice"
        );
    }

    #[test]
    fn a_redirect_location_names_its_host_and_path() {
        assert_eq!(
            absolute_location("https://us.aws.cdn.hf.co/xet/file?sig=1"),
            Some((
                HttpTarget::Https {
                    host: "us.aws.cdn.hf.co".to_string(),
                    port: 443,
                },
                "/xet/file?sig=1".to_string()
            ))
        );
        assert_eq!(
            absolute_location("http://127.0.0.1:8080/cdn/x"),
            Some((HttpTarget::LoopbackHttp { port: 8080 }, "/cdn/x".to_string()))
        );
        assert_eq!(absolute_location("ftp://example.com/x"), None);
        assert_eq!(
            absolute_location("https://user@evil.example/x"),
            None,
            "userinfo is refused"
        );
        assert_eq!(
            absolute_location("http://example.com/x"),
            None,
            "plain http is only the loopback fake"
        );
    }

    #[test]
    fn a_cdn_request_reports_under_its_own_purpose_and_never_carries_the_token() {
        let (reports, _receiver) = tokio::sync::mpsc::channel(1);
        let access = HubAccess {
            hub: production_hub(),
            egress: Some(EgressTag {
                purpose: HF_DOWNLOAD_EGRESS_PURPOSE.to_string(),
                subject_id: "job_1".to_string(),
                reports,
            }),
            token: Some("hf_secret".to_string()),
        };
        let current = hub_route(&access, "/repo/resolve/c/file".to_string());
        let route = route_for_location(&access, &current, "https://us.aws.cdn.hf.co/xet/file").unwrap();
        let tag = route.egress.clone().unwrap();
        assert_eq!(tag.purpose, HF_CDN_EGRESS_PURPOSE);
        assert_eq!(tag.subject_id, "job_1");
        let request = request_for_route(&access, &route, Some(10), 1_000);
        assert!(
            request.headers.iter().all(|(name, _)| name != "Authorization"),
            "the token stays with the Hub"
        );
        assert!(
            request
                .headers
                .contains(&("Range".to_string(), "bytes=10-".to_string()))
        );
        let hub_request = request_for_route(&access, &current, None, 1_000);
        assert!(
            hub_request.headers.iter().any(|(name, _)| name == "Authorization"),
            "the Hub itself gets the token"
        );
    }

    #[test]
    fn a_content_range_names_the_byte_it_starts_at() {
        assert_eq!(content_range_start("bytes 8000-19999/20000"), Some(8000));
        assert_eq!(content_range_start("items 8000-19999/20000"), None);
        assert_eq!(content_range_start("bytes x-1/2"), None);
    }
}
