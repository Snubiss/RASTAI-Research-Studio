//! Reattach after a node restart, and immediate heartbeats (layout: rust-node-gateway-proto.md 7.4 "Reattach", 7.5
//! and 7.9; platform flows 2.3 and 6.5 row 14; contracts 9.1 and 9.2).
//!
//! - **kill -9 of the node mid-run.** The node runs as a process of its own: this test binary started again with
//!   `RASTAI_TEST_NODE_FOLDER` set, which runs only `node_process_for_kill_tests`, a node in that folder. The test
//!   SIGKILLs it while a job runs, starts it again, and checks the job was taken back: `lost` with its pid, then its
//!   end from its log once its process is gone, and every record the job wrote while no node was there streamed
//!   exactly once after the daemon's ack.
//! - **A `job.json` that names a process that isn't the job's** (other start ticks, an earlier boot, a pid that
//!   doesn't lead its group) ends `failed` at once, and that process is never signalled.
//! - **A gone job's end comes from its log**, except after exit 5: a job that never opened the log, whose
//!   `stderr.log` ends with the `RRS-J3006` refusal, fails with it. `stderr.log` is the run's, so the same line left
//!   by an earlier job of the run doesn't change the end of a job that wrote its own records.
//! - **A live job whose worker never registers again** is killed with its group and fails with `RRS-J3003`; a
//!   `Register` with a wrong token is refused with `RRS-J2002` and the job stays `lost`.
//! - **Heartbeats**: one at once on a handshake and on every job state change, besides the regular beat.
//!
//! Most tests use shell stand-ins for the worker. The one where the worker registers again after the restart and
//! `Hello` succeeds runs `tests/support/fake_worker.py` in the cpu environment, so it is `#[ignore]`d in a plain
//! `cargo test` and `just test-node-jobs` runs it, as for jobs.rs.
//!
//! Linux only: the node reads /proc for a process's identity and signals groups with procps' `kill`.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod on_linux {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use rastai_core::ids::{IdPrefix, new_id};
    use rastai_core::paths::{
        DataPaths, job_socket_path, node_hub_socket_path, node_socket_path, run_dir, run_events_log_path,
        worker_host_socket_path,
    };
    use rastai_node::jobs::job_file::{JobFile, job_file_path, read_job_file, write_job_file};
    use rastai_node::jobs::signals::{ProcessIdentity, read_process_identity};
    use rastai_node::jobs::supervisor::{JobEnvironment, JobOptions};
    use rastai_node::node::{NodeOptions, run_node};
    use rastai_node::worker_host_service::request_with_job_token;
    use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, EventClass, JobState, RunState};
    use rastai_proto::events::v1::event_payload::Payload;
    use rastai_proto::events::v1::run_state_changed::State;
    use rastai_proto::events::v1::{EventPayload, EventRecord, RunMetrics, RunStateChanged};
    use rastai_proto::node::v1::node_service_client::NodeServiceClient;
    use rastai_proto::node::v1::{
        HeartbeatRequest, JobInfo, JobKind, JobSignal, JobSpec, ListJobsRequest, SignalJobRequest,
        WatchRunEventsRequest,
    };
    use rastai_proto::worker::v1::RegisterRequest;
    use rastai_proto::worker::v1::worker_host_service_client::WorkerHostServiceClient;
    use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
    use rastai_store::event_log::writer::{EventLogWriter, SyncPolicy, append_event, create_event_log};
    use rastai_transport::grpc_channel::connect_grpc_channel;
    use tonic::Streaming;
    use tonic::transport::Channel;

    use crate::common::{
        NvidiaSmi, kill_job_groups_under, machine_from_fixture, new_test_folder, paired_daemon_handshake,
        start_fake_hub, start_node, test_node_options, test_paths, write_tool,
    };

    const TEST_FOLDER_PREFIX: &str = "rnra";
    const HARDWARE_FIXTURE: &str = "no_gpu";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    const ENVIRONMENT_ID: &str = "cpu";
    /// The variable that makes this binary a node process for the kill -9 tests, naming its test folder (contracts
    /// 15.9, tests only), and the one test such a process runs.
    const NODE_FOLDER_VARIABLE: &str = "RASTAI_TEST_NODE_FOLDER";
    const NODE_PROCESS_TEST: &str = "on_linux::node_process_for_kill_tests";
    /// Where a node process writes what it prints, in the test folder; a failing test shows it.
    const NODE_OUTPUT_FILE: &str = "node-output.txt";
    /// Long enough that a test never meets it unless the test is about it.
    const LONG_TIMEOUT: Duration = Duration::from_secs(60);
    /// The reconnect timeout of the tests about a worker that never comes back.
    const SHORT_RECONNECT_TIMEOUT: Duration = Duration::from_secs(1);
    /// How long a test waits for a job, a file or a node to get where it should.
    const WAIT_LIMIT: Duration = Duration::from_secs(30);
    const POLL: Duration = Duration::from_millis(20);
    /// How soon a heartbeat must follow the change that asked for it. The regular beat is an hour away in those
    /// tests, so a beat this soon can only be the immediate one.
    const IMMEDIATE_BEAT: Duration = Duration::from_secs(2);
    const HOUR_SECONDS: u64 = 3600;
    /// The node polls a reattached job's process once a second; a check that nothing was signalled waits longer.
    const AFTER_SEVERAL_POLLS: Duration = Duration::from_millis(2500);
    const READY_FILE: &str = "ready.txt";
    const FINISH_FILE: &str = "finish";
    const CHILD_PID_FILE: &str = "child.pid";
    const FAKE_WORKER_RECORD: &str = "fake-worker-record.jsonl";
    const FAKE_WORKER: &str = "tests/support/fake_worker.py";
    const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
    const SCRIPT_FILE: &str = "script.json";
    /// The line the worker prints to stderr.log before exit 5 (python diagnostics.py).
    const EVENT_LOG_LOCKED_LINE: &str = "RRS-J3006: Another process is writing the event log, so this job stopped.";
    const STDERR_LOG: &str = "stderr.log";

    /// Notes it is ready, then runs until the test creates `finish` in its run folder, and exits 0. `$5` is the job
    /// file (`-m rastai_worker run --job-file <path>`). It never registers, so its job stays `starting`, or `lost`
    /// after a node restart.
    const WAITING_WORKER: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
echo ready > "$run_folder/ready.txt"
while [ ! -e "$run_folder/finish" ]; do sleep 0.05; done
exit 0
"#;
    /// A group leader with a child in its group, standing in for a worker from before a restart. `$1` is the folder
    /// where it notes its child's pid.
    const LEADER_WITH_CHILD: &str = r#"sleep 600 &
echo $! > "$1/child.pid"
exec sleep 600
"#;

    // ---- The node as a process of its own ----

    /// Not a test of its own: the kill -9 tests start this binary again to run only this, with
    /// `RASTAI_TEST_NODE_FOLDER` naming the folder, so the node is a process they can SIGKILL. Without the variable
    /// it returns at once.
    #[test]
    fn node_process_for_kill_tests() {
        let Ok(folder) = std::env::var(NODE_FOLDER_VARIABLE) else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let stopped = runtime.block_on(run_node(node_options_in(Path::new(&folder), LONG_TIMEOUT)));
        panic!("the node stopped: {stopped:?}");
    }

    /// The node of a test folder: the recorded machine without a GPU, and one `cpu` environment, whose `bin/python`
    /// the test writes.
    fn node_options_in(folder: &Path, host_reconnect_timeout: Duration) -> NodeOptions {
        let sources = machine_from_fixture(HARDWARE_FIXTURE, folder, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let mut options = test_node_options(folder, sources);
        options.jobs = JobOptions {
            worker_register_timeout: LONG_TIMEOUT,
            stop_grace: LONG_TIMEOUT,
            host_reconnect_timeout,
            environments: vec![JobEnvironment {
                environment_id: ENVIRONMENT_ID.to_string(),
                folder: environment_folder(folder),
            }],
        };
        options
    }

    fn environment_folder(folder: &Path) -> PathBuf {
        folder.join("env")
    }

    /// Makes the `cpu` environment's `bin/python` run `script`.
    fn install_worker(folder: &Path, script: &str) {
        let environment = environment_folder(folder);
        fs::create_dir_all(environment.join("bin")).unwrap();
        write_tool(&environment.join("bin/python"), script);
    }

    /// A node process and a client of its node.sock. It is SIGKILLed when dropped, and so are the workers it started
    /// in `folder`, unless the test killed it on purpose (`kill_9`).
    struct NodeProcess {
        process: Child,
        client: NodeServiceClient<Channel>,
        folder: PathBuf,
        /// Set by `kill_9`: the workers it leaves are the next node's to reattach, so they must outlive this drop.
        workers_outlive_it: bool,
    }

    impl Drop for NodeProcess {
        fn drop(&mut self) {
            let _ = self.process.kill();
            let _ = self.process.wait();
            if !self.workers_outlive_it {
                kill_job_groups_under(&self.folder);
            }
        }
    }

    impl NodeProcess {
        /// kill -9, then waits until the process is gone.
        fn kill_9(mut self) {
            self.process.kill().unwrap();
            self.process.wait().unwrap();
            self.workers_outlive_it = true;
        }
    }

    /// Starts a node process in `folder` and returns once its node.sock answers.
    async fn start_node_process(folder: &Path) -> NodeProcess {
        let output = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join(NODE_OUTPUT_FILE))
            .unwrap();
        let mut process = Command::new(std::env::current_exe().unwrap())
            .args([NODE_PROCESS_TEST, "--exact", "--nocapture"])
            .env(NODE_FOLDER_VARIABLE, folder)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap();
        let socket_path = node_socket_path(&test_paths(folder));
        let started = Instant::now();
        loop {
            if let Ok(channel) = connect_grpc_channel(&socket_path).await {
                let client = NodeServiceClient::new(channel);
                return NodeProcess {
                    process,
                    client,
                    folder: folder.to_path_buf(),
                    workers_outlive_it: false,
                };
            }
            let exited = process.try_wait().unwrap();
            let output = fs::read_to_string(folder.join(NODE_OUTPUT_FILE)).unwrap_or_default();
            if exited.is_some() || started.elapsed() >= WAIT_LIMIT {
                // The failing assertions below panic before a NodeProcess exists, so the start's own process and
                // any worker it started are stopped here, not left for the next test.
                let _ = process.kill();
                let _ = process.wait();
                kill_job_groups_under(folder);
            }
            assert!(exited.is_none(), "the node process stopped: {exited:?}\n{output}");
            assert!(started.elapsed() < WAIT_LIMIT, "node.sock didn't answer\n{output}");
            tokio::time::sleep(POLL).await;
        }
    }

    // ---- Processes ----

    /// Kills a whole process group when dropped, so a failed test leaves no worker running.
    struct GroupGuard {
        process_group_id: u32,
    }

    impl Drop for GroupGuard {
        fn drop(&mut self) {
            let group = format!("-{}", self.process_group_id);
            let _ = Command::new("kill").args(["-s", "KILL", "--", &group]).output();
        }
    }

    /// `script` run by sh as the leader of a new process group, with `folder` as `$1`.
    fn spawn_group_leader(script: &str, folder: &Path) -> Child {
        Command::new("sh")
            .args(["-c", script, "stand-in"])
            .arg(folder)
            .process_group(0)
            .spawn()
            .unwrap()
    }

    /// The identity of a process the test started, once it runs what it was started for.
    fn identity_of(pid: u32) -> ProcessIdentity {
        read_process_identity(pid).expect("the process runs")
    }

    /// Listed in /proc and not a zombie (field 3 of `/proc/<pid>/stat` is `Z` for one that exited and that nobody
    /// has reaped yet).
    fn is_running(pid: u32) -> bool {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let is_zombie = stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'));
        !is_zombie
    }

    /// Still running and still the same process: never signalled.
    fn still_runs_as(identity: &ProcessIdentity) -> bool {
        is_running(identity.pid) && read_process_identity(identity.pid).as_ref() == Some(identity)
    }

    /// Gone, or a zombie nobody has reaped yet: either way it no longer runs.
    async fn wait_until_stopped(pid: u32) {
        let started = Instant::now();
        loop {
            if !is_running(pid) {
                return;
            }
            assert!(started.elapsed() < WAIT_LIMIT, "process {pid} still runs");
            tokio::time::sleep(POLL).await;
        }
    }

    async fn wait_for_file(path: &Path) -> String {
        let started = Instant::now();
        loop {
            match fs::read_to_string(path) {
                Ok(text) if !text.trim().is_empty() => return text,
                _ => {}
            }
            assert!(started.elapsed() < WAIT_LIMIT, "{} didn't appear", path.display());
            tokio::time::sleep(POLL).await;
        }
    }

    // ---- Jobs, job.json and the run's log ----

    fn new_run_id() -> String {
        new_id(IdPrefix::Run).unwrap()
    }

    fn job_spec(run_id: &str) -> JobSpec {
        JobSpec {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: run_id.to_string(),
            kind: JobKind::Train as i32,
            environment_id: ENVIRONMENT_ID.to_string(),
            ..JobSpec::default()
        }
    }

    /// The `job.json` a node wrote for a job of `run_id` whose worker is `process`, as it is while the job runs.
    fn write_live_job_file(paths: &DataPaths, run_id: &str, process: &ProcessIdentity) -> JobFile {
        let job_file = JobFile {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: run_id.to_string(),
            kind: "train".to_string(),
            state: "running".to_string(),
            pid: process.pid,
            process_group_id: process.process_group_id,
            process_start_ticks: process.start_ticks,
            boot_id: process.boot_id.clone(),
            job_token_hash: format!("blake3:{}", "ab".repeat(32)),
            ..JobFile::default()
        };
        let run_folder = run_dir(paths, run_id);
        fs::create_dir_all(&run_folder).unwrap();
        write_job_file(&run_folder, &job_file).unwrap();
        job_file
    }

    async fn list_job(client: &mut NodeServiceClient<Channel>, job_id: &str) -> JobInfo {
        let jobs = client.list_jobs(ListJobsRequest {}).await.unwrap().into_inner().jobs;
        jobs.into_iter()
            .find(|info| info.job_id == job_id)
            .expect("the job is listed")
    }

    async fn wait_for_state(client: &mut NodeServiceClient<Channel>, job_id: &str, wanted: JobState) -> JobInfo {
        let started = Instant::now();
        loop {
            let info = list_job(client, job_id).await;
            if info.state() == wanted {
                return info;
            }
            assert!(
                started.elapsed() < WAIT_LIMIT,
                "job is {:?}, waiting for {wanted:?}: {info:?}",
                info.state()
            );
            tokio::time::sleep(POLL).await;
        }
    }

    async fn signal(client: &mut NodeServiceClient<Channel>, job_id: &str, signal: JobSignal) {
        let request = SignalJobRequest {
            job_id: job_id.to_string(),
            signal: signal as i32,
        };
        let refusal = client.signal_job(request).await.unwrap().into_inner().diagnostic;
        assert_eq!(refusal, None);
    }

    fn error_code(info: &JobInfo) -> String {
        info.last_error
            .as_ref()
            .map(|found| found.code.clone())
            .unwrap_or_default()
    }

    fn final_state_on_disk(paths: &DataPaths, run_id: &str) -> String {
        read_job_file(&job_file_path(&run_dir(paths, run_id)))
            .unwrap()
            .final_state
    }

    /// The terminal `job.json` is the one the node read back, with only its state and final state changed: the pid,
    /// boot id, start ticks and hardware the job started on stay.
    fn assert_terminal_file_keeps(paths: &DataPaths, written: &JobFile, final_state: &str) {
        let on_disk = read_job_file(&job_file_path(&run_dir(paths, &written.run_id))).unwrap();
        let expected = JobFile {
            state: final_state.to_string(),
            final_state: final_state.to_string(),
            ..written.clone()
        };
        assert_eq!(on_disk, expected);
    }

    /// The run's `stderr.log` as an earlier job left it: refused the run's log (exit 5).
    fn write_earlier_refusal(paths: &DataPaths, run_id: &str) {
        let run_folder = run_dir(paths, run_id);
        fs::create_dir_all(&run_folder).unwrap();
        fs::write(run_folder.join(STDERR_LOG), format!("{EVENT_LOG_LOCKED_LINE}\n")).unwrap();
    }

    /// Calls `Register` on worker-host.sock for `job_id` with `job_token`, and returns the node's refusal, if any.
    async fn register_with_token(paths: &DataPaths, job_id: &str, job_token: &str) -> Option<ProtoDiagnostic> {
        let channel = connect_grpc_channel(&worker_host_socket_path(paths)).await.unwrap();
        let mut client = WorkerHostServiceClient::new(channel);
        let message = RegisterRequest {
            job_id: job_id.to_string(),
            job_socket_path: job_socket_path(paths, job_id).display().to_string(),
        };
        let request = request_with_job_token(message, job_token).unwrap();
        client.register(request).await.unwrap().into_inner().diagnostic
    }

    /// A new events.log for the run, as a worker creates it.
    fn new_run_log(paths: &DataPaths, run_id: &str) -> EventLogWriter {
        fs::create_dir_all(run_dir(paths, run_id)).unwrap();
        create_event_log(&run_events_log_path(paths, run_id)).unwrap()
    }

    fn append_record(writer: &mut EventLogWriter, run_id: &str, job_id: &str, event_type: &str, payload: Payload) {
        let mut record = EventRecord {
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            event_type: event_type.to_string(),
            event_class: EventClass::Durable as i32,
            payload: Some(EventPayload { payload: Some(payload) }),
            ..EventRecord::default()
        };
        append_event(writer, &mut record, SyncPolicy::Batched).unwrap();
    }

    /// Metric records of `job_id` until the log's last sequence is `last_sequence`.
    fn append_metrics(writer: &mut EventLogWriter, run_id: &str, job_id: &str, first: u64, last_sequence: u64) {
        for step in first..=last_sequence {
            let mut values = BTreeMap::new();
            values.insert("train.loss".to_string(), 1.0 / (step as f64 + 1.0));
            let metrics = Payload::RunMetrics(RunMetrics { step, values });
            append_record(writer, run_id, job_id, "run.metrics", metrics);
        }
    }

    /// The job's final `run.state_changed`, as its worker writes it.
    fn append_run_state(writer: &mut EventLogWriter, run_id: &str, job_id: &str, state: RunState) {
        let change = RunStateChanged {
            state: Some(State::RunState(state as i32)),
            ..RunStateChanged::default()
        };
        append_record(
            writer,
            run_id,
            job_id,
            "run.state_changed",
            Payload::RunStateChanged(change),
        );
    }

    fn records_in(path: &Path) -> Vec<EventRecord> {
        let mut reader = open_event_log(path).unwrap();
        let mut records = Vec::new();
        while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
            records.push(record);
        }
        records
    }

    async fn watch(client: &mut NodeServiceClient<Channel>, run_id: &str, after: u64) -> Streaming<EventRecord> {
        let request = WatchRunEventsRequest {
            run_id: run_id.to_string(),
            after_sequence: after,
        };
        client.watch_run_events(request).await.unwrap().into_inner()
    }

    async fn next_record(stream: &mut Streaming<EventRecord>) -> Option<EventRecord> {
        let message = tokio::time::timeout(WAIT_LIMIT, stream.message()).await;
        message.expect("the stream neither sent nor ended").unwrap()
    }

    async fn take_records(stream: &mut Streaming<EventRecord>, count: usize) -> Vec<EventRecord> {
        let mut records = Vec::new();
        for _ in 0..count {
            records.push(next_record(stream).await.expect("the stream ended early"));
        }
        records
    }

    async fn read_to_end(stream: &mut Streaming<EventRecord>) -> Vec<EventRecord> {
        let mut records = Vec::new();
        while let Some(record) = next_record(stream).await {
            records.push(record);
        }
        records
    }

    fn sequences(records: &[EventRecord]) -> Vec<u64> {
        records.iter().map(|record| record.sequence).collect()
    }

    // ---- kill -9 of the node ----

    // Platform flow 6.5 row 14 with a worker that never registers: the node is SIGKILLed while the job runs, the
    // worker keeps running and writing its log, and the restarted node takes the job back as `lost` with its pid.
    // When the worker then finishes, the node sees its process gone and takes the job's end from its log, and the
    // daemon's watch gets every record exactly once. An earlier job of the run was refused the log, and its
    // RRS-J3006 line is still the last of the run's stderr.log: it isn't this job's, which wrote its own records.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_node_killed_mid_run_takes_the_running_job_back_and_its_end_from_the_log() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        install_worker(&folder.path, WAITING_WORKER);
        let mut node = start_node_process(&folder.path).await;
        let run_id = new_run_id();
        write_earlier_refusal(&paths, &run_id);
        let spec = job_spec(&run_id);
        let job_id = spec.job_id.clone();
        let started = node.client.start_job(spec).await.unwrap().into_inner();
        assert_eq!(started.state(), JobState::Starting, "{started:?}");
        let _worker_group = GroupGuard {
            process_group_id: started.process_group_id,
        };
        wait_for_file(&run_dir(&paths, &run_id).join(READY_FILE)).await;
        let worker = identity_of(started.pid);

        node.kill_9();
        assert!(still_runs_as(&worker), "the worker outlives the node");
        let mut writer = new_run_log(&paths, &run_id);
        append_metrics(&mut writer, &run_id, &job_id, 1, 40);

        let mut node = start_node_process(&folder.path).await;
        let lost = list_job(&mut node.client, &job_id).await;
        assert_eq!(lost.state(), JobState::Lost, "{lost:?}");
        assert_eq!((lost.pid, lost.process_group_id), (worker.pid, worker.process_group_id));
        let mut stream = watch(&mut node.client, &run_id, 10).await;
        assert_eq!(
            sequences(&take_records(&mut stream, 30).await),
            (11..=40).collect::<Vec<u64>>()
        );

        append_run_state(&mut writer, &run_id, &job_id, RunState::Completed);
        fs::write(run_dir(&paths, &run_id).join(FINISH_FILE), "").unwrap();
        let ended = wait_for_state(&mut node.client, &job_id, JobState::Succeeded).await;
        assert_eq!(ended.last_error, None);
        let rest = read_to_end(&mut stream).await;
        assert_eq!(sequences(&rest), vec![41], "the last record, then the stream ends");
        assert_eq!(final_state_on_disk(&paths, &run_id), "succeeded");
        assert_eq!(
            records_in(&run_events_log_path(&paths, &run_id)).len(),
            41,
            "the node never wrote the log"
        );
    }

    // ---- job.json files that name another process ----

    // A job.json may name a pid that is now some unrelated long-lived process: one with other start ticks, one
    // recorded in an earlier boot (same ticks), or one that doesn't lead its own process group (a member of someone
    // else's group, which the node's group signals would hit). None of them is the job's: each job ends failed with
    // RRS-J3002 at once, and none of the processes is ever signalled, even after the reconnect timeout.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_file_pointing_at_an_unrelated_live_pid_is_never_signalled_and_ends_failed() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let mut other_ticks = spawn_group_leader("exec sleep 600", &folder.path);
        let mut earlier_boot = spawn_group_leader("exec sleep 600", &folder.path);
        let mut group_with_member = spawn_group_leader(LEADER_WITH_CHILD, &folder.path);
        let _guards = [
            GroupGuard {
                process_group_id: other_ticks.id(),
            },
            GroupGuard {
                process_group_id: earlier_boot.id(),
            },
            GroupGuard {
                process_group_id: group_with_member.id(),
            },
        ];
        let member_pid: u32 = wait_for_file(&folder.path.join(CHILD_PID_FILE))
            .await
            .trim()
            .parse()
            .unwrap();

        let mut cases = Vec::new();
        let mut identity = identity_of(other_ticks.id());
        let untouched_first = identity.clone();
        identity.start_ticks += 1;
        cases.push((new_run_id(), identity, untouched_first));
        let mut identity = identity_of(earlier_boot.id());
        let untouched_second = identity.clone();
        identity.boot_id = "an-earlier-boot".to_string();
        cases.push((new_run_id(), identity, untouched_second));
        // The member's true identity, group and all: only that it doesn't lead its group gives it away.
        let member = identity_of(member_pid);
        assert_ne!(member.pid, member.process_group_id);
        cases.push((new_run_id(), member.clone(), member));
        let mut job_ids = Vec::new();
        for (run_id, recorded, _) in &cases {
            job_ids.push(write_live_job_file(&paths, run_id, recorded).job_id);
        }

        let (node, mut client) = start_node(node_options_in(&folder.path, SHORT_RECONNECT_TIMEOUT)).await;
        for ((run_id, _, untouched), job_id) in cases.iter().zip(&job_ids) {
            let ended = wait_for_state(&mut client, job_id, JobState::Failed).await;
            assert_eq!(error_code(&ended), "RRS-J3002", "{ended:?}");
            assert_eq!(final_state_on_disk(&paths, run_id), "failed");
            assert!(still_runs_as(untouched));
        }
        tokio::time::sleep(SHORT_RECONNECT_TIMEOUT + AFTER_SEVERAL_POLLS).await;
        for (_, _, untouched) in &cases {
            assert!(still_runs_as(untouched), "{untouched:?} was signalled");
        }
        assert!(still_runs_as(&identity_of(group_with_member.id())));
        node.abort();
        for process in [&mut other_ticks, &mut earlier_boot, &mut group_with_member] {
            process.kill().unwrap();
            process.wait().unwrap();
        }
    }

    // ---- A gone job's end ----

    // The node was down when these jobs' processes ended, so it saw no exit code: each job's end is its last run
    // state in the log, or RRS-J3002 when it left none. The exit-5 edge (#199): a job that wrote nothing to the log,
    // whose stderr.log ends with the RRS-J3006 refusal, fails with RRS-J3006 and never takes the running copy's
    // completed run as its own. stderr.log is the run's: a job that completed after an earlier job's refusal still
    // succeeds. Each terminal job.json keeps what the node read back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_gone_jobs_end_comes_from_its_log_but_never_after_exit_five() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let mut ended_process = spawn_group_leader("exit 0", &folder.path);
        let gone = identity_of(ended_process.id());
        ended_process.wait().unwrap();

        let completed_run = new_run_id();
        let completed = write_live_job_file(&paths, &completed_run, &gone);
        let mut writer = new_run_log(&paths, &completed_run);
        append_metrics(&mut writer, &completed_run, &completed.job_id, 1, 3);
        append_run_state(&mut writer, &completed_run, &completed.job_id, RunState::Completed);

        let silent_run = new_run_id();
        let silent = write_live_job_file(&paths, &silent_run, &gone);
        let mut writer = new_run_log(&paths, &silent_run);
        append_metrics(&mut writer, &silent_run, &silent.job_id, 1, 3);
        fs::write(
            run_dir(&paths, &silent_run).join(STDERR_LOG),
            "Traceback (most recent call last):\n",
        )
        .unwrap();

        // Refused the log while the running copy, a job the node didn't know, wrote it and completed.
        let refused_run = new_run_id();
        let refused = write_live_job_file(&paths, &refused_run, &gone);
        let running_copy = new_id(IdPrefix::Job).unwrap();
        let mut writer = new_run_log(&paths, &refused_run);
        append_metrics(&mut writer, &refused_run, &running_copy, 1, 3);
        append_run_state(&mut writer, &refused_run, &running_copy, RunState::Completed);
        let stderr = format!("starting\n{EVENT_LOG_LOCKED_LINE}\n");
        fs::write(run_dir(&paths, &refused_run).join(STDERR_LOG), stderr).unwrap();

        // An earlier job of the run was refused the log; this one then ran, printed nothing and completed.
        let rerun_run = new_run_id();
        write_earlier_refusal(&paths, &rerun_run);
        let rerun = write_live_job_file(&paths, &rerun_run, &gone);
        let mut writer = new_run_log(&paths, &rerun_run);
        append_metrics(&mut writer, &rerun_run, &rerun.job_id, 1, 3);
        append_run_state(&mut writer, &rerun_run, &rerun.job_id, RunState::Completed);

        let (node, mut client) = start_node(node_options_in(&folder.path, LONG_TIMEOUT)).await;
        let succeeded = wait_for_state(&mut client, &completed.job_id, JobState::Succeeded).await;
        assert_eq!(succeeded.last_error, None);
        assert_terminal_file_keeps(&paths, &completed, "succeeded");

        let failed = wait_for_state(&mut client, &silent.job_id, JobState::Failed).await;
        assert_eq!(error_code(&failed), "RRS-J3002");
        let shown = failed.last_error.unwrap();
        assert!(shown.message.contains(&silent.job_id), "{}", shown.message);
        assert!(shown.path.ends_with(STDERR_LOG), "{}", shown.path);

        let locked = wait_for_state(&mut client, &refused.job_id, JobState::Failed).await;
        assert_eq!(error_code(&locked), "RRS-J3006", "{locked:?}");
        assert_eq!(final_state_on_disk(&paths, &refused_run), "failed");

        let rerun_ended = wait_for_state(&mut client, &rerun.job_id, JobState::Succeeded).await;
        assert_eq!(rerun_ended.last_error, None);
        assert_eq!(final_state_on_disk(&paths, &rerun_run), "succeeded");
        node.abort();
    }

    // ---- A worker that never comes back ----

    // A live job whose worker doesn't register again within the reconnect timeout is killed with its whole group
    // (the pid's identity checked first) and fails with RRS-J3003.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_job_that_never_registers_again_fails_with_j3003_and_its_group_is_killed() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let mut leader = spawn_group_leader(LEADER_WITH_CHILD, &folder.path);
        let _group = GroupGuard {
            process_group_id: leader.id(),
        };
        let child_pid: u32 = wait_for_file(&folder.path.join(CHILD_PID_FILE))
            .await
            .trim()
            .parse()
            .unwrap();
        let worker = identity_of(leader.id());
        let run_id = new_run_id();
        let job_file = write_live_job_file(&paths, &run_id, &worker);

        let (node, mut client) = start_node(node_options_in(&folder.path, SHORT_RECONNECT_TIMEOUT)).await;
        let lost = list_job(&mut client, &job_file.job_id).await;
        assert_eq!(lost.state(), JobState::Lost);
        let failed = wait_for_state(&mut client, &job_file.job_id, JobState::Failed).await;
        assert_eq!(error_code(&failed), "RRS-J3003", "{failed:?}");
        wait_until_stopped(leader.id()).await;
        wait_until_stopped(child_pid).await;
        assert_eq!(final_state_on_disk(&paths, &run_id), "failed");
        node.abort();
        leader.wait().unwrap();
    }

    // A Register for a lost job with a wrong token is refused with RRS-J2002, and that is all: the job stays lost,
    // holding its run, and its process is never signalled. Its real worker may still register.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_token_for_a_lost_job_is_refused_and_the_job_stays_lost() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let mut leader = spawn_group_leader("exec sleep 600", &folder.path);
        let _group = GroupGuard {
            process_group_id: leader.id(),
        };
        let worker = identity_of(leader.id());
        let run_id = new_run_id();
        let job_file = write_live_job_file(&paths, &run_id, &worker);

        let (node, mut client) = start_node(node_options_in(&folder.path, LONG_TIMEOUT)).await;
        assert_eq!(list_job(&mut client, &job_file.job_id).await.state(), JobState::Lost);
        let refusal = register_with_token(&paths, &job_file.job_id, &"cd".repeat(32)).await;
        assert_eq!(refusal.map(|found| found.code), Some("RRS-J2002".to_string()));
        tokio::time::sleep(AFTER_SEVERAL_POLLS).await;
        let still_lost = list_job(&mut client, &job_file.job_id).await;
        assert_eq!(still_lost.state(), JobState::Lost, "{still_lost:?}");
        assert!(still_runs_as(&worker), "the job's process was signalled");
        assert_eq!(final_state_on_disk(&paths, &run_id), "");
        node.abort();
        leader.kill().unwrap();
        leader.wait().unwrap();
    }

    // ---- Heartbeats ----

    fn job_in(beat: &HeartbeatRequest, job_id: &str) -> Option<JobState> {
        let found = beat.jobs.iter().find(|info| info.job_id == job_id);
        found.map(|info| info.state())
    }

    // The regular beat is an hour away, so every beat here is one a change asked for: the handshake, a new job, its
    // kill. Each comes at once.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_state_change_sends_a_heartbeat_at_once() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        install_worker(&folder.path, WAITING_WORKER);
        let hub_socket = node_hub_socket_path(&paths);
        let hub = start_fake_hub(&hub_socket);
        let mut options = node_options_in(&folder.path, LONG_TIMEOUT);
        options.settings.heartbeat_seconds = HOUR_SECONDS;
        let (node, mut client) = start_node(options).await;
        assert!(hub.heartbeats().is_empty(), "no daemon has handshaken yet");

        let handshake = paired_daemon_handshake(&folder.path, &hub_socket);
        let answer = client.handshake(handshake).await.unwrap().into_inner();
        assert_eq!(answer.diagnostic, None);
        let asked = Instant::now();
        let (arrived, first) = hub.wait_for_heartbeat(0, |_| true).await;
        assert!(arrived - asked < IMMEDIATE_BEAT);
        assert_eq!(first.node_id, answer.node_id);
        assert!(first.telemetry.is_some());
        // Each beat carries the session's token, which the daemon's hub checks (contracts 4.4).
        assert_eq!(hub.session_tokens()[0], Some(answer.session_token.clone()));

        let spec = job_spec(&new_run_id());
        let job_id = spec.job_id.clone();
        let seen = hub.heartbeats().len();
        let asked = Instant::now();
        let started = client.start_job(spec).await.unwrap().into_inner();
        let _worker_group = GroupGuard {
            process_group_id: started.process_group_id,
        };
        let (arrived, _) = hub
            .wait_for_heartbeat(seen, |beat| job_in(beat, &job_id).is_some())
            .await;
        assert!(arrived - asked < IMMEDIATE_BEAT);

        let seen = hub.heartbeats().len();
        let asked = Instant::now();
        signal(&mut client, &job_id, JobSignal::Kill).await;
        let ended = |beat: &HeartbeatRequest| job_in(beat, &job_id) == Some(JobState::Cancelled);
        let (arrived, _) = hub.wait_for_heartbeat(seen, ended).await;
        assert!(arrived - asked < IMMEDIATE_BEAT);
        node.abort();
    }

    // Without any change, a beat still goes every nodes.heartbeat_seconds.
    #[tokio::test(flavor = "multi_thread")]
    async fn heartbeats_go_every_heartbeat_seconds_without_changes() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let hub_socket = node_hub_socket_path(&paths);
        let hub = start_fake_hub(&hub_socket);
        let mut options = node_options_in(&folder.path, LONG_TIMEOUT);
        options.settings.heartbeat_seconds = 1;
        let (node, mut client) = start_node(options).await;
        let handshake = paired_daemon_handshake(&folder.path, &hub_socket);
        client.handshake(handshake).await.unwrap();
        let (second, _) = hub.wait_for_heartbeat(1, |_| true).await;
        let (third, _) = hub.wait_for_heartbeat(2, |_| true).await;
        assert!(third - second >= Duration::from_millis(900), "{:?}", third - second);
        node.abort();
    }

    // ---- kill -9 with a worker that registers again ----

    fn fake_worker_events(paths: &DataPaths, run_id: &str, event: &str) -> Vec<serde_json::Value> {
        let path = run_dir(paths, run_id).join(FAKE_WORKER_RECORD);
        let text = fs::read_to_string(path).unwrap_or_default();
        let entries = text
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap());
        entries.filter(|entry| entry["event"] == event).collect()
    }

    /// The fake worker's `bin/python`, running `script` in the cpu environment.
    fn fake_worker_launcher(folder: &Path, script: &str) -> String {
        let crate_folder = Path::new(env!("CARGO_MANIFEST_DIR"));
        let python = crate_folder.join(CPU_PYTHON);
        assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
        let script_path = folder.join(SCRIPT_FILE);
        fs::write(&script_path, script).unwrap();
        format!(
            "#!/bin/sh\nexec '{}' -I '{}' --script '{}' \"$@\"\n",
            python.display(),
            crate_folder.join(FAKE_WORKER).display(),
            script_path.display()
        )
    }

    // The weeks 3-4 exit test at the node (platform flow 2.3, plan §10 row 5): kill -9 of the node while a
    // registered worker runs. The worker keeps writing its log; the restarted node takes the job back as `lost`; the
    // worker registers again with its token, the node checks it against job_token_hash and calls Hello, and the job
    // is `running`. The daemon's watch from its ack gets every record the worker wrote meanwhile, exactly once, and
    // STOP still reaches the worker through Control; its end comes from its log.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_node_killed_mid_run_reattaches_the_registered_worker_and_loses_no_event() {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let paths = test_paths(&folder.path);
        let script = r#"{"register_again_on_sigusr1": true, "control": {"STOP": {"exit": 0}}}"#;
        install_worker(&folder.path, &fake_worker_launcher(&folder.path, script));
        let mut node = start_node_process(&folder.path).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let job_id = spec.job_id.clone();
        let started = node.client.start_job(spec).await.unwrap().into_inner();
        let _worker_group = GroupGuard {
            process_group_id: started.process_group_id,
        };
        wait_for_state(&mut node.client, &job_id, JobState::Running).await;
        let mut writer = new_run_log(&paths, &run_id);
        append_metrics(&mut writer, &run_id, &job_id, 1, 100);
        let mut stream = watch(&mut node.client, &run_id, 0).await;
        let acknowledged = take_records(&mut stream, 100).await;
        assert_eq!(sequences(&acknowledged), (1..=100).collect::<Vec<u64>>());
        drop(stream);

        node.kill_9();
        let worker = identity_of(started.pid);
        append_metrics(&mut writer, &run_id, &job_id, 101, 250);
        let mut node = start_node_process(&folder.path).await;
        assert_eq!(list_job(&mut node.client, &job_id).await.state(), JobState::Lost);
        assert!(still_runs_as(&worker), "the worker outlived the node");
        // The real worker notices the new worker-host.sock; the fake one registers again when told.
        Command::new("kill")
            .args(["-s", "USR1", &worker.pid.to_string()])
            .output()
            .unwrap();
        wait_for_state(&mut node.client, &job_id, JobState::Running).await;
        let hellos = fake_worker_events(&paths, &run_id, "hello");
        assert_eq!(hellos.len(), 2, "{hellos:?}");
        assert!(hellos.iter().all(|hello| hello["token_ok"] == true));
        let registered = fake_worker_events(&paths, &run_id, "registered");
        assert!(
            registered.iter().all(|answer| answer["diagnostic_code"] == ""),
            "{registered:?}"
        );

        let mut resumed = watch(&mut node.client, &run_id, 100).await;
        let mut after_restart = take_records(&mut resumed, 150).await;
        append_metrics(&mut writer, &run_id, &job_id, 251, 260);
        append_run_state(&mut writer, &run_id, &job_id, RunState::Completed);
        after_restart.extend(take_records(&mut resumed, 11).await);
        assert_eq!(sequences(&after_restart), (101..=261).collect::<Vec<u64>>());
        let all_records = records_in(&run_events_log_path(&paths, &run_id));
        assert_eq!(after_restart, all_records[100..].to_vec());

        signal(&mut node.client, &job_id, JobSignal::Stop).await;
        let ended = wait_for_state(&mut node.client, &job_id, JobState::Succeeded).await;
        assert_eq!(ended.last_error, None);
        assert_eq!(read_to_end(&mut resumed).await, Vec::<EventRecord>::new());
        assert_eq!(final_state_on_disk(&paths, &run_id), "succeeded");
    }
}
