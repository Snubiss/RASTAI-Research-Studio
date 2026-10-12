//! The TextCNN train job (rastai_worker's `jobs/train.py`, plan M1-46) under the real node, end to end on the CPU:
//! `NodeService.StartJob` spawns `python -m rastai_worker run --job-file <job.json>` in the cpu environment, the
//! worker registers, the node calls Hello, and the job trains on the fixture dataset and ends `succeeded`; a
//! `TERMINATE` preempts it with a checkpoint, exit 75 (contracts 4.4 and 9.1). The job's `JobSpec` fields come from
//! `fixtures/jobs/textcnn_train/job.json`, the launch file the worker's own tests run.
//!
//! It runs the real worker in the cpu environment's Python (`just env-cpu`), which a plain `cargo test` on a CI
//! runner doesn't have, so its tests are `#[ignore]`d and `just test-node-jobs` runs them (`just check` includes it).
//! Linux only, like the node's other job tests: the node reads /proc and signals process groups.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod on_linux {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use rastai_core::ids::{IdPrefix, new_id};
    use rastai_core::paths::{DataPaths, run_dir, run_events_log_path};
    use rastai_node::error::NodeError;
    use rastai_node::jobs::supervisor::{JobEnvironment, JobOptions};
    use rastai_proto::common::v1::{JobState, RunState};
    use rastai_proto::events::v1::EventRecord;
    use rastai_proto::events::v1::event_payload::Payload;
    use rastai_proto::events::v1::run_state_changed::State;
    use rastai_proto::node::v1::node_service_client::NodeServiceClient;
    use rastai_proto::node::v1::{JobInfo, JobKind, JobSignal, JobSpec, ListJobsRequest, SignalJobRequest};
    use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
    use serde_json::Value;
    use tokio::task::JoinHandle;
    use tonic::transport::Channel;

    use crate::common::{NvidiaSmi, TestFolder, machine_from_fixture, new_test_folder, start_node, test_node_options};
    use crate::common::{test_paths, write_tool};

    const TEST_FOLDER_PREFIX: &str = "rntj";
    const HARDWARE_FIXTURE: &str = "no_gpu";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    const ENVIRONMENT_ID: &str = "cpu";
    /// The node's own defaults would do; a test only needs them longer than any wait below.
    const REGISTER_TIMEOUT: Duration = Duration::from_secs(60);
    const STOP_GRACE: Duration = Duration::from_secs(60);
    /// Starting Python and torch takes a few seconds; training the fixture's 40 steps about one more.
    const WAIT_LIMIT: Duration = Duration::from_secs(120);
    const POLL: Duration = Duration::from_millis(50);
    /// The cpu environment's interpreter and the train job's launch file, relative to this crate.
    const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
    const REPOSITORY_ROOT: &str = "../..";
    const TRAIN_JOB_FIXTURE: &str = "fixtures/jobs/textcnn_train/job.json";
    /// The `TrainStartArgs` paths in the fixture, relative to the repository root.
    const START_PATH_FIELDS: &[&str] = &["design_path", "dataset_path"];
    const FIXTURE_MAX_STEPS: u64 = 40;
    /// Long enough that the job is still training when TERMINATE comes.
    const LONG_RUN_STEPS: u64 = 100_000;
    /// The worker's exit after SIGTERM (contracts 9.1).
    const EXIT_PREEMPTED: i32 = 75;
    const RUN_STATE_CHANGED: &str = "run.state_changed";
    const RUN_METRICS: &str = "run.metrics";
    const CHECKPOINT_SAVED: &str = "checkpoint.saved";

    /// A node whose `cpu` environment is the repository's cpu environment, and a client for it.
    struct TrainNode {
        /// Kept for its Drop, which removes the test's folder.
        _folder: TestFolder,
        paths: DataPaths,
        client: NodeServiceClient<Channel>,
        node: JoinHandle<Result<(), NodeError>>,
    }

    impl Drop for TrainNode {
        fn drop(&mut self) {
            self.node.abort();
        }
    }

    /// The environment's `bin/python` runs the cpu environment's interpreter with the node's arguments
    /// (`-m rastai_worker run --job-file <path>`), so the job runs the real worker.
    async fn start_train_node() -> TrainNode {
        let python = Path::new(env!("CARGO_MANIFEST_DIR")).join(CPU_PYTHON);
        assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let environment_folder = folder.path.join("env");
        fs::create_dir_all(environment_folder.join("bin")).unwrap();
        let script = format!("#!/bin/sh\nexec '{}' \"$@\"\n", python.display());
        write_tool(&environment_folder.join("bin/python"), &script);
        let sources = machine_from_fixture(HARDWARE_FIXTURE, &folder.path, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let mut options = test_node_options(&folder.path, sources);
        options.jobs = JobOptions {
            worker_register_timeout: REGISTER_TIMEOUT,
            stop_grace: STOP_GRACE,
            environments: vec![JobEnvironment {
                environment_id: ENVIRONMENT_ID.to_string(),
                folder: environment_folder,
            }],
        };
        let (node, client) = start_node(options).await;
        TrainNode {
            paths: test_paths(&folder.path),
            _folder: folder,
            client,
            node,
        }
    }

    /// The fixture's train `JobSpec` for a new job of `run_id`: its two JSON fields as the daemon writes them, with
    /// the design and dataset paths made absolute, and `max_steps` changed when one is given.
    fn train_job_spec(run_id: &str, max_steps: Option<u64>) -> JobSpec {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join(REPOSITORY_ROOT);
        let fixture_text = fs::read_to_string(repository_root.join(TRAIN_JOB_FIXTURE)).unwrap();
        let fixture: Value = serde_json::from_str(&fixture_text).unwrap();
        let fixture_spec = &fixture["job_spec"];
        let mut arguments: Value = serde_json::from_str(fixture_spec["arguments_json"].as_str().unwrap()).unwrap();
        for name in START_PATH_FIELDS {
            let relative = arguments["start_arguments"][*name].as_str().unwrap().to_string();
            let absolute = repository_root.join(relative).display().to_string();
            arguments["start_arguments"][*name] = Value::String(absolute);
        }
        if let Some(steps) = max_steps {
            arguments["max_steps"] = Value::from(steps);
        }
        JobSpec {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: run_id.to_string(),
            kind: JobKind::Train as i32,
            environment_id: ENVIRONMENT_ID.to_string(),
            arguments_json: arguments.to_string(),
            checkpoint_policy_json: fixture_spec["checkpoint_policy_json"].as_str().unwrap().to_string(),
            data_policy: fixture_spec["data_policy"].as_str().unwrap().to_string(),
            ..JobSpec::default()
        }
    }

    async fn list_job(node: &mut TrainNode, job_id: &str) -> JobInfo {
        let jobs = node
            .client
            .list_jobs(ListJobsRequest {})
            .await
            .unwrap()
            .into_inner()
            .jobs;
        jobs.into_iter()
            .find(|info| info.job_id == job_id)
            .expect("the job is listed")
    }

    fn is_ended(info: &JobInfo) -> bool {
        matches!(
            info.state(),
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }

    /// Polls ListJobs until the job has ended.
    async fn wait_for_end(node: &mut TrainNode, job_id: &str, run_id: &str) -> JobInfo {
        let started = Instant::now();
        loop {
            let info = list_job(node, job_id).await;
            if is_ended(&info) {
                return info;
            }
            assert!(
                started.elapsed() < WAIT_LIMIT,
                "job didn't end: {info:?}\nits stderr.log:\n{}",
                stderr_log(node, run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Polls the run's log until the job has written a record of `event_type`, while the job is live.
    async fn wait_for_record(node: &mut TrainNode, job_id: &str, run_id: &str, event_type: &str) {
        let log_path = run_events_log_path(&node.paths, run_id);
        let started = Instant::now();
        loop {
            if log_path.exists()
                && records_in(&log_path)
                    .iter()
                    .any(|record| record.event_type == event_type)
            {
                return;
            }
            let info = list_job(node, job_id).await;
            let gave_up = is_ended(&info) || started.elapsed() > WAIT_LIMIT;
            assert!(
                !gave_up,
                "no {event_type} record came: {info:?}\nits stderr.log:\n{}",
                stderr_log(node, run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// The log's whole records, read by rastai-store's reader as the node's tail reads them.
    fn records_in(path: &Path) -> Vec<EventRecord> {
        let mut reader = open_event_log(path).unwrap();
        let mut records = Vec::new();
        while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
            records.push(record);
        }
        records
    }

    fn stderr_log(node: &TrainNode, run_id: &str) -> String {
        fs::read_to_string(run_dir(&node.paths, run_id).join("stderr.log")).unwrap_or_default()
    }

    /// The run states of the log's `run.state_changed` records, in order.
    fn run_states(records: &[EventRecord]) -> Vec<RunState> {
        let mut states = Vec::new();
        for record in records {
            let payload = record.payload.as_ref().and_then(|payload| payload.payload.as_ref());
            if let Some(Payload::RunStateChanged(changed)) = payload
                && let Some(State::RunState(value)) = changed.state
            {
                states.push(RunState::try_from(value).unwrap());
            }
        }
        states
    }

    fn saved_checkpoint_steps(records: &[EventRecord]) -> Vec<u64> {
        records
            .iter()
            .filter_map(|record| match record.payload.as_ref()?.payload.as_ref()? {
                Payload::CheckpointSaved(saved) => Some(saved.step),
                _ => None,
            })
            .collect()
    }

    fn event_types(records: &[EventRecord]) -> Vec<&str> {
        records.iter().map(|record| record.event_type.as_str()).collect()
    }

    /// Every file and folder under `folder` whose name ends with `suffix`.
    fn paths_ending_with(folder: &Path, suffix: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut folders = vec![folder.to_path_buf()];
        while let Some(current) = folders.pop() {
            for entry in fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                if path.to_string_lossy().ends_with(suffix) {
                    found.push(path.clone());
                }
                if path.is_dir() {
                    folders.push(path);
                }
            }
        }
        found
    }

    /// One job's records, numbered from 1 with no gap, all of this job and run.
    fn check_one_job_in_sequence(records: &[EventRecord], job_id: &str, run_id: &str) {
        let sequences: Vec<u64> = records.iter().map(|record| record.sequence).collect();
        let expected: Vec<u64> = (1..=records.len() as u64).collect();
        assert_eq!(sequences, expected);
        assert!(
            records
                .iter()
                .all(|record| record.job_id == job_id && record.run_id == run_id)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn the_node_runs_the_textcnn_train_job_to_success() {
        let mut node = start_train_node().await;
        let run_id = new_id(IdPrefix::Run).unwrap();
        let spec = train_job_spec(&run_id, None);
        node.client.start_job(spec.clone()).await.unwrap();
        let ended = wait_for_end(&mut node, &spec.job_id, &run_id).await;
        assert_eq!(
            ended.state(),
            JobState::Succeeded,
            "{ended:?}\n{}",
            stderr_log(&node, &run_id)
        );
        assert_eq!(ended.exit_code, Some(0));

        let records = records_in(&run_events_log_path(&node.paths, &run_id));
        check_one_job_in_sequence(&records, &spec.job_id, &run_id);
        assert_eq!(
            run_states(&records),
            vec![RunState::Preparing, RunState::Running, RunState::Completed]
        );
        let types = event_types(&records);
        assert_eq!(types[..2], [RUN_STATE_CHANGED, RUN_STATE_CHANGED]);
        assert!(types.contains(&RUN_METRICS));
        assert_eq!(saved_checkpoint_steps(&records), vec![FIXTURE_MAX_STEPS]);
        let saved_at = types.iter().position(|kind| *kind == CHECKPOINT_SAVED).unwrap();
        assert!(
            saved_at < records.len() - 1,
            "the checkpoint comes before the final state"
        );
        let run_folder = run_dir(&node.paths, &run_id);
        assert!(
            paths_ending_with(&run_folder, ".pt").is_empty(),
            "no pickle in the run folder"
        );
        assert_eq!(paths_ending_with(&run_folder, "weights.safetensors").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn terminate_preempts_the_train_job_with_a_checkpoint() {
        let mut node = start_train_node().await;
        let run_id = new_id(IdPrefix::Run).unwrap();
        let spec = train_job_spec(&run_id, Some(LONG_RUN_STEPS));
        node.client.start_job(spec.clone()).await.unwrap();
        wait_for_record(&mut node, &spec.job_id, &run_id, RUN_METRICS).await;
        let request = SignalJobRequest {
            job_id: spec.job_id.clone(),
            signal: JobSignal::Terminate as i32,
        };
        let response = node.client.signal_job(request).await.unwrap().into_inner();
        assert_eq!(response.diagnostic, None);
        let ended = wait_for_end(&mut node, &spec.job_id, &run_id).await;
        // SIGTERM is preemption: the job ends cancelled with the worker's exit 75, and the run is resumable.
        assert_eq!(
            ended.state(),
            JobState::Cancelled,
            "{ended:?}\n{}",
            stderr_log(&node, &run_id)
        );
        assert_eq!(ended.exit_code, Some(EXIT_PREEMPTED));

        let records = records_in(&run_events_log_path(&node.paths, &run_id));
        check_one_job_in_sequence(&records, &spec.job_id, &run_id);
        assert_eq!(run_states(&records).last(), Some(&RunState::Preempted));
        assert_eq!(records[records.len() - 2].event_type, CHECKPOINT_SAVED);
        let steps = saved_checkpoint_steps(&records);
        assert_eq!(steps.len(), 1);
        assert!(steps[0] > 0 && steps[0] < LONG_RUN_STEPS);
        assert!(paths_ending_with(&run_dir(&node.paths, &run_id), ".partial").is_empty());
    }
}
