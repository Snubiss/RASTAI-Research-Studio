//! The TextCNN train job through the whole daemon (plan M1-47; flows/platform.md 2.3 and 5.1, training flow 4.1):
//! `train.start` answers a job id at once, the scheduler launches the job on the embedded node, the real worker
//! trains in the cpu environment, and the run tail indexes every record. `job.cancel` ends the job and its run
//! `cancelled`; a daemon restart mid-run reconciles through `ListJobs` and the tail goes on from `acked_sequence`.
//!
//! It runs the real worker in the cpu environment's Python (`just env-cpu`), which a plain `cargo test` on a CI
//! runner doesn't have, so its tests are `#[ignore]`d and `just test-daemon-jobs` runs them (`just check` includes
//! it). Linux only, like the node's job tests.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used)]
mod on_linux {
    use std::collections::BTreeSet;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use rastai_core::paths::{DataPaths, environments_dir, run_dir, run_events_log_path};
    use rastai_core::principal::owner_principal;
    use rastai_core::scopes::Scope;
    use rastai_proto::commands::v1::ExecuteResponse;
    use rastai_proto::events::v1::EventRecord;
    use rastai_proto::events::v1::event_payload::Payload;
    use rastai_server::daemon::RunningDaemon;
    use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
    use rastai_store::index::reads::{list_table_rows, open_index_reader};
    use rastai_store::index::runs::{JobRow, RunRow, get_job, get_run};
    use serde_json::{Value, json};

    use crate::common::{
        execute_request, ok_result, remove_test_folder, response_codes, run_command, start_test_daemon,
        stop_test_daemon, test_paths, token_principal,
    };

    /// The cpu environment's interpreter, the design and the dataset, relative to this crate.
    const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
    const DESIGN: &str = "../../templates/designs/textcnn.rrsmodel";
    const DATASET: &str = "../../fixtures/datasets/textcnn_tiny.jsonl";
    /// `<data>/envs/<environment_id>-<lock_hash12>/`: the embedded node finds the cpu environment here (daemon.rs).
    const ENVIRONMENT_FOLDER: &str = "cpu-000000000000";
    /// Starting Python and torch takes a few seconds and the run 400 steps; a loaded machine is slower.
    const RUN_LIMIT: Duration = Duration::from_secs(300);
    /// "At once": the answer doesn't wait for the job, only for its rows.
    const ANSWER_LIMIT: Duration = Duration::from_secs(5);
    const POLL: Duration = Duration::from_millis(100);
    const EXECUTABLE_MODE: u32 = 0o755;
    const RUN_METRICS: &str = "run.metrics";
    const ENDED_JOB_STATES: &[&str] = &["succeeded", "failed", "cancelled"];
    /// The worker's exit after CANCEL (contracts 9.1).
    const EXIT_CANCELLED: &str = "3";

    /// A test daemon whose embedded node runs jobs in the repository's cpu environment.
    async fn start_daemon_with_cpu_environment(paths: &DataPaths) -> RunningDaemon {
        link_cpu_environment(paths);
        start_test_daemon(paths).await
    }

    /// `<data>/envs/cpu-.../bin/python` runs the cpu environment's interpreter with the node's arguments.
    fn link_cpu_environment(paths: &DataPaths) {
        let python = crate_path(CPU_PYTHON);
        assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
        let bin = environments_dir(paths).join(ENVIRONMENT_FOLDER).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = format!("#!/bin/sh\nexec '{}' \"$@\"\n", python.display());
        let launcher = bin.join("python");
        std::fs::write(&launcher, script).unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
    }

    /// A path relative to this crate. Never canonicalized: the cpu environment's `bin/python` is a link whose own
    /// path is what makes Python use the environment.
    fn crate_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    async fn start_training(running: &RunningDaemon) -> (ExecuteResponse, String, String) {
        let arguments = json!({
            "recipe": "classification",
            "design_path": crate_path(DESIGN),
            "dataset_path": crate_path(DATASET),
        });
        let request = execute_request("train.start", &arguments.to_string());
        let asked = Instant::now();
        let response = run_command(&running.server, &owner_principal(), request).await;
        assert!(asked.elapsed() < ANSWER_LIMIT, "train.start took {:?}", asked.elapsed());
        let result = ok_result(&response);
        let run_id = result["run_id"].as_str().unwrap().to_string();
        let job_id = response.job_id.clone();
        (response, run_id, job_id)
    }

    fn job_row(running: &RunningDaemon, job_id: &str) -> JobRow {
        let reader = open_index_reader(&running.server.store.index_readers).unwrap();
        get_job(&reader, job_id).unwrap().unwrap()
    }

    fn run_row(running: &RunningDaemon, run_id: &str) -> RunRow {
        let reader = open_index_reader(&running.server.store.index_readers).unwrap();
        get_run(&reader, run_id).unwrap().unwrap()
    }

    fn stderr_log(paths: &DataPaths, run_id: &str) -> String {
        std::fs::read_to_string(run_dir(paths, run_id).join("stderr.log")).unwrap_or_default()
    }

    /// Waits until the job's row is in `state` and the run's row in `run_state`. A job that ended otherwise fails
    /// the test at once.
    async fn wait_for_end(running: &RunningDaemon, paths: &DataPaths, job_id: &str, run_id: &str, ends: (&str, &str)) {
        let started = Instant::now();
        loop {
            let job = job_row(running, job_id);
            let run = run_row(running, run_id);
            if (job.state.as_str(), run.state.as_str()) == ends {
                return;
            }
            let ended_otherwise = ENDED_JOB_STATES.contains(&job.state.as_str()) && job.state != ends.0;
            assert!(
                started.elapsed() < RUN_LIMIT && !ended_otherwise,
                "job {job:?}, run {run:?}; its stderr.log:\n{}",
                stderr_log(paths, run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Waits until at least one of the run's metrics is indexed.
    async fn wait_for_first_metric(running: &RunningDaemon, paths: &DataPaths, run_id: &str) {
        let started = Instant::now();
        while metric_rows(running, run_id).is_empty() {
            assert!(
                started.elapsed() < RUN_LIMIT,
                "no metric was indexed; stderr.log:\n{}",
                stderr_log(paths, run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Waits until the run has more metric values indexed than `seen`: the job is still training.
    async fn wait_for_more_metrics(running: &RunningDaemon, paths: &DataPaths, run_id: &str, seen: usize) {
        let started = Instant::now();
        while metric_rows(running, run_id).len() <= seen {
            assert!(
                started.elapsed() < RUN_LIMIT,
                "the job stopped training; stderr.log:\n{}",
                stderr_log(paths, run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// The run's `run_metrics` rows as (sequence, name).
    fn metric_rows(running: &RunningDaemon, run_id: &str) -> BTreeSet<(u64, String)> {
        let reader = open_index_reader(&running.server.store.index_readers).unwrap();
        let rows = list_table_rows(&reader, "run_metrics").unwrap();
        // Columns: run_id, sequence, step, name, value, emitted_unix_ms (contracts 8.2); text comes quoted.
        let quoted_run_id = format!("{run_id:?}");
        rows.into_iter()
            .filter(|row| row[0] == quoted_run_id)
            .map(|row| (row[1].parse().unwrap(), row[3].trim_matches('"').to_string()))
            .collect()
    }

    /// The run's log, record by record.
    fn log_records(paths: &DataPaths, run_id: &str) -> Vec<EventRecord> {
        let mut reader = open_event_log(&run_events_log_path(paths, run_id)).unwrap();
        let mut records = Vec::new();
        while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
            records.push(record);
        }
        records
    }

    /// Every metric value the log holds, as (sequence, name): what the index must hold, once each.
    fn logged_metrics(records: &[EventRecord]) -> BTreeSet<(u64, String)> {
        let mut metrics = BTreeSet::new();
        for record in records.iter().filter(|record| record.event_type == RUN_METRICS) {
            if let Some(Payload::RunMetrics(values)) =
                record.payload.as_ref().and_then(|payload| payload.payload.as_ref())
            {
                for name in values.values.keys() {
                    metrics.insert((record.sequence, name.clone()));
                }
            }
        }
        metrics
    }

    /// The index holds exactly the log: every metric once, and the ack at the log's last record.
    fn check_index_matches_log(running: &RunningDaemon, paths: &DataPaths, run_id: &str) {
        let records = log_records(paths, run_id);
        let expected = logged_metrics(&records);
        assert!(!expected.is_empty());
        assert_eq!(metric_rows(running, run_id), expected);
        let last = records.last().unwrap().sequence;
        assert_eq!(run_row(running, run_id).acked_sequence, last);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-daemon-jobs runs it"]
    async fn train_start_trains_textcnn_to_success_with_every_metric_indexed() {
        let paths = test_paths("tj-success");
        let running = start_daemon_with_cpu_environment(&paths).await;
        let (_, run_id, job_id) = start_training(&running).await;
        assert!(job_id.starts_with("job_"), "{job_id}");
        // The answer came before the job could have ended: it is queued or starting.
        let answered = job_row(&running, &job_id).state;
        assert!(
            ["queued", "starting", "running"].contains(&answered.as_str()),
            "{answered}"
        );

        wait_for_end(&running, &paths, &job_id, &run_id, ("succeeded", "completed")).await;
        check_index_matches_log(&running, &paths, &run_id);
        let job = job_row(&running, &job_id);
        assert_eq!(job.exit_code, Some(0));
        let listed = ok_result(
            &run_command(
                &running.server,
                &owner_principal(),
                execute_request("job.list", &json!({"run_id": run_id}).to_string()),
            )
            .await,
        );
        assert_eq!(listed["jobs"][0]["state"], "succeeded");
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-daemon-jobs runs it"]
    async fn job_cancel_ends_the_train_job_and_its_run_cancelled() {
        let paths = test_paths("tj-cancel");
        let running = start_daemon_with_cpu_environment(&paths).await;
        let (_, run_id, job_id) = start_training(&running).await;
        wait_for_first_metric(&running, &paths, &run_id).await;

        // contracts 6.4: a token with only `read` is refused (it couldn't have started the job) and the job runs on.
        let cancel = execute_request("job.cancel", &json!({"job_id": job_id}).to_string());
        let reader_only = run_command(&running.server, &token_principal(vec![Scope::Read]), cancel.clone()).await;
        assert_eq!(response_codes(&reader_only), ["RRS-C1005"]);
        let metrics_seen = metric_rows(&running, &run_id).len();
        wait_for_more_metrics(&running, &paths, &run_id, metrics_seen).await;
        assert_eq!(job_row(&running, &job_id).state, "running");

        let answered: Value = ok_result(&run_command(&running.server, &owner_principal(), cancel).await);
        assert_eq!(answered["job"]["state"], "stopping");
        wait_for_end(&running, &paths, &job_id, &run_id, ("cancelled", "cancelled")).await;
        let job = job_row(&running, &job_id);
        assert_eq!(
            job.exit_code.map(|code| code.to_string()).as_deref(),
            Some(EXIT_CANCELLED)
        );
        check_index_matches_log(&running, &paths, &run_id);
        stop_test_daemon(running).await;
        remove_test_folder(&paths);
    }

    // flows/platform.md 2.3: the daemon stops mid-run (the embedded node with it); the worker keeps training. The
    // next daemon reconciles through ListJobs, the run's tail goes on after the ack the first one stored, and every
    // metric ends up indexed exactly once.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-daemon-jobs runs it"]
    async fn a_daemon_restart_mid_run_reconciles_and_the_tail_resumes_from_the_ack() {
        let paths = test_paths("tj-restart");
        let running = start_daemon_with_cpu_environment(&paths).await;
        let (_, run_id, job_id) = start_training(&running).await;
        wait_for_first_metric(&running, &paths, &run_id).await;
        let acked_before = run_row(&running, &run_id).acked_sequence;
        assert!(acked_before > 0);
        stop_test_daemon(running).await;

        let restarted = start_test_daemon(&paths).await;
        // start_daemon returns after ListJobs: the job is known to the new node, so it hasn't been failed as gone.
        assert_ne!(job_row(&restarted, &job_id).error_code, "RRS-J3002");
        assert!(run_row(&restarted, &run_id).acked_sequence >= acked_before);
        wait_for_end(&restarted, &paths, &job_id, &run_id, ("succeeded", "completed")).await;
        check_index_matches_log(&restarted, &paths, &run_id);
        stop_test_daemon(restarted).await;
        remove_test_folder(&paths);
    }
}
