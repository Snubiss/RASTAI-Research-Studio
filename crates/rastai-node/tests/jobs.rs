//! Jobs in their own process groups, through a running node's NodeService over node.sock, as the daemon starts and
//! signals them (layout: rust-node-gateway-proto.md 7.4; platform flows 5.1, 5.2, 5.4 and 5.5; plan §10 row 24).
//!
//! Two kinds of stand-in worker run behind a stand-in environment whose `bin/python` the node launches with
//! `-m rastai_worker run --job-file <job.json>`:
//! - Shell scripts, for everything that needs no `Register`: the process group and environment, the register
//!   timeout, the group signals and the exit codes. They run in every `cargo test` on Linux.
//! - `tests/support/fake_worker.py`, a scripted WorkerService that registers, for what goes through `Control` and
//!   the running state. It needs the cpu environment (`just env-cpu`), which a plain `cargo test` on a CI runner
//!   doesn't have, so those tests are `#[ignore]`d and `just test-node-jobs` runs them (`just check` includes it).
//!
//! Linux only: the node reads /proc for a process's identity and signals groups with procps' `kill`.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod on_linux {
    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use rastai_core::ids::{IdPrefix, new_id};
    use rastai_core::paths::{DataPaths, run_dir, run_events_log_path};
    use rastai_formats::hashing::hash_bytes;
    use rastai_node::error::NodeError;
    use rastai_node::jobs::job_file::{JobFile, job_file_path, read_job_file, write_job_file};
    use rastai_node::jobs::signals::read_process_identity;
    use rastai_node::jobs::supervisor::{JobEnvironment, JobOptions};
    use rastai_proto::common::v1::{Diagnostic, EventClass, JobState, RunState};
    use rastai_proto::events::v1::event_payload::Payload;
    use rastai_proto::events::v1::run_state_changed::State;
    use rastai_proto::events::v1::{DiagnosticReported, EventPayload, EventRecord, RunStateChanged};
    use rastai_proto::node::v1::node_service_client::NodeServiceClient;
    use rastai_proto::node::v1::{JobInfo, JobKind, JobSignal, JobSpec, ListJobsRequest, SignalJobRequest};
    use rastai_store::event_log::writer::{SyncPolicy, append_event, create_event_log};
    use tokio::task::JoinHandle;
    use tonic::transport::Channel;

    use crate::common::{
        NvidiaSmi, TestFolder, kill_job_groups_under, machine_from_fixture, new_test_folder, start_node,
        test_node_options,
    };
    use crate::common::{test_paths, write_tool};

    const TEST_FOLDER_PREFIX: &str = "rnjb";
    const HARDWARE_FIXTURE: &str = "no_gpu";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
    const ENVIRONMENT_ID: &str = "cpu";
    /// Long enough that a test's job never meets it unless the test is about it.
    const LONG_TIMEOUT: Duration = Duration::from_secs(60);
    /// The shortened register timeout and grace period of the tests about them.
    const SHORT_REGISTER_TIMEOUT: Duration = Duration::from_secs(1);
    const SHORT_GRACE: Duration = Duration::from_millis(1500);
    /// How long a test waits for a job or a file to get where it should.
    const WAIT_LIMIT: Duration = Duration::from_secs(30);
    /// The group signal is sent at once; a few seconds cover a loaded machine's /proc catching up.
    const GROUP_GONE_WAIT: Duration = Duration::from_secs(5);
    const POLL: Duration = Duration::from_millis(20);
    /// The worker's notes, in its run folder.
    const READY_FILE: &str = "ready.txt";
    const ENVIRONMENT_FILE: &str = "environment.txt";
    const STAT_FILE: &str = "stat.txt";
    const CHILD_PID_FILE: &str = "child.pid";
    const CHILD_TERM_FILE: &str = "child-term.txt";
    const EXIT_CODE_FILE: &str = "exit-code";
    const FAKE_WORKER_RECORD: &str = "fake-worker-record.jsonl";
    /// The fake worker, and the cpu environment's interpreter that runs it, relative to this crate.
    const FAKE_WORKER: &str = "tests/support/fake_worker.py";
    const CPU_PYTHON: &str = "../../.dev/envs/cpu/bin/python";
    /// The fake worker's script, in the test folder.
    const SCRIPT_FILE: &str = "script.json";
    /// 128 + 9: how JobInfo.exit_code shows a SIGKILL.
    const KILLED_EXIT_CODE: i32 = 137;

    /// Writes the environment and the process's stat, then waits; its sleeps are short, so a failed test leaves
    /// nothing running for long. `$5` is the job file (`-m rastai_worker run --job-file <path>`).
    const RECORDING_WORKER: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
env > "$run_folder/environment.txt"
cat /proc/$$/stat > "$run_folder/stat.txt"
echo ready > "$run_folder/ready.txt"
exec sleep 60
"#;
    /// A worker with a child in its group, which never registers.
    const WORKER_WITH_CHILD: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
sleep 60 &
echo $! > "$run_folder/child.pid"
echo ready > "$run_folder/ready.txt"
exec sleep 60
"#;
    /// Ignores SIGINT (and so do its child and its own sleep, which inherit the ignored signal). The child shows that
    /// the kill after the grace reaches the whole group, not only the leader.
    const IGNORES_INTERRUPT: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
trap '' INT
sleep 60 &
echo $! > "$run_folder/child.pid"
echo ready > "$run_folder/ready.txt"
exec sleep 60
"#;
    /// SIGINT is its cancel flag: exit 3.
    const HONORS_INTERRUPT: &str = r#"#!/bin/sh
trap 'exit 3' INT
echo ready > "$(dirname "$5")/ready.txt"
while :; do sleep 1; done
"#;
    /// SIGTERM is preemption: exit 75. Its child notes the SIGTERM it gets as a member of the group. The worker is
    /// ready only once its child has set its trap, so the signal can't arrive before the child listens for it.
    const PREEMPTIBLE_WITH_CHILD: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
sh -c 'trap "echo term > \"$1/child-term.txt\"; exit 0" TERM; echo ready > "$1/child-ready.txt"; while :; do sleep 1; done' child "$run_folder" &
trap 'exit 75' TERM
while [ ! -s "$run_folder/child-ready.txt" ]; do sleep 0.02; done
echo ready > "$run_folder/ready.txt"
while :; do sleep 1; done
"#;
    /// Exits 2, a bad launch, without a word on stderr.
    const EXITS_SILENTLY: &str = "#!/bin/sh\nexit 2\n";
    /// Exits with the code in its run folder's `exit-code`, after a line on stderr.
    const EXITS_WITH_CODE: &str = r#"#!/bin/sh
run_folder=$(dirname "$5")
echo "the stand-in worker's last words" >&2
exit "$(cat "$run_folder/exit-code")"
"#;

    /// A node with one stand-in environment, `cpu`.
    struct JobNode {
        folder: TestFolder,
        paths: DataPaths,
        client: NodeServiceClient<Channel>,
        node: JoinHandle<Result<(), NodeError>>,
    }

    impl Drop for JobNode {
        fn drop(&mut self) {
            // Before the abort: aborting the node leaves its worker running, and only the node knew its group.
            kill_job_groups_under(&self.folder.path);
            self.node.abort();
        }
    }

    /// Starts a node whose `cpu` environment runs `worker_script` as its `bin/python`.
    async fn start_job_node(worker_script: &str, register_timeout: Duration, stop_grace: Duration) -> JobNode {
        start_job_node_with(|_folder| worker_script.to_string(), register_timeout, stop_grace).await
    }

    /// Starts a node whose `cpu` environment's `bin/python` is what `make_worker` writes for the test folder.
    async fn start_job_node_with(
        make_worker: impl Fn(&Path) -> String,
        register_timeout: Duration,
        stop_grace: Duration,
    ) -> JobNode {
        let folder = new_test_folder(TEST_FOLDER_PREFIX);
        let environment_folder = folder.path.join("env");
        fs::create_dir_all(environment_folder.join("bin")).unwrap();
        write_tool(&environment_folder.join("bin/python"), &make_worker(&folder.path));
        let sources = machine_from_fixture(HARDWARE_FIXTURE, &folder.path, NvidiaSmi::Recorded, TOOL_TIMEOUT);
        let mut options = test_node_options(&folder.path, sources);
        options.jobs = JobOptions {
            worker_register_timeout: register_timeout,
            stop_grace,
            host_reconnect_timeout: LONG_TIMEOUT,
            environments: vec![JobEnvironment {
                environment_id: ENVIRONMENT_ID.to_string(),
                folder: environment_folder,
            }],
        };
        let (node, client) = start_node(options).await;
        JobNode {
            paths: test_paths(&folder.path),
            folder,
            client,
            node,
        }
    }

    /// A node whose `cpu` environment runs the fake worker with the JSON `script`, in the cpu environment's Python.
    async fn start_fake_worker_node(script: &str, stop_grace: Duration) -> JobNode {
        let crate_folder = Path::new(env!("CARGO_MANIFEST_DIR"));
        let python = crate_folder.join(CPU_PYTHON);
        assert!(python.exists(), "{} is missing: run `just env-cpu`", python.display());
        let fake_worker = crate_folder.join(FAKE_WORKER);
        let make_worker = |folder: &Path| {
            let script_path = folder.join(SCRIPT_FILE);
            fs::write(&script_path, script).unwrap();
            format!(
                "#!/bin/sh\nexec '{}' -I '{}' --script '{}' \"$@\"\n",
                python.display(),
                fake_worker.display(),
                script_path.display()
            )
        };
        start_job_node_with(make_worker, LONG_TIMEOUT, stop_grace).await
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

    fn new_run_id() -> String {
        new_id(IdPrefix::Run).unwrap()
    }

    async fn start_job(node: &mut JobNode, spec: JobSpec) -> JobInfo {
        node.client.start_job(spec).await.unwrap().into_inner()
    }

    async fn signal(node: &mut JobNode, job_id: &str, signal: JobSignal) -> Option<Diagnostic> {
        let request = SignalJobRequest {
            job_id: job_id.to_string(),
            signal: signal as i32,
        };
        node.client.signal_job(request).await.unwrap().into_inner().diagnostic
    }

    async fn list_job(node: &mut JobNode, job_id: &str) -> JobInfo {
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

    /// Polls ListJobs until the job is in `wanted`.
    async fn wait_for_state(node: &mut JobNode, job_id: &str, wanted: JobState) -> JobInfo {
        let started = Instant::now();
        loop {
            let info = list_job(node, job_id).await;
            if info.state() == wanted {
                return info;
            }
            let gave_up = is_ended(&info) || started.elapsed() > WAIT_LIMIT;
            assert!(
                !gave_up,
                "job is {:?}, waiting for {wanted:?}: {info:?}\nits stderr.log:\n{}",
                info.state(),
                stderr_log(node, &info.run_id)
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Polls ListJobs until the job has ended.
    async fn wait_for_end(node: &mut JobNode, job_id: &str) -> JobInfo {
        let started = Instant::now();
        loop {
            let info = list_job(node, job_id).await;
            if is_ended(&info) {
                return info;
            }
            assert!(started.elapsed() < WAIT_LIMIT, "job didn't end: {info:?}");
            tokio::time::sleep(POLL).await;
        }
    }

    fn is_ended(info: &JobInfo) -> bool {
        matches!(
            info.state(),
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }

    /// What the job's worker printed on stderr, for a failed test's message.
    fn stderr_log(node: &JobNode, run_id: &str) -> String {
        fs::read_to_string(run_folder(node, run_id).join("stderr.log")).unwrap_or_default()
    }

    async fn wait_for_file(path: &Path) -> String {
        let started = Instant::now();
        loop {
            match fs::read_to_string(path) {
                Ok(text) if !text.is_empty() => return text,
                _ => {}
            }
            assert!(started.elapsed() < WAIT_LIMIT, "{} didn't appear", path.display());
            tokio::time::sleep(POLL).await;
        }
    }

    /// Gone, or a zombie its parent hasn't reaped yet: either way it no longer runs.
    fn process_has_stopped(pid: u32) -> bool {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
            Err(_) => true,
        }
    }

    async fn wait_until_stopped(pid: u32) {
        let started = Instant::now();
        while !process_has_stopped(pid) {
            assert!(started.elapsed() < WAIT_LIMIT, "process {pid} is still running");
            tokio::time::sleep(POLL).await;
        }
    }

    fn error_code(info: &JobInfo) -> String {
        info.last_error
            .as_ref()
            .map(|found| found.code.clone())
            .unwrap_or_default()
    }

    fn error_message(info: &JobInfo) -> String {
        info.last_error
            .as_ref()
            .map(|found| found.message.clone())
            .unwrap_or_default()
    }

    fn run_folder(node: &JobNode, run_id: &str) -> PathBuf {
        run_dir(&node.paths, run_id)
    }

    /// The `NAME=value` lines of an `env` dump.
    fn variables(text: &str) -> Vec<(String, String)> {
        let pairs = text.lines().filter_map(|line| line.split_once('='));
        pairs
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn variable<'a>(variables: &'a [(String, String)], name: &str) -> Option<&'a str> {
        variables
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Ends a job the test leaves running.
    async fn kill_and_wait(node: &mut JobNode, job_id: &str) -> JobInfo {
        assert_eq!(signal(node, job_id, JobSignal::Kill).await, None);
        wait_for_end(node, job_id).await
    }

    /// Writes the run's event log with these records of `job_id`, as its worker would have.
    fn write_run_log(node: &JobNode, run_id: &str, job_id: &str, payloads: Vec<(&str, Payload)>) {
        fs::create_dir_all(run_folder(node, run_id)).unwrap();
        let mut writer = create_event_log(&run_events_log_path(&node.paths, run_id)).unwrap();
        for (event_type, payload) in payloads {
            let mut record = EventRecord {
                run_id: run_id.to_string(),
                job_id: job_id.to_string(),
                event_type: event_type.to_string(),
                event_class: EventClass::Durable as i32,
                payload: Some(EventPayload { payload: Some(payload) }),
                ..EventRecord::default()
            };
            append_event(&mut writer, &mut record, SyncPolicy::Now).unwrap();
        }
    }

    fn run_completed() -> (&'static str, Payload) {
        let change = RunStateChanged {
            reason: String::new(),
            state: Some(State::RunState(RunState::Completed as i32)),
        };
        ("run.state_changed", Payload::RunStateChanged(change))
    }

    fn reported(code: &str) -> (&'static str, Payload) {
        let found = Diagnostic {
            code: code.to_string(),
            message: "logged by the worker".to_string(),
            ..Diagnostic::default()
        };
        let report = DiagnosticReported {
            diagnostic: Some(found),
            subject_id: String::new(),
        };
        ("diagnostic.reported", Payload::DiagnosticReported(report))
    }

    // ---- Spawn: the process group, the environment, job.json ----

    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_starts_in_a_new_process_group_with_the_expected_environment() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(&mut node, spec.clone()).await;
        assert_eq!(started.state(), JobState::Starting, "{started:?}");
        assert!(started.pid > 0);
        assert_eq!(
            started.process_group_id, started.pid,
            "the worker leads its own process group"
        );
        let folder = run_folder(&node, &run_id);
        wait_for_file(&folder.join(READY_FILE)).await;

        // /proc/<pid>/stat: field 5 is the process group, field 6 the session; setsid made both the worker's own.
        let stat = fs::read_to_string(folder.join(STAT_FILE)).unwrap();
        let fields: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
        assert_eq!(fields[2], started.pid.to_string(), "process group");
        assert_eq!(fields[3], started.pid.to_string(), "session");

        let environment = variables(&fs::read_to_string(folder.join(ENVIRONMENT_FILE)).unwrap());
        assert_eq!(variable(&environment, "RASTAI_RUN_ID"), Some(run_id.as_str()));
        assert_eq!(variable(&environment, "HF_HUB_OFFLINE"), Some("1"));
        assert_eq!(variable(&environment, "UV_OFFLINE"), Some("1"));
        assert_eq!(variable(&environment, "PIP_NO_INDEX"), Some("1"));
        assert_eq!(variable(&environment, "CUDA_DEVICE_ORDER"), Some("PCI_BUS_ID"));
        assert_eq!(
            variable(&environment, "CUDA_VISIBLE_DEVICES"),
            Some(""),
            "a job without GPUs sees none"
        );
        let torch_cache = node.folder.path.join("cache/torch/inductor");
        assert_eq!(
            variable(&environment, "TORCHINDUCTOR_CACHE_DIR"),
            Some(torch_cache.to_str().unwrap())
        );
        assert!(variable(&environment, "PATH").is_some());
        // cargo gives the test process these; a cleared environment never passes them on.
        for absent in ["CARGO_MANIFEST_DIR", "CARGO_PKG_NAME", "HSA_OVERRIDE_GFX_VERSION"] {
            assert_eq!(variable(&environment, absent), None, "{absent} reached the job");
        }

        // job.json: private, the process's identity, and the token's hash, never the token.
        let token = variable(&environment, "RASTAI_JOB_TOKEN").unwrap().to_string();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let job_file_text = fs::read_to_string(job_file_path(&folder)).unwrap();
        assert!(!job_file_text.contains(&token), "the token is never on disk");
        let mode = fs::metadata(job_file_path(&folder)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let job_file = read_job_file(&job_file_path(&folder)).unwrap();
        assert_eq!(job_file.job_token_hash, hash_bytes(token.as_bytes()));
        assert_eq!(job_file.job_id, spec.job_id);
        assert_eq!(job_file.kind, "train");
        assert_eq!(job_file.pid, started.pid);
        assert_eq!(job_file.process_group_id, started.pid);
        let identity = read_process_identity(started.pid).unwrap();
        assert_eq!(job_file.process_start_ticks, identity.start_ticks);
        assert_eq!(job_file.boot_id, identity.boot_id);
        assert_eq!(job_file.final_state, "");
        assert_eq!(job_file.job_spec["kind"], "JOB_KIND_TRAIN");

        let ended = kill_and_wait(&mut node, &spec.job_id).await;
        assert_eq!(read_job_file(&job_file_path(&folder)).unwrap().final_state, "cancelled");
        assert_eq!(ended.state(), JobState::Cancelled);
    }

    // ---- Register within 60 s ----

    #[tokio::test(flavor = "multi_thread")]
    async fn no_register_in_time_fails_the_job_and_kills_its_group() {
        let mut node = start_job_node(WORKER_WITH_CHILD, SHORT_REGISTER_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(&mut node, spec.clone()).await;
        let child_pid: u32 = wait_for_file(&run_folder(&node, &run_id).join(CHILD_PID_FILE))
            .await
            .trim()
            .parse()
            .unwrap();
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Failed);
        assert_eq!(error_code(&ended), "RRS-J0002");
        assert!(
            error_message(&ended).contains("within 1 seconds"),
            "{}",
            error_message(&ended)
        );
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
        wait_until_stopped(started.pid).await;
        wait_until_stopped(child_pid).await;
    }

    // ---- Signals to the group (plan §10 row 24) ----

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_without_a_worker_to_call_interrupts_the_group_and_kills_it_after_the_grace() {
        let mut node = start_job_node(IGNORES_INTERRUPT, LONG_TIMEOUT, SHORT_GRACE).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(&mut node, spec.clone()).await;
        wait_for_file(&run_folder(&node, &run_id).join(READY_FILE)).await;
        let child_pid: u32 = wait_for_file(&run_folder(&node, &run_id).join(CHILD_PID_FILE))
            .await
            .trim()
            .parse()
            .unwrap();
        let cancelled_at = Instant::now();
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Cancel).await, None);
        tokio::time::sleep(SHORT_GRACE / 2).await;
        assert_eq!(
            list_job(&mut node, &spec.job_id).await.state(),
            JobState::Starting,
            "SIGINT was ignored"
        );
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert!(cancelled_at.elapsed() >= SHORT_GRACE, "killed before the grace ran out");
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(error_code(&ended), "RRS-J1003");
        assert!(
            error_message(&ended).contains("CANCEL didn't end it"),
            "{}",
            error_message(&ended)
        );
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
        wait_until_stopped(started.pid).await;
        wait_until_stopped(child_pid).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_honored_through_the_interrupt_ends_cancelled() {
        let mut node = start_job_node(HONORS_INTERRUPT, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        wait_for_file(&run_folder(&node, &run_id).join(READY_FILE)).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Cancel).await, None);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(ended.exit_code, Some(3));
        assert_eq!(ended.last_error, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminate_sends_sigterm_to_the_group_and_exit_75_is_preempted() {
        let mut node = start_job_node(PREEMPTIBLE_WITH_CHILD, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        let folder = run_folder(&node, &run_id);
        wait_for_file(&folder.join(READY_FILE)).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Terminate).await, None);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(
            ended.exit_code,
            Some(75),
            "preempted (the reason is checked in supervisor.rs's tests)"
        );
        assert_eq!(ended.last_error, None);
        assert_eq!(
            wait_for_file(&folder.join(CHILD_TERM_FILE)).await.trim(),
            "term",
            "the whole group got SIGTERM"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_sends_sigkill_to_the_group() {
        let mut node = start_job_node(WORKER_WITH_CHILD, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(&mut node, spec.clone()).await;
        let child_pid: u32 = wait_for_file(&run_folder(&node, &run_id).join(CHILD_PID_FILE))
            .await
            .trim()
            .parse()
            .unwrap();
        let ended = kill_and_wait(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(error_code(&ended), "RRS-J1003");
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
        wait_until_stopped(started.pid).await;
        wait_until_stopped(child_pid).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_signal_the_node_cannot_deliver_is_refused_with_j0004() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let unknown = signal(&mut node, "job_unknown", JobSignal::Kill).await.unwrap();
        assert_eq!(unknown.code, "RRS-J0004");
        assert!(unknown.message.contains("no job with this id"), "{}", unknown.message);
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        wait_for_file(&run_folder(&node, &run_id).join(READY_FILE)).await;
        for control_only in [JobSignal::Stop, JobSignal::Pause] {
            let refused = signal(&mut node, &spec.job_id, control_only).await.unwrap();
            assert_eq!(refused.code, "RRS-J0004");
            assert!(refused.message.contains("hasn't registered"), "{}", refused.message);
        }
        let unnamed = signal(&mut node, &spec.job_id, JobSignal::Unspecified).await.unwrap();
        assert_eq!(unnamed.code, "RRS-J0004");
        kill_and_wait(&mut node, &spec.job_id).await;
        let ended = signal(&mut node, &spec.job_id, JobSignal::Terminate).await.unwrap();
        assert!(ended.message.contains("already ended"), "{}", ended.message);
    }

    // ---- Exit causes ----

    /// Starts a job whose worker exits with `code` before it registers, and returns how it ended.
    async fn job_exiting_with(node: &mut JobNode, code: i32) -> (JobInfo, String) {
        let run_id = new_run_id();
        let folder = run_folder(node, &run_id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(EXIT_CODE_FILE), code.to_string()).unwrap();
        let spec = job_spec(&run_id);
        start_job(node, spec.clone()).await;
        (wait_for_end(node, &spec.job_id).await, run_id)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_exit_code_maps_to_its_state_and_code() {
        let mut node = start_job_node(EXITS_WITH_CODE, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let expected = [
            (2, JobState::Failed, "RRS-J2001"),
            (3, JobState::Cancelled, ""),
            (4, JobState::Failed, "RRS-S3002"),
            (5, JobState::Failed, "RRS-J3006"),
            (75, JobState::Cancelled, ""),
            (42, JobState::Failed, "RRS-J1004"),
            // Before Register no exit is a success: the node never met the worker.
            (0, JobState::Failed, "RRS-J1004"),
        ];
        for (code, state, error) in expected {
            let (ended, run_id) = job_exiting_with(&mut node, code).await;
            assert_eq!(ended.state(), state, "exit {code}: {ended:?}");
            assert_eq!(error_code(&ended), error, "exit {code}");
            assert_eq!(ended.exit_code, Some(code));
            let job_file = read_job_file(&job_file_path(&run_folder(&node, &run_id))).unwrap();
            assert_eq!(job_file.job_id, ended.job_id, "exit {code}: job.json is this job's own");
            assert_eq!(
                job_file.final_state,
                rastai_core::states::job_state_name(state),
                "exit {code}"
            );
            if code == 4 {
                let message = error_message(&ended);
                assert!(message.contains("the stand-in worker's last words"), "{message}");
                assert!(message.contains("bytes free"), "{message}");
            }
            if code == 5 {
                let path = run_events_log_path(&node.paths, &run_id);
                assert!(
                    error_message(&ended).contains(path.to_str().unwrap()),
                    "{}",
                    error_message(&ended)
                );
            }
        }
    }

    // The run's stderr.log is appended to across relaunches: a job that wrote nothing there gets the fallback reason,
    // never the earlier job's last line.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_silent_relaunch_never_quotes_the_earlier_jobs_stderr() {
        let mut node = start_job_node(EXITS_SILENTLY, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let folder = run_folder(&node, &run_id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("stderr.log"), "the earlier job's last words\n").unwrap();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(error_code(&ended), "RRS-J2001");
        let message = error_message(&ended);
        assert!(!message.contains("the earlier job's last words"), "{message}");
        assert!(message.contains("it reported a bad launch (exit 2)"), "{message}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_worker_killed_from_outside_fails_naming_sigkill() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(&mut node, spec.clone()).await;
        wait_for_file(&run_folder(&node, &run_id).join(READY_FILE)).await;
        let status = std::process::Command::new("kill")
            .args(["-9", &started.pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Failed);
        assert_eq!(error_code(&ended), "RRS-J1001");
        assert!(error_message(&ended).contains("SIGKILL"), "{}", error_message(&ended));
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
    }

    // ---- Admission: one live job per run (contracts 9.1) ----

    fn folder_listing(folder: &Path) -> BTreeSet<String> {
        let entries = fs::read_dir(folder).unwrap();
        entries
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_job_on_a_live_run_is_refused_and_writes_nothing() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let first = job_spec(&run_id);
        start_job(&mut node, first.clone()).await;
        let folder = run_folder(&node, &run_id);
        wait_for_file(&folder.join(READY_FILE)).await;
        let listing_before = folder_listing(&folder);
        let job_file_before = fs::read(job_file_path(&folder)).unwrap();

        let second = start_job(&mut node, job_spec(&run_id)).await;
        assert_eq!(second.state(), JobState::Failed);
        assert_eq!(error_code(&second), "RRS-J3006");
        assert_eq!(second.pid, 0, "nothing was spawned");
        assert_eq!(
            folder_listing(&folder),
            listing_before,
            "nothing was written into the run folder"
        );
        assert_eq!(
            fs::read(job_file_path(&folder)).unwrap(),
            job_file_before,
            "the running job's job.json stays"
        );
        assert_eq!(
            list_job(&mut node, &first.job_id).await.state(),
            JobState::Starting,
            "the running job goes on"
        );

        // A retried StartJob is the same job, not a second one.
        let retried = start_job(&mut node, first.clone()).await;
        assert_eq!(retried.state(), JobState::Starting);

        // Once the first has ended (and so given the run back), the run takes a new job.
        kill_and_wait(&mut node, &first.job_id).await;
        let third = job_spec(&run_id);
        assert_eq!(start_job(&mut node, third.clone()).await.state(), JobState::Starting);
        kill_and_wait(&mut node, &third.job_id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn starts_racing_for_one_run_admit_exactly_one() {
        let node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let mut racers = Vec::new();
        for _ in 0..8 {
            let mut client = node.client.clone();
            let spec = job_spec(&run_id);
            racers.push(tokio::spawn(async move {
                client.start_job(spec).await.unwrap().into_inner()
            }));
        }
        let mut admitted = Vec::new();
        for racer in racers {
            let info = racer.await.unwrap();
            if info.state() == JobState::Failed {
                assert_eq!(error_code(&info), "RRS-J3006");
            } else {
                admitted.push(info.job_id);
            }
        }
        assert_eq!(admitted.len(), 1, "{admitted:?}");
        let mut node = node;
        kill_and_wait(&mut node, &admitted[0]).await;
    }

    // A job.json with no final state whose process is alive (a job from before a node restart) holds its run too.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_live_job_file_on_disk_holds_its_run() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let folder = run_folder(&node, &run_id);
        fs::create_dir_all(&folder).unwrap();
        // A stand-in process leading its own group, as a worker does, is the live worker; the node must never
        // signal it.
        let mut stand_in = std::process::Command::new("sleep")
            .arg("600")
            .process_group(0)
            .spawn()
            .unwrap();
        let alive = read_process_identity(stand_in.id()).unwrap();
        let mut earlier = JobFile {
            job_id: "job_from_before_the_restart".to_string(),
            run_id: run_id.clone(),
            pid: alive.pid,
            process_group_id: alive.process_group_id,
            process_start_ticks: alive.start_ticks,
            boot_id: alive.boot_id,
            ..JobFile::default()
        };
        write_job_file(&folder, &earlier).unwrap();
        let refused = start_job(&mut node, job_spec(&run_id)).await;
        assert_eq!(error_code(&refused), "RRS-J3006");
        assert_eq!(
            read_job_file(&job_file_path(&folder)).unwrap(),
            earlier,
            "its job.json stays"
        );

        // The same file with a start time that isn't the process's names a reused pid: not live.
        earlier.process_start_ticks += 1;
        write_job_file(&folder, &earlier).unwrap();
        let admitted = job_spec(&run_id);
        assert_eq!(start_job(&mut node, admitted.clone()).await.state(), JobState::Starting);
        kill_and_wait(&mut node, &admitted.job_id).await;
        assert!(
            read_process_identity(stand_in.id()).is_some(),
            "the stand-in was never signalled"
        );
        stand_in.kill().unwrap();
        stand_in.wait().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn jobs_the_node_cannot_run_are_refused() {
        let mut node = start_job_node(RECORDING_WORKER, LONG_TIMEOUT, LONG_TIMEOUT).await;
        let download = JobSpec {
            kind: JobKind::Download as i32,
            ..job_spec(&new_run_id())
        };
        let refused = start_job(&mut node, download).await;
        assert_eq!(error_code(&refused), "RRS-N0004");
        assert!(
            error_message(&refused).contains("job kind download"),
            "{}",
            error_message(&refused)
        );

        let elsewhere = JobSpec {
            environment_id: "cuda13".to_string(),
            ..job_spec(&new_run_id())
        };
        let refused = start_job(&mut node, elsewhere).await;
        assert_eq!(error_code(&refused), "RRS-N0004");
        assert!(
            error_message(&refused).contains("environment cuda13"),
            "{}",
            error_message(&refused)
        );

        let escaping = JobSpec {
            run_id: "../../escape".to_string(),
            ..job_spec(&new_run_id())
        };
        let refused = start_job(&mut node, escaping).await;
        assert_eq!(refused.state(), JobState::Failed);
        assert_eq!(error_code(&refused), "RRS-J0001");
        assert!(!node.paths.data.join("escape").exists());
        assert!(!node.folder.path.join("escape").exists());

        // An environment whose interpreter is gone: admitted, then failed before anything runs.
        fs::remove_file(node.folder.path.join("env/bin/python")).unwrap();
        let run_id = new_run_id();
        let no_python = start_job(&mut node, job_spec(&run_id)).await;
        assert_eq!(no_python.state(), JobState::Failed);
        assert_eq!(error_code(&no_python), "RRS-J0001");
        assert!(
            error_message(&no_python).contains("interpreter"),
            "{}",
            error_message(&no_python)
        );
        let job_file = read_job_file(&job_file_path(&run_folder(&node, &run_id))).unwrap();
        assert_eq!(job_file.final_state, "failed");
    }

    // ---- Through the fake worker: Register, Hello and Control ----

    fn fake_worker_record(node: &JobNode, run_id: &str) -> Vec<serde_json::Value> {
        let path = run_folder(node, run_id).join(FAKE_WORKER_RECORD);
        let text = fs::read_to_string(path).unwrap_or_default();
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    fn recorded_events(record: &[serde_json::Value], event: &str) -> Vec<serde_json::Value> {
        record.iter().filter(|entry| entry["event"] == event).cloned().collect()
    }

    /// Starts a fake-worker job and waits until it is running.
    async fn running_fake_job(node: &mut JobNode) -> (JobSpec, String) {
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        let started = start_job(node, spec.clone()).await;
        assert_eq!(started.state(), JobState::Starting, "{started:?}");
        wait_for_state(node, &spec.job_id, JobState::Running).await;
        (spec, run_id)
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_registered_worker_runs_and_stop_goes_through_control() {
        let script = r#"{"control": {"STOP": {"exit": 0}}}"#;
        let mut node = start_fake_worker_node(script, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        write_run_log(&node, &run_id, &spec.job_id, vec![run_completed()]);
        start_job(&mut node, spec.clone()).await;
        wait_for_state(&mut node, &spec.job_id, JobState::Running).await;
        // The worker records its Register answer only after the node has sent it, so the job can be Running before
        // the line is in the record.
        let registered = wait_for_registrations(&node, &run_id, 1).await;
        assert_eq!(registered[0]["diagnostic_code"], "");
        let record = fake_worker_record(&node, &run_id);
        assert_eq!(
            recorded_events(&record, "hello")[0]["token_ok"],
            true,
            "Hello carried the job token"
        );

        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Stop).await, None);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Succeeded, "{ended:?}");
        assert_eq!(ended.exit_code, Some(0));
        let record = fake_worker_record(&node, &run_id);
        let controls = recorded_events(&record, "control");
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0]["action"], "STOP");
        assert_eq!(controls[0]["token_ok"], true);
        assert!(
            recorded_events(&record, "signal").is_empty(),
            "STOP is a Control value, never a signal"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn cancel_goes_through_control_and_is_killed_after_the_grace_when_ignored() {
        // The fake worker answers CANCEL and does nothing (its Control default), so the node kills it after the grace.
        let script = r#"{"control": {}}"#;
        let mut node = start_fake_worker_node(script, SHORT_GRACE).await;
        let (spec, run_id) = running_fake_job(&mut node).await;
        let cancelled_at = Instant::now();
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Cancel).await, None);
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Stopping);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert!(cancelled_at.elapsed() >= SHORT_GRACE);
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(error_code(&ended), "RRS-J1003");
        let record = fake_worker_record(&node, &run_id);
        assert_eq!(recorded_events(&record, "control")[0]["action"], "CANCEL");
        assert!(
            recorded_events(&record, "signal").is_empty(),
            "the Control call worked, so no SIGINT"
        );
    }

    /// Sends `job_signal` to a running fake-worker job that ignores it, and checks the node kills the group once the
    /// grace runs out (layout 7.4: STOP and TERMINATE escalate to SIGKILL as CANCEL does).
    async fn ignored_signal_is_killed_after_the_grace(script: &str, job_signal: JobSignal, expected_reason: &str) {
        let mut node = start_fake_worker_node(script, SHORT_GRACE).await;
        let (spec, _run_id) = running_fake_job(&mut node).await;
        let signalled_at = Instant::now();
        let pid = signal_and_check_alive_at_half_the_grace(&mut node, &spec.job_id, job_signal).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert!(signalled_at.elapsed() >= SHORT_GRACE, "killed before the grace ran out");
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(error_code(&ended), "RRS-J1003");
        assert!(
            error_message(&ended).contains(expected_reason),
            "{}",
            error_message(&ended)
        );
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
        wait_until_stopped(pid).await;
        assert_group_is_gone(ended.process_group_id);
    }

    /// Signals the job, which moves to stopping, and checks it is still stopping and alive at half the grace.
    /// Returns the worker's pid.
    async fn signal_and_check_alive_at_half_the_grace(node: &mut JobNode, job_id: &str, job_signal: JobSignal) -> u32 {
        let pid = list_job(node, job_id).await.pid;
        assert_eq!(signal(node, job_id, job_signal).await, None);
        assert_eq!(list_job(node, job_id).await.state(), JobState::Stopping);
        tokio::time::sleep(SHORT_GRACE / 2).await;
        assert_eq!(
            list_job(node, job_id).await.state(),
            JobState::Stopping,
            "the worker ignored {job_signal:?}, and the grace hasn't run out"
        );
        assert!(!process_has_stopped(pid), "the worker still runs at half the grace");
        pid
    }

    /// No process is left running in the group.
    fn assert_group_is_gone(process_group_id: u32) {
        let entries = fs::read_dir("/proc").unwrap();
        for entry in entries.flatten() {
            let Some((state, group)) = process_state_and_group(&entry.path()) else {
                continue;
            };
            let runs_in_group = group == process_group_id && state != "Z";
            assert!(
                !runs_in_group,
                "{} of group {process_group_id} still runs",
                entry.path().display()
            );
        }
    }

    /// A /proc entry's state and process group (fields 3 and 5 of its stat); None for an entry that isn't a process
    /// or one that has gone since the listing.
    fn process_state_and_group(proc_entry: &Path) -> Option<(String, u32)> {
        let stat = fs::read_to_string(proc_entry.join("stat")).ok()?;
        let (_, rest) = stat.rsplit_once(')')?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let group = fields.get(2)?.parse().ok()?;
        Some((fields.first()?.to_string(), group))
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn stop_is_killed_after_the_grace_when_ignored() {
        let script = r#"{"control": {"STOP": "ignore"}}"#;
        ignored_signal_is_killed_after_the_grace(script, JobSignal::Stop, "STOP didn't end it").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn terminate_is_killed_after_the_grace_when_ignored() {
        let script = r#"{"on_sigterm": "ignore"}"#;
        ignored_signal_is_killed_after_the_grace(script, JobSignal::Terminate, "TERMINATE didn't end it").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn cancel_honored_through_control_ends_cancelled() {
        let script = r#"{"control": {"CANCEL": {"exit": 3}}}"#;
        let mut node = start_fake_worker_node(script, LONG_TIMEOUT).await;
        let (spec, _run_id) = running_fake_job(&mut node).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Cancel).await, None);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(ended.exit_code, Some(3));
        assert_eq!(ended.last_error, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn pause_resume_and_checkpoint_go_through_control() {
        let mut node = start_fake_worker_node("{}", LONG_TIMEOUT).await;
        let (spec, run_id) = running_fake_job(&mut node).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Pause).await, None);
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Paused);
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Resume).await, None);
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Running);
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::CheckpointNow).await, None);
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Running);
        let actions: Vec<serde_json::Value> = recorded_events(&fake_worker_record(&node, &run_id), "control")
            .into_iter()
            .map(|entry| entry["action"].clone())
            .collect();
        assert_eq!(actions, vec!["PAUSE", "RESUME", "CHECKPOINT_NOW"]);
        kill_and_wait(&mut node, &spec.job_id).await;
    }

    // A worker that registers again with the right token while its job is paused (it reconnected) gets through
    // Register and Hello, and the job stays paused: only a starting job moves to running.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_second_register_leaves_a_paused_job_paused() {
        let mut node = start_fake_worker_node(r#"{"register_again_on_sigusr1": true}"#, LONG_TIMEOUT).await;
        let (spec, run_id) = running_fake_job(&mut node).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Pause).await, None);
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Paused);
        let pid = list_job(&mut node, &spec.job_id).await.pid;
        let status = std::process::Command::new("kill")
            .args(["-USR1", &pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let registered = wait_for_registrations(&node, &run_id, 2).await;
        assert_eq!(registered[1]["diagnostic_code"], "", "the second Register was accepted");
        let hellos = recorded_events(&fake_worker_record(&node, &run_id), "hello");
        assert_eq!(hellos.len(), 2, "the node called Hello on the worker again");
        assert_eq!(list_job(&mut node, &spec.job_id).await.state(), JobState::Paused);
        kill_and_wait(&mut node, &spec.job_id).await;
    }

    /// Waits until the fake worker has recorded `count` answers to its Register calls, and returns them.
    async fn wait_for_registrations(node: &JobNode, run_id: &str, count: usize) -> Vec<serde_json::Value> {
        let started = Instant::now();
        loop {
            let registered = recorded_events(&fake_worker_record(node, run_id), "registered");
            if registered.len() >= count {
                return registered;
            }
            assert!(
                started.elapsed() < WAIT_LIMIT,
                "{} of {count} Register calls answered",
                registered.len()
            );
            tokio::time::sleep(POLL).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn terminate_reaches_a_running_worker_as_sigterm() {
        let mut node = start_fake_worker_node("{}", LONG_TIMEOUT).await;
        let (spec, run_id) = running_fake_job(&mut node).await;
        assert_eq!(signal(&mut node, &spec.job_id, JobSignal::Terminate).await, None);
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled);
        assert_eq!(ended.exit_code, Some(75));
        let signals = recorded_events(&fake_worker_record(&node, &run_id), "signal");
        assert_eq!(signals[0]["name"], "SIGTERM");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_wrong_token_fails_the_job_and_kills_its_group() {
        let script = format!(r#"{{"token": "{}"}}"#, "0".repeat(64));
        let mut node = start_fake_worker_node(&script, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Failed);
        assert_eq!(error_code(&ended), "RRS-J2002");
        assert_eq!(ended.exit_code, Some(KILLED_EXIT_CODE));
        // The node kills the group before it answers, so the worker never reads the refusal, and it never got a
        // Hello: the node calls nothing on a socket whose token it refused.
        let record = fake_worker_record(&node, &run_id);
        assert!(recorded_events(&record, "hello").is_empty());
        assert_eq!(recorded_events(&record, "started").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_running_worker_exit_takes_its_logged_outcome() {
        let script = r#"{"exit_after_register": 1}"#;
        let mut node = start_fake_worker_node(script, LONG_TIMEOUT).await;
        // Exit 1 with a logged diagnostic: failed with that diagnostic.
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        write_run_log(&node, &run_id, &spec.job_id, vec![reported("RRS-T3001")]);
        start_job(&mut node, spec.clone()).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Failed);
        assert_eq!(error_code(&ended), "RRS-T3001");
        // Exit 1 whose log holds only another job's diagnostic: nothing of its own, so RRS-J1004.
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        write_run_log(&node, &run_id, "job_another_attempt", vec![reported("RRS-T3001")]);
        start_job(&mut node, spec.clone()).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(error_code(&ended), "RRS-J1004");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_running_worker_killed_with_kill_9_fails_naming_sigkill() {
        let mut node = start_fake_worker_node("{}", LONG_TIMEOUT).await;
        let (spec, _run_id) = running_fake_job(&mut node).await;
        let pid = list_job(&mut node, &spec.job_id).await.pid;
        let status = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Failed);
        assert_eq!(error_code(&ended), "RRS-J1001");
        assert!(error_message(&ended).contains("SIGKILL"), "{}", error_message(&ended));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn a_worker_of_another_protocol_is_terminated_with_j2003() {
        let mut node = start_fake_worker_node(r#"{"protocol_version": 2}"#, LONG_TIMEOUT).await;
        let run_id = new_run_id();
        let spec = job_spec(&run_id);
        start_job(&mut node, spec.clone()).await;
        let ended = wait_for_end(&mut node, &spec.job_id).await;
        assert_eq!(ended.state(), JobState::Cancelled, "{ended:?}");
        assert_eq!(ended.exit_code, Some(75), "TERMINATE: a checkpoint, then exit 75");
        assert_eq!(error_code(&ended), "RRS-J2003");
        let record = fake_worker_record(&node, &run_id);
        assert_eq!(
            recorded_events(&record, "registered")[0]["diagnostic_code"],
            "RRS-J2003"
        );
        assert_eq!(recorded_events(&record, "signal")[0]["name"], "SIGTERM");
    }

    /// Whether any process other than a zombie is in the process group.
    fn group_has_members(process_group_id: u32) -> bool {
        let entries = fs::read_dir("/proc").unwrap();
        entries.flatten().any(|entry| {
            process_state_and_group(&entry.path())
                .is_some_and(|(state, group)| group == process_group_id && state != "Z")
        })
    }

    async fn wait_until_group_is_gone(process_group_id: u32) {
        let started = Instant::now();
        while group_has_members(process_group_id) {
            assert!(
                started.elapsed() < GROUP_GONE_WAIT,
                "the group {process_group_id} still has members {GROUP_GONE_WAIT:?} after its node was dropped"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    // A failed test drops its node while the worker runs. The worker leads its own group, and aborting the node
    // doesn't stop it, so the node's Drop has to signal the group itself (issue #244).
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the cpu environment (just env-cpu); just test-node-jobs runs it"]
    async fn dropping_the_node_kills_the_running_worker_group() {
        let mut node = start_fake_worker_node("{}", LONG_TIMEOUT).await;
        let (_spec, run_id) = running_fake_job(&mut node).await;
        let job_file = read_job_file(&job_file_path(&run_folder(&node, &run_id))).unwrap();
        let process_group_id = job_file.process_group_id;
        assert!(
            group_has_members(process_group_id),
            "the worker isn't running in group {process_group_id}"
        );
        drop(node);
        wait_until_group_is_gone(process_group_id).await;
    }
}
