//! The Hugging Face downloader against a local fake Hub (fixtures/hf-server/fake_hub.rs): the cache layout that other
//! HF tools read, resume after an interruption and after a killed job, gated and missing repositories, offline mode,
//! the SHA-256 check, the disk check, the lock another program holds, two jobs for one file, and the progress on the
//! live queue. Unix only: the snapshot links are symbolic links.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

#[path = "../../../fixtures/hf-server/fake_hub.rs"]
mod fake_hub;

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use common::{TestFolder, new_test_folder};
use fake_hub::{FakeFile, FakeHub, FakeRepo, GatedAccess, Redirect, patterned_bytes, sha256_hex};
use rastai_node::downloads::{
    DOWNLOAD_RETRY_LIMIT, DownloadObserver, DownloadOptions, DownloadOutcome, DownloadRequest, HfTokenSource,
    LiveDownloadObserver, NoHfToken, RetryPolicy, SilentObserver, download_model, offline_requested, production_hub,
};
use rastai_node::error::NodeError;
use rastai_node::hf_cache;
use rastai_node::live::{LiveHub, take_live_event};
use rastai_proto::node::v1::live_event::Payload;
use rastai_transport::http_client::HttpTarget;

const MODEL: &str = "acme/tiny-model";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const CONFIG_NAME: &str = "config.json";
const WEIGHTS_NAME: &str = "model.safetensors";
const SAE_NAME: &str = "layers/0/sae.safetensors";
const WEIGHTS_BYTES: usize = 20_000;
const SAE_BYTES: usize = 4_000;
const CONFIG_BYTES: usize = 300;
const GRANTED_TOKEN: &str = "hf_granted";
/// The repository folder huggingface_hub names for MODEL, and the cpu environment's Python with the layout reader.
const REPO_FOLDER: &str = "models--acme--tiny-model";
const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
const LAYOUT_READER: &str = "tests/support/hf_tools_read.py";

/// Records what the downloader reports, so a test can read it after the download.
#[derive(Default)]
struct RecordingObserver {
    progress: Mutex<Vec<(u64, u64)>>,
    waiting: Mutex<Vec<String>>,
}

impl DownloadObserver for RecordingObserver {
    fn progress(&self, _model_ref: &str, bytes_done: u64, bytes_total: u64) {
        self.progress.lock().unwrap().push((bytes_done, bytes_total));
    }

    fn waiting_for_lock(&self, file_name: &str) {
        self.waiting.lock().unwrap().push(file_name.to_string());
    }
}

/// A token source with a fixed answer, standing in for M2-18's GetSecret.
struct FixedToken(Option<String>);

impl HfTokenSource for FixedToken {
    fn hf_token(&self) -> Option<String> {
        self.0.clone()
    }
}

fn tiny_repo(gated: Option<GatedAccess>) -> FakeRepo {
    FakeRepo {
        name: MODEL.to_string(),
        commit: COMMIT.to_string(),
        files: vec![
            FakeFile::plain(CONFIG_NAME, patterned_bytes(CONFIG_BYTES, 1)),
            FakeFile::lfs(WEIGHTS_NAME, patterned_bytes(WEIGHTS_BYTES, 2)),
            FakeFile::lfs(SAE_NAME, patterned_bytes(SAE_BYTES, 3)),
        ],
        gated,
    }
}

fn weights_content() -> Vec<u8> {
    patterned_bytes(WEIGHTS_BYTES, 2)
}

/// The blob name of an LFS file is its SHA-256.
fn weights_blob() -> String {
    sha256_hex(&weights_content())
}

fn sae_content() -> Vec<u8> {
    patterned_bytes(SAE_BYTES, 3)
}

/// The blob name of the nested SAE file, its SHA-256.
fn sae_blob() -> String {
    sha256_hex(&sae_content())
}

fn no_retry_delay() -> RetryPolicy {
    RetryPolicy {
        limit: DOWNLOAD_RETRY_LIMIT,
        first_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    }
}

/// The options that send a download to the fake Hub on loopback, with no waits.
fn options_for(hub: &FakeHub, cache_root: &Path) -> DownloadOptions {
    options_for_port(hub.port(), cache_root)
}

fn options_for_port(port: u16, cache_root: &Path) -> DownloadOptions {
    DownloadOptions {
        hub: HttpTarget::LoopbackHttp { port },
        egress: None,
        cache_root: cache_root.to_path_buf(),
        offline: false,
        free_space_margin_bytes: 0,
        lock_wait: Duration::ZERO,
        lock_poll: Duration::from_millis(10),
        retry: no_retry_delay(),
    }
}

fn request_for(model: &str, revision: &str, only_files: &[&str]) -> DownloadRequest {
    DownloadRequest {
        model_ref: model.to_string(),
        revision: revision.to_string(),
        only_files: only_files.iter().map(|name| name.to_string()).collect(),
    }
}

fn cache_in(folder: &TestFolder) -> PathBuf {
    folder.path.join("hub")
}

fn download_error(result: Result<DownloadOutcome, NodeError>) -> NodeError {
    match result {
        Err(error) => error,
        Ok(outcome) => panic!("expected the download to be refused, it finished with {outcome:?}"),
    }
}

async fn download_all(hub: &FakeHub, cache: &Path) -> DownloadOutcome {
    let request = request_for(MODEL, "", &[]);
    download_model(&request, &options_for(hub, cache), &SilentObserver, &NoHfToken)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_download_lays_out_the_hub_cache_so_other_tools_read_it() {
    let folder = new_test_folder("hf-layout");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;

    let outcome = download_all(&hub, &cache).await;

    assert_eq!(outcome.commit, COMMIT);
    assert_eq!(outcome.files, vec![CONFIG_NAME, WEIGHTS_NAME, SAE_NAME]);
    assert_eq!(outcome.fetched_files.len(), 3, "a first download fetches every file");
    let repo = hf_cache::repo_dir(&cache, MODEL);
    assert_eq!(repo, cache.join("models--acme--tiny-model"));
    let snapshot = hf_cache::snapshot_path(&repo, COMMIT, WEIGHTS_NAME);
    assert_eq!(
        fs::read(&snapshot).unwrap(),
        weights_content(),
        "the snapshot reads the file's bytes"
    );
    let link = fs::read_link(&snapshot).unwrap();
    assert_eq!(link, hf_cache::snapshot_link_target(WEIGHTS_NAME, &weights_blob()));
    assert!(link.is_relative(), "the link is relative, so the cache can move");
    let sae = hf_cache::snapshot_path(&repo, COMMIT, SAE_NAME);
    assert_eq!(
        fs::read(&sae).unwrap().len(),
        SAE_BYTES,
        "a file in a folder is linked from there"
    );
    assert_eq!(fs::read_to_string(hf_cache::ref_path(&repo, "main")).unwrap(), COMMIT);
    let incomplete = fs::read_dir(repo.join("blobs"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".incomplete")
        })
        .count();
    assert_eq!(incomplete, 0, "no partial file is left once the downloads finish");
    assert!(
        cache.join(".locks").join("models--acme--tiny-model").is_dir(),
        "the HF lock folder is used"
    );
}

#[tokio::test]
async fn a_second_download_links_what_is_already_there_and_fetches_nothing() {
    let folder = new_test_folder("hf-again");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    download_all(&hub, &cache).await;

    let second = download_all(&hub, &cache).await;

    assert!(
        second.fetched_files.is_empty(),
        "the blobs are there, so nothing is fetched"
    );
    assert_eq!(
        hub.file_requests(WEIGHTS_NAME).len(),
        1,
        "the file was requested by the first download only"
    );
}

#[tokio::test]
async fn an_interrupted_download_resumes_from_its_length_and_the_hash_is_checked() {
    let folder = new_test_folder("hf-resume");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files[1].drop_after = Some(WEIGHTS_BYTES / 2);
    let hub = FakeHub::start(vec![repo]).await;

    let outcome = download_all(&hub, &cache).await;

    let requests = hub.file_requests(WEIGHTS_NAME);
    assert_eq!(requests.len(), 2, "the first answer dropped halfway, so one retry");
    assert_eq!(requests[0].range, None);
    assert_eq!(
        requests[1].range.as_deref(),
        Some("bytes=10000-"),
        "the retry asks for the rest only"
    );
    assert!(outcome.fetched_files.contains(&WEIGHTS_NAME.to_string()));
    let blob = hf_cache::blob_path(&hf_cache::repo_dir(&cache, MODEL), &weights_blob());
    assert_eq!(
        fs::read(blob).unwrap(),
        weights_content(),
        "the whole file arrived and matches its hash"
    );
}

#[tokio::test]
async fn a_partial_file_from_a_killed_job_resumes_at_its_length() {
    let folder = new_test_folder("hf-killed");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);
    let incomplete = hf_cache::incomplete_path(&repo_dir, &weights_blob());
    fs::create_dir_all(incomplete.parent().unwrap()).unwrap();
    let first_half = weights_content()[..8_000].to_vec();
    fs::write(&incomplete, &first_half).unwrap();

    download_all(&hub, &cache).await;

    let requests = hub.file_requests(WEIGHTS_NAME);
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].range.as_deref(),
        Some("bytes=8000-"),
        "the bytes already on disk are not asked again"
    );
    let blob = hf_cache::blob_path(&repo_dir, &weights_blob());
    assert_eq!(fs::read(blob).unwrap(), weights_content());
}

#[tokio::test]
async fn a_server_that_ignores_the_range_makes_the_partial_file_start_over() {
    let folder = new_test_folder("hf-ignores-range");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files[1].drop_after = Some(5_000);
    repo.files[1].ignores_range = true;
    let hub = FakeHub::start(vec![repo]).await;

    download_all(&hub, &cache).await;

    let blob = hf_cache::blob_path(&hf_cache::repo_dir(&cache, MODEL), &weights_blob());
    assert_eq!(
        fs::read(blob).unwrap(),
        weights_content(),
        "the full body replaced the partial bytes, not appended"
    );
}

#[tokio::test]
async fn retries_stop_at_the_limit_and_keep_what_arrived() {
    let folder = new_test_folder("hf-retry-limit");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files[1].drop_after = Some(5_000);
    repo.files[1].drop_every_time = true;
    let hub = FakeHub::start(vec![repo]).await;
    let mut options = options_for(&hub, &cache);
    options.retry.limit = 3;

    let error =
        download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver, &NoHfToken).await);

    assert_eq!(error.code(), "RRS-N3009");
    assert_eq!(hub.file_requests(WEIGHTS_NAME).len(), 3, "three tries in all");
    let incomplete = hf_cache::incomplete_path(&hf_cache::repo_dir(&cache, MODEL), &weights_blob());
    assert_eq!(
        fs::metadata(incomplete).unwrap().len(),
        15_000,
        "the three partial answers are kept to resume later"
    );
}

#[tokio::test]
async fn a_gated_repository_is_refused_with_the_gated_code_for_401_and_403() {
    for status in [401, 403] {
        let folder = new_test_folder("hf-gated");
        let cache = cache_in(&folder);
        let gated = Some(GatedAccess {
            status,
            granted_token: GRANTED_TOKEN.to_string(),
            resolve_only: false,
        });
        let hub = FakeHub::start(vec![tiny_repo(gated)]).await;

        let error = download_error(
            download_model(
                &request_for(MODEL, "", &[]),
                &options_for(&hub, &cache),
                &SilentObserver,
                &NoHfToken,
            )
            .await,
        );

        assert_eq!(error.code(), "RRS-N3005", "status {status} is the gated-model code");
        assert!(
            !cache
                .join("models--acme--tiny-model")
                .join("blobs")
                .join(weights_blob())
                .exists()
        );
    }
}

#[tokio::test]
async fn a_missing_repository_or_file_is_not_found() {
    let folder = new_test_folder("hf-missing");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;

    let missing_repo = download_error(
        download_model(
            &request_for("acme/nope", "", &[]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );
    let missing_file = download_error(
        download_model(
            &request_for(MODEL, "", &["nope.bin"]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(missing_repo.code(), "RRS-N3004");
    assert_eq!(missing_file.code(), "RRS-N3004");
}

#[tokio::test]
async fn offline_mode_refuses_before_any_request() {
    let folder = new_test_folder("hf-offline");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let mut options = options_for(&hub, &cache);
    options.offline = true;

    let error =
        download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver, &NoHfToken).await);

    assert_eq!(error.code(), "RRS-N3006");
    assert!(hub.requests().is_empty(), "offline means no network call at all");
}

#[test]
fn offline_mode_follows_huggingface_hub_words_and_the_setting() {
    assert!(!offline_requested(None, false));
    assert!(offline_requested(Some("1"), false));
    assert!(offline_requested(Some("TRUE"), false));
    assert!(offline_requested(Some(" yes "), false));
    assert!(!offline_requested(Some("0"), false));
    assert!(!offline_requested(Some("off"), false));
    assert!(
        offline_requested(None, true),
        "the hf.offline setting is enough on its own"
    );
}

#[tokio::test]
async fn a_hash_mismatch_removes_the_partial_file_and_refuses() {
    let folder = new_test_folder("hf-hash");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files[1].listed_sha256 = Some("00".repeat(32));
    let hub = FakeHub::start(vec![repo]).await;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3003");
    let requests = hub.file_requests(WEIGHTS_NAME);
    assert_eq!(
        requests.len(),
        2,
        "one more try after the first mismatch, the second mismatch is the refusal"
    );
    assert_eq!(
        requests[1].range, None,
        "the partial file was removed, so the second try starts from zero"
    );
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);
    assert!(!hf_cache::incomplete_path(&repo_dir, &"00".repeat(32)).exists());
    assert!(
        !hf_cache::blob_path(&repo_dir, &"00".repeat(32)).exists(),
        "no file with the wrong hash is kept"
    );
}

#[tokio::test]
async fn the_disk_check_refuses_before_any_file_is_fetched() {
    let folder = new_test_folder("hf-disk");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let mut options = options_for(&hub, &cache);
    options.free_space_margin_bytes = u64::MAX / 2;

    let error =
        download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver, &NoHfToken).await);

    assert_eq!(error.code(), "RRS-S3001");
    assert!(hub.file_requests(WEIGHTS_NAME).is_empty(), "no file was requested");
}

#[tokio::test]
async fn two_jobs_for_one_file_fetch_it_once_and_both_finish() {
    let folder = new_test_folder("hf-two-jobs");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let mut options = options_for(&hub, &cache);
    // The second job waits for the first to release the lock; the cap is far above the time that takes.
    options.lock_wait = Duration::from_secs(30);
    let request = request_for(MODEL, "", &[WEIGHTS_NAME]);

    let (first, second) = tokio::join!(
        download_model(&request, &options, &SilentObserver, &NoHfToken),
        download_model(&request, &options, &SilentObserver, &NoHfToken)
    );

    first.unwrap();
    second.unwrap();
    assert_eq!(
        hub.file_requests(WEIGHTS_NAME).len(),
        1,
        "the file crossed the network once"
    );
    let blob = hf_cache::blob_path(&hf_cache::repo_dir(&cache, MODEL), &weights_blob());
    assert_eq!(fs::read(blob).unwrap(), weights_content());
}

#[tokio::test]
async fn a_lock_another_program_holds_times_out_with_its_code_and_is_announced() {
    let folder = new_test_folder("hf-lock");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let lock_file = rastai_core::paths::hf_lock_path(&cache, &hf_cache::repo_folder_name(MODEL), &weights_blob());
    fs::create_dir_all(lock_file.parent().unwrap()).unwrap();
    let holder = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_file)
        .unwrap();
    holder.try_lock().unwrap();
    let observer = RecordingObserver::default();

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[WEIGHTS_NAME]),
            &options_for(&hub, &cache),
            &observer,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3011");
    assert_eq!(
        observer.waiting.lock().unwrap().clone(),
        vec![WEIGHTS_NAME.to_string()],
        "the wait is announced once"
    );
    assert!(
        hub.file_requests(WEIGHTS_NAME).is_empty(),
        "nothing is fetched while another program holds the file"
    );
    drop(holder);
}

#[tokio::test]
async fn progress_reaches_the_live_queue_as_download_progress() {
    let folder = new_test_folder("hf-live");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let live = std::sync::Arc::new(LiveHub::default());
    let observer = LiveDownloadObserver::new(live.clone(), "job_live");

    let outcome = download_model(
        &request_for(MODEL, "", &[]),
        &options_for(&hub, &cache),
        &observer,
        &NoHfToken,
    )
    .await
    .unwrap();

    let mut reported = Vec::new();
    while let Some(event) = take_live_event(&live) {
        assert_eq!(event.job_id, "job_live");
        if let Some(Payload::DownloadProgress(progress)) = event.payload {
            assert_eq!(progress.model_ref, MODEL);
            assert_eq!(progress.bytes_total, outcome.bytes_total);
            reported.push(progress.bytes_done);
        }
    }
    assert!(!reported.is_empty(), "download.progress reaches WatchLive");
    assert_eq!(
        reported,
        vec![0, outcome.bytes_total],
        "the start and the end are reported, and the chunks and files between them are not: a file under one step adds no event"
    );
}

#[tokio::test]
async fn a_token_from_the_source_authorises_a_gated_download() {
    let folder = new_test_folder("hf-token");
    let cache = cache_in(&folder);
    let gated = Some(GatedAccess {
        status: 403,
        granted_token: GRANTED_TOKEN.to_string(),
        resolve_only: false,
    });
    let hub = FakeHub::start(vec![tiny_repo(gated)]).await;
    let options = options_for(&hub, &cache);

    download_model(
        &request_for(MODEL, "", &[WEIGHTS_NAME]),
        &options,
        &SilentObserver,
        &FixedToken(Some(GRANTED_TOKEN.to_string())),
    )
    .await
    .unwrap();

    let listing = hub.requests()[0].clone();
    assert_eq!(listing.authorization.as_deref(), Some("Bearer hf_granted"));
}

#[tokio::test]
async fn without_a_token_a_gated_download_is_refused() {
    let folder = new_test_folder("hf-no-token");
    let cache = cache_in(&folder);
    let gated = Some(GatedAccess {
        status: 401,
        granted_token: GRANTED_TOKEN.to_string(),
        resolve_only: false,
    });
    let hub = FakeHub::start(vec![tiny_repo(gated)]).await;
    let options = options_for(&hub, &cache);

    let error =
        download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver, &NoHfToken).await);

    assert_eq!(error.code(), "RRS-N3005");
    assert_eq!(
        hub.requests()[0].authorization,
        None,
        "no header is sent without a token"
    );
}

#[tokio::test]
async fn only_the_chosen_files_are_fetched() {
    let folder = new_test_folder("hf-chosen");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;

    let outcome = download_model(
        &request_for(MODEL, "", &[CONFIG_NAME]),
        &options_for(&hub, &cache),
        &SilentObserver,
        &NoHfToken,
    )
    .await
    .unwrap();

    assert_eq!(outcome.files, vec![CONFIG_NAME]);
    assert!(hub.file_requests(WEIGHTS_NAME).is_empty());
}

#[tokio::test]
async fn a_revision_names_its_ref_and_a_commit_names_none() {
    let folder = new_test_folder("hf-revision");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);

    download_model(
        &request_for(MODEL, "main", &[CONFIG_NAME]),
        &options_for(&hub, &cache),
        &SilentObserver,
        &NoHfToken,
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(hf_cache::ref_path(&repo_dir, "main")).unwrap(),
        COMMIT
    );

    download_model(
        &request_for(MODEL, COMMIT, &[CONFIG_NAME]),
        &options_for(&hub, &cache),
        &SilentObserver,
        &NoHfToken,
    )
    .await
    .unwrap();
    assert!(
        !hf_cache::ref_path(&repo_dir, COMMIT).exists(),
        "a commit is not a ref name"
    );
}

#[tokio::test]
async fn a_name_that_would_leave_the_cache_is_refused_before_any_request() {
    let folder = new_test_folder("hf-escape");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;

    let error = download_error(
        download_model(
            &request_for("../escape", "", &[]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3001");
    assert!(hub.requests().is_empty());
    assert!(
        !folder.path.join("escape").exists(),
        "nothing was written outside the cache"
    );
}

#[tokio::test]
async fn a_listed_file_name_that_would_leave_the_cache_is_refused() {
    let folder = new_test_folder("hf-listed-escape");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files
        .push(FakeFile::plain("../../escaped.txt", patterned_bytes(10, 9)));
    let hub = FakeHub::start(vec![repo]).await;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3001");
    assert!(!folder.path.join("escaped.txt").exists());
}

#[tokio::test]
async fn a_download_to_the_hub_without_an_egress_tag_is_refused_before_connecting() {
    let folder = new_test_folder("hf-untagged");
    let cache = cache_in(&folder);
    let mut options = options_for(&FakeHub::start(Vec::new()).await, &cache);
    options.hub = production_hub();

    let error =
        download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver, &NoHfToken).await);

    assert_eq!(
        error.code(),
        "RRS-N3008",
        "an HTTPS request without its tag never leaves the node"
    );
}

#[tokio::test]
async fn the_observer_hears_each_file_as_it_completes() {
    let folder = new_test_folder("hf-observer");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let observer = RecordingObserver::default();

    let outcome = download_model(
        &request_for(MODEL, "", &[]),
        &options_for(&hub, &cache),
        &observer,
        &NoHfToken,
    )
    .await
    .unwrap();

    let progress = observer.progress.lock().unwrap().clone();
    assert_eq!(progress.first(), Some(&(0, outcome.bytes_total)));
    assert_eq!(progress.last(), Some(&(outcome.bytes_total, outcome.bytes_total)));
    assert!(observer.waiting.lock().unwrap().is_empty(), "no lock was waited for");
}

/// The gated-model test for a repository whose listing is open and whose files are gated: the Hub lists such a
/// repository to anyone and answers the file with 401. The download must still stop with RRS-N3005 and its fix.
#[tokio::test]
async fn a_gate_on_the_files_alone_is_the_gated_code_with_its_license_fix() {
    let folder = new_test_folder("hf-gated-files");
    let cache = cache_in(&folder);
    let gated = Some(GatedAccess {
        status: 401,
        granted_token: GRANTED_TOKEN.to_string(),
        resolve_only: true,
    });
    let hub = FakeHub::start(vec![tiny_repo(gated)]).await;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3005");
    let diagnostic = rastai_node::error::node_error_diagnostic(&error);
    assert_eq!(
        diagnostic.fix_command_id, "hf.license.check",
        "the gated code offers the license check as its fix"
    );
    assert_eq!(
        hub.file_requests(CONFIG_NAME).len(),
        1,
        "the listing answered, and the first file was refused"
    );
}

/// A file that the Hub sends to its CDN: the client follows the 302 with its Range, the CDN request carries no token,
/// and the bytes arrive whole. The partial file here is 8,000 bytes, so the CDN sees a Range from there.
#[tokio::test]
async fn a_file_redirected_to_the_cdn_resumes_there_without_the_token() {
    let folder = new_test_folder("hf-cdn");
    let cache = cache_in(&folder);
    let cdn = FakeHub::start(vec![tiny_repo(None)]).await;
    let mut hub_repo = tiny_repo(None);
    hub_repo.files[1].redirect = Redirect::ToCdn;
    let hub = FakeHub::start_with_cdn(vec![hub_repo], &cdn).await;
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);
    let incomplete = hf_cache::incomplete_path(&repo_dir, &weights_blob());
    fs::create_dir_all(incomplete.parent().unwrap()).unwrap();
    fs::write(&incomplete, &weights_content()[..8_000]).unwrap();

    download_model(
        &request_for(MODEL, "", &[WEIGHTS_NAME]),
        &options_for(&hub, &cache),
        &SilentObserver,
        &FixedToken(Some(GRANTED_TOKEN.to_string())),
    )
    .await
    .unwrap();

    let hub_file = hub.file_requests(WEIGHTS_NAME);
    assert_eq!(hub_file.len(), 1, "the Hub answered the file with a redirect");
    assert_eq!(
        hub_file[0].authorization.as_deref(),
        Some("Bearer hf_granted"),
        "the Hub gets the token"
    );
    let cdn_file = cdn.file_requests(WEIGHTS_NAME);
    assert_eq!(cdn_file.len(), 1, "the CDN answered the rest of the file");
    assert_eq!(
        cdn_file[0].range.as_deref(),
        Some("bytes=8000-"),
        "the Range travels to the CDN"
    );
    assert_eq!(cdn_file[0].authorization, None, "the token never reaches the CDN host");
    let blob = hf_cache::blob_path(&repo_dir, &weights_blob());
    assert_eq!(fs::read(blob).unwrap(), weights_content());
}

/// A 206 whose Content-Range starts at another byte than the one asked for is a connection that went wrong. The client
/// must not append its bytes to the partial file, so the partial file keeps its 8,000 bytes.
#[tokio::test]
async fn a_range_that_starts_at_the_wrong_byte_is_refused_and_nothing_is_appended() {
    let folder = new_test_folder("hf-range-start");
    let cache = cache_in(&folder);
    let mut repo = tiny_repo(None);
    repo.files[1].wrong_content_range = true;
    let hub = FakeHub::start(vec![repo]).await;
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);
    let incomplete = hf_cache::incomplete_path(&repo_dir, &weights_blob());
    fs::create_dir_all(incomplete.parent().unwrap()).unwrap();
    fs::write(&incomplete, &weights_content()[..8_000]).unwrap();
    let mut options = options_for(&hub, &cache);
    options.retry.limit = 3;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[WEIGHTS_NAME]),
            &options,
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(
        error.code(),
        "RRS-N3009",
        "the wrong range is retried like a dropped connection"
    );
    assert_eq!(
        fs::metadata(&incomplete).unwrap().len(),
        8_000,
        "no byte of the wrong answer was written"
    );
}

/// A disk that fills while the file is written is RRS-S3002, and the partial file stays for a later job. `/dev/full`
/// opens for writing and fails every write with ENOSPC, which is a full disk, so the partial file is that device.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_full_disk_while_writing_is_rrs_s3002_and_the_partial_file_is_kept() {
    let folder = new_test_folder("hf-full-disk");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    let repo_dir = hf_cache::repo_dir(&cache, MODEL);
    let incomplete = hf_cache::incomplete_path(&repo_dir, &weights_blob());
    fs::create_dir_all(incomplete.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/dev/full", &incomplete).unwrap();

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[WEIGHTS_NAME]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-S3002");
    assert!(
        fs::symlink_metadata(&incomplete).is_ok(),
        "the partial file is kept, so a later job resumes it"
    );
}

/// The cache the node wrote has the layout huggingface_hub writes: `refs/`, `snapshots/<commit>/` links into `blobs/`,
/// and the `.locks/` files. The reader checks it against the recording in `hf_cache_recorded.json` (huggingface_hub
/// 1.33.0) with the standard library only, so no Hugging Face package is needed. It is #[ignore]d because it runs a
/// Python script; `just test-node-jobs` runs it with the cpu environment in place.
#[tokio::test]
#[ignore]
async fn the_cache_the_node_wrote_has_the_huggingface_hub_layout() {
    let folder = new_test_folder("hf-layout-read");
    let cache = downloaded_tiny_repo(&folder).await;

    let output = run_layout_reader(&cache);

    assert!(
        output.status.success(),
        "the cache doesn't match the recorded layout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The reader checks the link target of a nested file, not only of the required ones. A link one level too shallow
/// dangles, and the reader refuses it.
#[tokio::test]
#[ignore]
async fn the_reader_refuses_a_nested_link_that_dangles() {
    let folder = new_test_folder("hf-layout-dangling");
    let cache = downloaded_tiny_repo(&folder).await;
    relink(&snapshot_link(&cache, SAE_NAME), &Path::new("../../blobs").join(sae_blob()));

    let output = run_layout_reader(&cache);

    assert!(!output.status.success(), "a nested link one level too shallow was accepted");
}

/// A link to the blob by its absolute path is not the relative link huggingface_hub writes, so the reader refuses it.
#[tokio::test]
#[ignore]
async fn the_reader_refuses_a_nested_link_with_an_absolute_target() {
    let folder = new_test_folder("hf-layout-absolute");
    let cache = downloaded_tiny_repo(&folder).await;
    let blob = fs::canonicalize(cache.join(REPO_FOLDER).join("blobs").join(sae_blob())).unwrap();
    relink(&snapshot_link(&cache, SAE_NAME), &blob);

    let output = run_layout_reader(&cache);

    assert!(!output.status.success(), "a nested link with an absolute target was accepted");
}

/// A file one folder down links three levels up to blobs/, one more than a file in the snapshot's top folder. A reader
/// that fixed every nested link at the depth of the recorded one would refuse this layout, which huggingface_hub writes.
#[tokio::test]
#[ignore]
async fn a_nested_file_in_one_folder_links_with_its_own_depth() {
    let folder = new_test_folder("hf-layout-one-folder");
    let cache = downloaded_tiny_repo(&folder).await;
    let link = snapshot_link(&cache, "a/b.bin");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(Path::new("../../../blobs").join(sae_blob()), &link).unwrap();

    let output = run_layout_reader(&cache);

    assert!(
        output.status.success(),
        "a one-folder file's link was refused: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Downloads the tiny repository into a fresh cache and returns the cache folder, so a test can change it.
async fn downloaded_tiny_repo(folder: &TestFolder) -> PathBuf {
    let cache = cache_in(folder);
    let hub = FakeHub::start(vec![tiny_repo(None)]).await;
    download_all(&hub, &cache).await;
    cache
}

/// The snapshot link of one file of the tiny repository, under the commit's folder.
fn snapshot_link(cache: &Path, file_name: &str) -> PathBuf {
    cache.join(REPO_FOLDER).join("snapshots").join(COMMIT).join(file_name)
}

/// Replaces a snapshot link with a symbolic link to `target`, the way a tool that rewrote the cache would.
fn relink(link: &Path, target: &Path) {
    fs::remove_file(link).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// Runs the layout reader on a cache, with the cpu environment's Python, and returns its exit status and stderr.
fn run_layout_reader(cache: &Path) -> std::process::Output {
    std::process::Command::new(CPU_PYTHON)
        .arg("-I")
        .arg(LAYOUT_READER)
        .arg(cache)
        .arg(MODEL)
        .arg(COMMIT)
        .arg(CONFIG_NAME)
        .output()
        .unwrap()
}

/// A redirect with no Location is refused at once. The node asks once: a retry would get the same answer, so the code
/// is RRS-N3013 and not the connection's retries running out.
#[tokio::test]
async fn a_redirect_with_no_location_is_refused_at_once() {
    let folder = new_test_folder("hf-redirect-no-location");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![redirecting_repo(Redirect::NoLocation)]).await;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[CONFIG_NAME]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3013");
    assert_eq!(hub.file_requests(CONFIG_NAME).len(), 1, "a refused redirect is not retried");
}

/// Five redirects are followed; the sixth answer is a redirect too, so the chain is refused after six requests, at once.
#[tokio::test]
async fn a_redirect_chain_past_five_hops_is_refused_at_once() {
    let folder = new_test_folder("hf-redirect-loop");
    let cache = cache_in(&folder);
    let hub = FakeHub::start(vec![redirecting_repo(Redirect::LoopsToItself)]).await;

    let error = download_error(
        download_model(
            &request_for(MODEL, "", &[CONFIG_NAME]),
            &options_for(&hub, &cache),
            &SilentObserver,
            &NoHfToken,
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3013");
    assert_eq!(
        hub.file_requests(CONFIG_NAME).len(),
        6,
        "five hops are followed, then the sixth redirect is refused"
    );
}

/// The tiny repository with only its config, which redirects as the test says.
fn redirecting_repo(redirect: Redirect) -> FakeRepo {
    let mut config = FakeFile::plain(CONFIG_NAME, patterned_bytes(CONFIG_BYTES, 1));
    config.redirect = redirect;
    FakeRepo {
        name: MODEL.to_string(),
        commit: COMMIT.to_string(),
        files: vec![config],
        gated: None,
    }
}
