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
use fake_hub::{FakeFile, FakeHub, FakeRepo, GatedAccess, patterned_bytes, sha256_hex};
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

fn no_retry_delay() -> RetryPolicy {
    RetryPolicy {
        limit: DOWNLOAD_RETRY_LIMIT,
        first_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    }
}

/// The options that send a download to the fake Hub on loopback, with no waits.
fn options_for(hub: &FakeHub, cache_root: &Path) -> DownloadOptions {
    DownloadOptions {
        hub: HttpTarget::LoopbackHttp { port: hub.port() },
        egress: None,
        cache_root: cache_root.to_path_buf(),
        token: None,
        offline: false,
        free_space_margin_bytes: 0,
        lock_wait: Duration::ZERO,
        lock_poll: Duration::from_millis(10),
        retry: no_retry_delay(),
    }
}

fn request_for(model: &str, revision: &str, only_files: &[&str]) -> DownloadRequest {
    DownloadRequest {
        job_id: "job_test".to_string(),
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
    download_model(&request, &options_for(hub, cache), &SilentObserver)
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

    let error = download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver).await);

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
        });
        let hub = FakeHub::start(vec![tiny_repo(gated)]).await;

        let error = download_error(
            download_model(
                &request_for(MODEL, "", &[]),
                &options_for(&hub, &cache),
                &SilentObserver,
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
        )
        .await,
    );
    let missing_file = download_error(
        download_model(
            &request_for(MODEL, "", &["nope.bin"]),
            &options_for(&hub, &cache),
            &SilentObserver,
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

    let error = download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver).await);

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
        )
        .await,
    );

    assert_eq!(error.code(), "RRS-N3003");
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

    let error = download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver).await);

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
        download_model(&request, &options, &SilentObserver),
        download_model(&request, &options, &SilentObserver)
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
    let lock_file = hf_cache::lock_path(&cache, MODEL, &weights_blob());
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
    let observer = LiveDownloadObserver {
        hub: live.clone(),
        job_id: "job_live".to_string(),
    };

    let outcome = download_model(&request_for(MODEL, "", &[]), &options_for(&hub, &cache), &observer)
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
        reported.last(),
        Some(&outcome.bytes_total),
        "the last report is the whole download"
    );
}

#[tokio::test]
async fn a_token_from_the_source_authorises_a_gated_download() {
    let folder = new_test_folder("hf-token");
    let cache = cache_in(&folder);
    let gated = Some(GatedAccess {
        status: 403,
        granted_token: GRANTED_TOKEN.to_string(),
    });
    let hub = FakeHub::start(vec![tiny_repo(gated)]).await;
    let mut options = options_for(&hub, &cache);
    options.token = FixedToken(Some(GRANTED_TOKEN.to_string())).hf_token();

    download_model(&request_for(MODEL, "", &[WEIGHTS_NAME]), &options, &SilentObserver)
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
    });
    let hub = FakeHub::start(vec![tiny_repo(gated)]).await;
    let mut options = options_for(&hub, &cache);
    options.token = NoHfToken.hf_token();

    let error = download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver).await);

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

    let error = download_error(download_model(&request_for(MODEL, "", &[]), &options, &SilentObserver).await);

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

    let outcome = download_model(&request_for(MODEL, "", &[]), &options_for(&hub, &cache), &observer)
        .await
        .unwrap();

    let progress = observer.progress.lock().unwrap().clone();
    assert_eq!(progress.first(), Some(&(0, outcome.bytes_total)));
    assert_eq!(progress.last(), Some(&(outcome.bytes_total, outcome.bytes_total)));
    assert!(observer.waiting.lock().unwrap().is_empty(), "no lock was waited for");
}
