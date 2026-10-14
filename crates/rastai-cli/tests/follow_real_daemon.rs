//! Following a job against the real daemon (`rastai serve`, which the first verb starts), not a fake one: the
//! follower's re-read after a reopen must understand the daemon's own `job.get` answer (`JobGetResult`).
//!
//! The job ends before its follower subscribes, so no stream the follower opens ever carries its terminal
//! `job.state_changed`. After its first chunk timeout the follower reopens, reads the job with `job.get` and reports
//! the end at once. The job ends `failed` before it starts on its own: the test's data folder has no cpu environment,
//! so the embedded node lacks the capability the job needs (`RRS-N0004`). Linux only, like the daemon guard.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rastai_cli::daemon_client::DaemonClient;
use rastai_cli::follow::{follow_job, open_job_events};
use rastai_cli::output::OutputSettings;
use rastai_cli::verbs::VerbOutcome;
use rastai_core::versions::APP_VERSION;
use rastai_diag::catalog::find_entry;
use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_proto::common::v1::JobState;
use serde_json::Value;

use common::{DaemonGuard, daemon_socket, run_rastai, test_home};

const DESIGN: &str = "templates/designs/textcnn.rrsmodel";
const DATASET: &str = "fixtures/datasets/textcnn_tiny.jsonl";
const FAILED_STATE: &str = "failed";
/// The code the job fails with on a daemon with no cpu environment (see the module doc).
const NODE_LACKS_CAPABILITY: &str = "RRS-N0004";
/// The job fails as soon as the scheduler offers it to the embedded node; a loaded machine is slower.
const JOB_END_LIMIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(200);
/// Short, so the follower reopens soon, and long enough for the head of a stream on a loaded machine.
const SHORT_CHUNK_TIMEOUT_MS: u64 = 1_000;
/// The follower must end well within this; one that waits for an event already gone never ends.
const FOLLOW_LIMIT: Duration = Duration::from_secs(30);

fn repository_file(relative: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
    path.canonicalize().unwrap()
}

/// `train.start --detach`: the job's id, without following it.
fn start_detached_job(home: &Path) -> String {
    let design = repository_file(DESIGN).display().to_string();
    let dataset = repository_file(DATASET).display().to_string();
    let arguments = [
        "train",
        "start",
        "--recipe",
        "classification",
        "--design-path",
        &design,
        "--dataset-path",
        &dataset,
        "--detach",
        "--json",
    ];
    let result = run_rastai(home, &arguments);
    assert_eq!(result.exit_code, 0, "{result:?}");
    let first_line = result.stdout.lines().next().unwrap();
    let answer: Value = serde_json::from_str(first_line).unwrap();
    answer["job_id"].as_str().unwrap().to_string()
}

/// Waits until `job.get` reads the job as failed.
fn wait_until_failed(home: &Path, job_id: &str) {
    let started = Instant::now();
    loop {
        let result = run_rastai(home, &["job", "get", "--job-id", job_id, "--no-start", "--json"]);
        let first_line = result.stdout.lines().next().unwrap_or_default();
        let answer: Value = serde_json::from_str(first_line).unwrap_or_default();
        if answer["job"]["state"] == FAILED_STATE {
            return;
        }
        assert!(started.elapsed() < JOB_END_LIMIT, "the job never failed: {result:?}");
        std::thread::sleep(POLL);
    }
}

#[test]
fn a_job_that_ended_before_its_follower_subscribed_is_read_with_the_daemons_job_get() {
    let home = test_home("follow-real");
    let _guard = DaemonGuard { home: home.clone() };
    let job_id = start_detached_job(&home);
    wait_until_failed(&home, &job_id);
    let client = DaemonClient {
        socket_path: daemon_socket(&home),
        daemon_version: APP_VERSION.to_string(),
        warnings: Vec::new(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let followed = runtime.block_on(async {
        let events = open_job_events(&client, SHORT_CHUNK_TIMEOUT_MS).await.unwrap();
        let following = follow_job(events, &job_id, OutputSettings::default());
        tokio::time::timeout(FOLLOW_LIMIT, following).await
    });
    let outcome = followed
        .expect("the follower waited for an event that had already been sent")
        .unwrap();
    // The row doesn't keep the state the job left, so the last state the follower saw (none: queued) stands in.
    // The row's error code comes with its catalog title, the job's failure as the follower reports it.
    let failure = Diagnostic {
        code: NODE_LACKS_CAPABILITY.to_string(),
        severity: Severity::Error,
        message: find_entry(NODE_LACKS_CAPABILITY).unwrap().title.to_string(),
        path: String::new(),
        fix_text: find_entry(NODE_LACKS_CAPABILITY).unwrap().fix_text.to_string(),
        fix_command_id: String::new(),
        fix_arguments_json: String::new(),
    };
    assert_eq!(
        outcome,
        VerbOutcome::JobEnded {
            job_id,
            final_state: JobState::Failed,
            from_state: JobState::Queued,
            diagnostics: vec![failure],
        }
    );
}
