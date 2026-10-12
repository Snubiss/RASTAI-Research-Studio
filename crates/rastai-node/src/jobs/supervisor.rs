//! The job supervisor (layout: rust-node-gateway-proto.md section 7.4; platform flows 5.1, 5.2, 5.4 and 5.5): the
//! `JobTable`, `start_job` from admission to a running worker, `signal_job`, the worker's `Register`, and the clean-up
//! when a job ends. The job states and their transitions are contracts 9.1's (rastai-core's states.rs).
//!
//! **One live job per run** (contracts 9.1). Checking that a run has no live job and reserving the run for the new
//! job happen under one hold of `NodeState.jobs`, so two `StartJob`s for one run can't both pass. A live job is one
//! in the table that still holds its run, or a `job.json` with no `final_state` whose process is alive. A refused job
//! fails with `RRS-J3006` and nothing is written into the run folder. An admitted job writes `job.json` only after
//! the reservation, and gives the run back only after its terminal `job.json` (with `final_state`) is written. After
//! exit 5 (another writer holds the event log's lock) that terminal write lands on the refused job's own `job.json`:
//! the writer holding the lock is one the node had no live `job.json` for.
//!
//! Each running job has three background tasks, started in `watch_job`: one waits for the process to exit and
//! classifies the exit (exit_cause.rs) on a blocking thread, since that reads the run's log and writes `job.json`;
//! one fails the job with `RRS-J0002` and kills its group when the worker hasn't registered within
//! `WORKER_REGISTER_TIMEOUT_SECONDS`; one follows its stdout.log and stderr.log onto the live queues (log_lines.rs).
//! A `CANCEL`, `STOP` or `TERMINATE` starts a fourth that kills the group if the job is still alive after
//! `STOP_GRACE_SECONDS` (`RRS-J1003`).
//!
//! Not here yet: leases and sandbox reservations at admission (M3, M5), an environment still installing that keeps
//! the job `queued` with `RRS-H2006` (M2-16), the disk-free check at admission (`RRS-S3001`, with a job's space
//! estimate), the heartbeat sent at once on every state change (M1-45), and reattach after a node restart (M1-45).

use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::paths::{
    job_socket_path, plugin_data_dir, run_dir, run_events_log_path, torch_cache_dir, worker_host_socket_path,
};
use rastai_core::proto_json::{hardware_profile_to_json, job_spec_to_json};
use rastai_core::random::random_bytes;
use rastai_core::states::{is_terminal_job_state, job_state_name, job_transition_allowed};
use rastai_core::time::now_unix_ms;
use rastai_core::versions::WORKER_PROTOCOL_VERSION;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::{log_diagnostic, log_warn};
use rastai_formats::hashing::hash_bytes;
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, JobState};
use rastai_proto::node::v1::{JobInfo, JobKind, JobSignal, JobSpec};
use rastai_proto::worker::v1::worker_service_client::WorkerServiceClient;
use rastai_proto::worker::v1::{HelloRequest, WorkerInfo};
use rastai_transport::grpc_channel::connect_grpc_channel;
use tokio::process::Child;
use tonic::transport::Channel;

use crate::error::node_error_diagnostic;
use crate::hardware::{AMD_VENDOR, NVIDIA_VENDOR};
use crate::jobs::exit_cause::{
    EXIT_EVENT_LOG_LOCKED, ExitFacts, JobOutcome, NodeKill, OomWatch, ProcessExit, RunLogFacts, StderrTail,
    classify_exit, exit_code_for_info, oom_kill_rose, process_exit_from_status, read_run_log_facts, start_oom_watch,
};
use crate::jobs::job_file::{JobFile, job_file_path, read_job_file, write_job_file};
use crate::jobs::log_lines::{JobLogs, STDERR_LOG_FILE, STDOUT_LOG_FILE, follow_job_logs, read_log_tail};
use crate::jobs::signals::{GroupSignal, ProcessIdentity, is_same_process, send_control, signal_process_group};
use crate::jobs::spawn::{
    REDACTED_TOKEN, SpawnedWorker, WorkerLaunch, WorkerVariableInputs, node_variables, prepare_job_socket_folder,
    spawn_worker, worker_variables,
};
use crate::node::{NodeState, current_hardware};
use crate::permissions::{JobPermissions, job_permissions};
use crate::worker_host_service::request_with_job_token;

/// How long a `CANCEL`, `STOP` or `TERMINATE` may take before the node kills the group (contracts 15.10).
pub const STOP_GRACE_SECONDS: u64 = 120;
/// How long a new worker has to call `Register` (contracts 9.1 and 15.10).
pub const WORKER_REGISTER_TIMEOUT_SECONDS: u64 = 60;
/// The job token: 32 random bytes, written as 64 lowercase hex characters (contracts 3.2 and 4.5).
const JOB_TOKEN_BYTES: usize = 32;
/// How many ended jobs `ListJobs` keeps showing. The daemon reconciles after a reconnect from the newest ones; an
/// older end is in its own `jobs` table already.
const ENDED_JOBS_KEPT: usize = 256;
/// The stderr lines a diagnostic about a job's end quotes from.
const STDERR_TAIL_LINES: usize = 20;
/// The kinds a Python worker runs (contracts 9.1). `plugin_module` joins with plugin_modules.rs (M4); `download`,
/// `transfer` and `serve_fast` are node-native (M2, M9, M3); `daemon_task` never reaches a node.
const PYTHON_JOB_KINDS: &[JobKind] = &[
    JobKind::ServeInspectable,
    JobKind::Train,
    JobKind::Eval,
    JobKind::Gym,
    JobKind::Lens,
    JobKind::DataImport,
    JobKind::DataCheck,
    JobKind::Compile,
    JobKind::Export,
    JobKind::DoctorSelftest,
];
const JOB_KIND_NAME_PREFIX: &str = "JOB_KIND_";
const JOB_SIGNAL_NAME_PREFIX: &str = "JOB_SIGNAL_";

/// How the supervisor times jobs and where it finds their environments. `default_job_options` gives the product's
/// times; tests shorten them (platform flow 5.1: "timeout shortened by a test setting").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOptions {
    pub worker_register_timeout: Duration,
    pub stop_grace: Duration,
    /// The environments this node can run jobs in. Until uv environments land (M2-16), whoever starts the node names
    /// them: the embedded node's start (M1-47) gives it the cpu environment that `just env-cpu` syncs.
    pub environments: Vec<JobEnvironment>,
}

/// One environment a job can run in: its id (`cpu`, `cuda13`, contracts 8.6) and its folder (`bin/python` inside).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEnvironment {
    pub environment_id: String,
    pub folder: PathBuf,
}

/// Every job the node knows, live or ended, in the order they arrived.
#[derive(Debug, Default)]
pub struct JobTable {
    pub records: Vec<JobRecord>,
}

/// One job. `info` is what `ListJobs` and heartbeats report.
#[derive(Debug)]
pub struct JobRecord {
    pub info: JobInfo,
    pub spec: JobSpec,
    pub run_folder: PathBuf,
    /// `blake3:<64 hex>` of the job token; the token itself is known again only when the worker registers.
    pub token_hash: String,
    pub worker: Option<WorkerLink>,
    /// Why, when the state alone doesn't say: `preempted` (contracts 9.1).
    pub reason: String,
    pub permissions: JobPermissions,
    /// Who the worker's process is, from the spawn on.
    pub process: Option<ProcessIdentity>,
    /// Where this job's output starts in the run's `stdout.log` and `stderr.log`, which earlier jobs of the run
    /// appended to.
    pub stdout_start_offset: u64,
    pub stderr_start_offset: u64,
    pub oom_watch: Option<OomWatch>,
    /// Why the node killed the group itself, if it did.
    pub node_kill: Option<NodeKill>,
    /// Why the node refused the worker at `Register` and sent it TERMINATE (`RRS-J2003`).
    pub start_refusal: Option<Diagnostic>,
    /// True from admission until the terminal `job.json` is written: the run is this job's.
    pub holds_run: bool,
}

/// The node's line to a registered worker. Debug is hand-written: it never prints the token (spawn.rs,
/// `REDACTED_TOKEN`).
#[derive(Clone)]
pub struct WorkerLink {
    /// The token's 64 hex characters, sent in the `rastai-job-token` metadata of every `WorkerService` call.
    pub job_token: String,
    pub channel: Channel,
}

impl std::fmt::Debug for WorkerLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerLink")
            .field("job_token", &REDACTED_TOKEN)
            .field("channel", &self.channel)
            .finish()
    }
}

/// What `admit_job` decided.
enum Admission {
    Admitted,
    /// A `StartJob` the node already has (the daemon retried after losing the answer): the job as it is now.
    AlreadyKnown(JobInfo),
    Refused(JobInfo),
}

/// What `finish_job` writes before it ends a job.
enum TerminalWrite {
    /// The job is unknown or has already ended: nothing to do.
    AlreadyEnded,
    /// A refused job never wrote a `job.json`, so there is none to finish.
    NothingWritten,
    /// The terminal `job.json` and the run folder it goes in. Boxed because a `JobFile` is far bigger than the other
    /// two variants (clippy's large_enum_variant).
    Write(PathBuf, Box<JobFile>),
}

/// What `signal_job` needs to know about a job, copied out of the table.
struct SignalTarget {
    worker: Option<WorkerLink>,
    process: Option<ProcessIdentity>,
}

/// The product's job times for these environments.
pub fn default_job_options(environments: Vec<JobEnvironment>) -> JobOptions {
    JobOptions {
        worker_register_timeout: Duration::from_secs(WORKER_REGISTER_TIMEOUT_SECONDS),
        stop_grace: Duration::from_secs(STOP_GRACE_SECONDS),
        environments,
    }
}

/// Starts a job: admission, `job.json`, the spawn, and the two watchers. A refusal or a failed start comes back as
/// the job's own `failed` state with its diagnostic in `last_error`, never as a call error (contracts 4).
pub async fn start_job(state: &Arc<NodeState>, spec: JobSpec) -> JobInfo {
    match admit_job(state, &spec) {
        Admission::AlreadyKnown(info) | Admission::Refused(info) => info,
        Admission::Admitted => launch_job(state, spec).await,
    }
}

/// Every job the node knows.
pub fn list_jobs(state: &NodeState) -> Vec<JobInfo> {
    let table = lock_jobs(state);
    table.records.iter().map(|record| record.info.clone()).collect()
}

/// Moves a live job to `to` when contracts 9.1 allows it from where it is; true when it moved. `reason` and
/// `diagnostic` replace the job's own when given.
pub fn set_job_state(
    state: &NodeState,
    job_id: &str,
    to: JobState,
    reason: &str,
    diagnostic: Option<Diagnostic>,
) -> bool {
    let mut table = lock_jobs(state);
    let Some(record) = find_record_mut(&mut table, job_id) else {
        return false;
    };
    let from = record.info.state();
    if !job_transition_allowed(from, to) {
        return false;
    }
    record.info.state = to as i32;
    if !reason.is_empty() {
        record.reason = reason.to_string();
    }
    if let Some(found) = diagnostic {
        record.info.last_error = Some(diagnostic_to_proto(&found));
    }
    true
}

/// Delivers one `JobSignal` (contracts 4.4). None when it was delivered; otherwise `RRS-J0004` with the reason, or
/// the worker's own refusal from `ControlResponse`.
pub async fn signal_job(state: &Arc<NodeState>, job_id: &str, signal: JobSignal) -> Option<ProtoDiagnostic> {
    let target = match signal_target(state, job_id) {
        Ok(target) => target,
        Err(reason) => return Some(not_delivered(job_id, signal, &reason)),
    };
    let delivered = match signal {
        JobSignal::Cancel => cancel_job(state, job_id, &target).await,
        JobSignal::Stop => stop_job(state, job_id, &target).await,
        JobSignal::Pause | JobSignal::Resume | JobSignal::CheckpointNow | JobSignal::Step => {
            control_job(state, job_id, &target, signal).await
        }
        JobSignal::Terminate => terminate_job(state, job_id, &target).await,
        JobSignal::Kill => kill_job_group(state, job_id, NodeKill::KillSignal).await.map(|()| None),
        JobSignal::Unspecified => Err("no signal was named".to_string()),
    };
    match delivered {
        Ok(worker_refusal) => worker_refusal,
        Err(reason) => Some(not_delivered(job_id, signal, &reason)),
    }
}

/// `WorkerHostService.Register`: checks the token against the job's hash, connects to the job socket, calls
/// `Hello`, and moves a starting job to `running` (platform flow 5.1). A worker that registers again later (it
/// reconnected) only gets its link replaced: its job keeps its state, so a paused job stays paused. A token that doesn't match a starting job
/// means something else is on the socket: the job fails with `RRS-J2002` and its group is killed.
// A refusal is one diagnostic, the response's own field; it is the rare path, so its size costs nothing in practice
// (as in rastai-core's scopes.rs).
#[allow(clippy::result_large_err)]
pub async fn register_worker(
    state: &Arc<NodeState>,
    job_id: &str,
    job_token: &str,
    socket_path: &Path,
) -> Result<(), Diagnostic> {
    let token_refused = || diagnostic("RRS-J2002", &[("job_id", job_id)]);
    let Some((token_hash, job_state)) = registration_target(state, job_id) else {
        return Err(token_refused());
    };
    if hash_bytes(job_token.as_bytes()) != token_hash {
        if job_state == JobState::Starting {
            let _ = kill_job_group(state, job_id, NodeKill::TokenRejected).await;
        }
        return Err(token_refused());
    }
    let expected_socket = job_socket_path(&state.paths, job_id);
    if socket_path != expected_socket {
        let reason = format!(
            "it serves on {}, not on {}",
            socket_path.display(),
            expected_socket.display()
        );
        return Err(diagnostic("RRS-J2001", &[("job_id", job_id), ("reason", &reason)]));
    }
    let channel = connect_grpc_channel(&expected_socket)
        .await
        .map_err(|error| node_error_diagnostic(&error.into()))?;
    let worker = hello_worker(&channel, job_token, job_id).await?;
    if worker.protocol_version != WORKER_PROTOCOL_VERSION {
        return Err(refuse_worker_protocol(state, job_id, worker.protocol_version).await);
    }
    let link = WorkerLink {
        job_token: job_token.to_string(),
        channel,
    };
    attach_worker(state, job_id, link);
    Ok(())
}

// ---- Admission ----

/// Decides, under one hold of the job table, whether the job may start, and enters it in the table either way.
fn admit_job(state: &NodeState, spec: &JobSpec) -> Admission {
    let refusal_before_lock = spec_refusal(state, spec);
    let mut table = lock_jobs(state);
    if let Some(record) = find_record(&table, &spec.job_id) {
        return Admission::AlreadyKnown(record.info.clone());
    }
    let refusal = refusal_before_lock.or_else(|| live_run_refusal(state, &table, spec));
    let mut record = new_record(state, spec);
    let admission = match refusal {
        Some(found) => {
            record.info.state = JobState::Failed as i32;
            record.info.last_error = Some(diagnostic_to_proto(&found));
            Admission::Refused(record.info.clone())
        }
        None => {
            record.holds_run = true;
            Admission::Admitted
        }
    };
    table.records.push(record);
    forget_old_ended_jobs(&mut table);
    admission
}

/// Why this node can never run the job: ids that aren't ids, a kind it doesn't run, an environment it doesn't
/// have (contracts 9.1, queued → failed). Until M2-16 no environment is ever still installing, so one that isn't in
/// `JobOptions.environments` is one the node can't provide (layout 7.4 step 1), not a wait.
fn spec_refusal(state: &NodeState, spec: &JobSpec) -> Option<Diagnostic> {
    if let Some(reason) = id_problem(spec) {
        return Some(diagnostic(
            "RRS-J0001",
            &[("job_id", &spec.job_id), ("reason", &reason)],
        ));
    }
    let lacks =
        |capability: String| diagnostic("RRS-N0004", &[("node_id", &state.node_id), ("capability", &capability)]);
    let kind = spec.kind();
    if !PYTHON_JOB_KINDS.contains(&kind) {
        return Some(lacks(format!("job kind {}", job_kind_text(kind))));
    }
    if environment_folder(state, &spec.environment_id).is_none() {
        return Some(lacks(format!("environment {}", spec.environment_id)));
    }
    None
}

/// The ids become folder and socket names, so they must be real ids (contracts 8.6), never a path.
fn id_problem(spec: &JobSpec) -> Option<String> {
    if !id_has_prefix(&spec.job_id, IdPrefix::Job) {
        return Some(format!("its job id {:?} isn't a job_ id", spec.job_id));
    }
    if !id_has_prefix(&spec.run_id, IdPrefix::Run) {
        return Some(format!("its run id {:?} isn't a run_ id", spec.run_id));
    }
    let plugin_id = &spec.plugin_id;
    if !plugin_id.is_empty() && !rastai_formats::validate::meaning::plugin_manifest::is_plugin_id(plugin_id) {
        return Some(format!("its plugin id {plugin_id:?} isn't a plugin id"));
    }
    None
}

/// `RRS-J3006` when the run already has a live job: one in the table that holds the run, or a `job.json` with no
/// `final_state` whose process is still the one it names. Called with the table locked, so reading that one small
/// file is part of the atomic check.
fn live_run_refusal(state: &NodeState, table: &JobTable, spec: &JobSpec) -> Option<Diagnostic> {
    let held_in_table = table
        .records
        .iter()
        .any(|record| record.holds_run && record.spec.run_id == spec.run_id);
    let run_folder = run_dir(&state.paths, &spec.run_id);
    if !held_in_table && !job_file_is_live(&job_file_path(&run_folder)) {
        return None;
    }
    let event_log = run_events_log_path(&state.paths, &spec.run_id);
    Some(diagnostic("RRS-J3006", &[("path", &event_log.display().to_string())]))
}

/// A `job.json` whose job hasn't ended and whose process is alive and still the one it recorded. A file that can't
/// be read names no live process; the worker's lock on the event log is the backstop (exit 5).
fn job_file_is_live(path: &Path) -> bool {
    let Ok(job_file) = read_job_file(path) else {
        return false;
    };
    if !job_file.final_state.is_empty() || job_file.pid == 0 {
        return false;
    }
    is_same_process(&ProcessIdentity {
        pid: job_file.pid,
        process_group_id: job_file.process_group_id,
        start_ticks: job_file.process_start_ticks,
        boot_id: job_file.boot_id,
    })
}

fn new_record(state: &NodeState, spec: &JobSpec) -> JobRecord {
    JobRecord {
        info: JobInfo {
            job_id: spec.job_id.clone(),
            run_id: spec.run_id.clone(),
            kind: spec.kind,
            state: JobState::Queued as i32,
            ..JobInfo::default()
        },
        spec: spec.clone(),
        run_folder: run_dir(&state.paths, &spec.run_id),
        token_hash: String::new(),
        worker: None,
        reason: String::new(),
        permissions: job_permissions(spec),
        process: None,
        stdout_start_offset: 0,
        stderr_start_offset: 0,
        oom_watch: None,
        node_kill: None,
        start_refusal: None,
        holds_run: false,
    }
}

/// Drops the oldest ended jobs beyond `ENDED_JOBS_KEPT`. A job that still holds its run is never dropped.
fn forget_old_ended_jobs(table: &mut JobTable) {
    let is_ended = |record: &JobRecord| !record.holds_run && is_terminal_job_state(record.info.state());
    let mut ended_count = table.records.iter().filter(|record| is_ended(record)).count();
    table.records.retain(|record| {
        if ended_count > ENDED_JOBS_KEPT && is_ended(record) {
            ended_count -= 1;
            return false;
        }
        true
    });
}

// ---- Launch ----

/// Spawns an admitted job and starts its watchers. A failure before the worker runs fails the job with `RRS-J0001`.
async fn launch_job(state: &Arc<NodeState>, spec: JobSpec) -> JobInfo {
    let job_id = spec.job_id.clone();
    set_job_state(state, &job_id, JobState::Starting, "", None);
    match spawn_job_worker(state, &spec).await {
        Ok(child) => watch_job(state, &job_id, child),
        Err(reason) => {
            let failed = diagnostic("RRS-J0001", &[("job_id", &job_id), ("reason", &reason)]);
            finish_job(state, &job_id, failed_outcome(failed), None);
        }
    }
    job_info(state, &job_id).unwrap_or_default()
}

/// Makes the token, writes `job.json`, starts the worker and records who it is (layout 7.4 steps 2 to 4).
async fn spawn_job_worker(state: &Arc<NodeState>, spec: &JobSpec) -> Result<Child, String> {
    let run_folder = run_dir(&state.paths, &spec.run_id);
    let socket_path = job_socket_path(&state.paths, &spec.job_id);
    std::fs::create_dir_all(&run_folder)
        .map_err(|error| format!("the run folder {} can't be created: {error}", run_folder.display()))?;
    prepare_job_socket_folder(&socket_path)?;
    let job_token = new_job_token()?;
    let token_hash = hash_bytes(job_token.as_bytes());
    let job_file = first_job_file(state, spec, &token_hash);
    write_job_file(&run_folder, &job_file)?;
    record_token_hash(state, &spec.job_id, &token_hash);
    let launch = worker_launch(state, spec, &job_token, &job_file)?;
    let spawned = spawn_worker(&launch).await?;
    record_spawned_worker(state, &spec.job_id, &run_folder, job_file, spawned).await
}

/// Records who the worker is, in the table and in `job.json` (layout 7.4 step 4). If that rewrite fails, the worker
/// is killed with its group and waited on before the error returns: no exit watcher or register timer exists yet, and
/// the failed job releases its run, so a worker left running would hold the run's event log behind the next job's
/// back (layout 7.4 step 1: nothing is half-started).
async fn record_spawned_worker(
    state: &NodeState,
    job_id: &str,
    run_folder: &Path,
    mut job_file: JobFile,
    spawned: SpawnedWorker,
) -> Result<Child, String> {
    let identity = spawned.identity;
    let mut child = spawned.child;
    record_process(
        state,
        job_id,
        &identity,
        spawned.stdout_start_offset,
        spawned.stderr_start_offset,
    );
    job_file.pid = identity.pid;
    job_file.process_group_id = identity.process_group_id;
    job_file.process_start_ticks = identity.start_ticks;
    job_file.boot_id = identity.boot_id.clone();
    job_file.started_unix_ms = job_info(state, job_id).map_or(0, |info| info.started_unix_ms);
    if let Err(reason) = write_job_file(run_folder, &job_file) {
        stop_unwatched_worker(job_id, &identity, &mut child).await;
        return Err(reason);
    }
    Ok(child)
}

/// SIGKILL to the group of a worker that has no watchers, then a wait, so it is gone when this returns. If the group
/// can't be signalled, the leader is still killed through its child handle.
async fn stop_unwatched_worker(job_id: &str, identity: &ProcessIdentity, child: &mut Child) {
    if let Err(reason) = signal_process_group(identity, GroupSignal::Kill).await {
        log_warn(
            "a worker that couldn't be recorded couldn't be killed with its group",
            &[("job_id", job_id), ("reason", &reason)],
        );
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

/// `job.json` before the spawn: everything but the process.
fn first_job_file(state: &NodeState, spec: &JobSpec, token_hash: &str) -> JobFile {
    JobFile {
        job_id: spec.job_id.clone(),
        run_id: spec.run_id.clone(),
        kind: job_kind_text(spec.kind()),
        state: job_state_name(JobState::Starting).to_string(),
        job_spec: job_spec_to_json(spec),
        job_socket_path: path_text(&job_socket_path(&state.paths, &spec.job_id)),
        worker_host_socket_path: path_text(&worker_host_socket_path(&state.paths)),
        run_folder: path_text(&run_dir(&state.paths, &spec.run_id)),
        environment_id: spec.environment_id.clone(),
        hardware_profile: hardware_profile_to_json(&current_hardware(state)),
        job_token_hash: token_hash.to_string(),
        ..JobFile::default()
    }
}

fn worker_launch(
    state: &NodeState,
    spec: &JobSpec,
    job_token: &str,
    job_file: &JobFile,
) -> Result<WorkerLaunch, String> {
    let environment_folder = environment_folder(state, &spec.environment_id)
        .ok_or_else(|| format!("the environment {} is gone", spec.environment_id))?;
    let plugin_data_folder = if spec.plugin_id.is_empty() {
        None
    } else {
        Some(plugin_data_dir(&state.paths, &spec.plugin_id))
    };
    let inputs = WorkerVariableInputs {
        job_token: job_token.to_string(),
        run_id: spec.run_id.clone(),
        gpu_indices: spec
            .resources
            .as_ref()
            .map(|resources| resources.gpu_indices.clone())
            .unwrap_or_default(),
        amd_gpus: computes_on_amd(state),
        torch_cache_folder: torch_cache_dir(&state.paths),
        plugin_data_folder,
    };
    let variables = worker_variables(&node_variables(), &inputs);
    if variables.removed_gfx_override {
        log_diagnostic(&diagnostic("RRS-H4002", &[("job_id", &spec.job_id)]), &[]);
    }
    let run_folder = PathBuf::from(&job_file.run_folder);
    Ok(WorkerLaunch {
        environment_folder,
        job_file: job_file_path(&run_folder),
        run_folder,
        variables: variables.variables,
    })
}

/// A box with both vendors computes on NVIDIA (platform flow 1.2), so AMD only when no NVIDIA GPU is there.
fn computes_on_amd(state: &NodeState) -> bool {
    let hardware = current_hardware(state);
    let has_vendor = |vendor: &str| hardware.gpus.iter().any(|gpu| gpu.vendor == vendor);
    has_vendor(AMD_VENDOR) && !has_vendor(NVIDIA_VENDOR)
}

fn new_job_token() -> Result<String, String> {
    let bytes = random_bytes(JOB_TOKEN_BYTES).map_err(|error| format!("no job token could be made: {error:?}"))?;
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(hex.concat())
}

fn environment_folder(state: &NodeState, environment_id: &str) -> Option<PathBuf> {
    let environments = &state.job_options.environments;
    let found = environments
        .iter()
        .find(|environment| environment.environment_id == environment_id);
    found.map(|environment| environment.folder.clone())
}

// ---- Watching a running job ----

/// Starts the exit watcher, the register timer and the log follower for a spawned job.
fn watch_job(state: &Arc<NodeState>, job_id: &str, mut child: Child) {
    start_log_follower(state, job_id);
    let exit_state = Arc::clone(state);
    let exit_job_id = job_id.to_string();
    tokio::spawn(async move {
        let exit = match child.wait().await {
            Ok(status) => process_exit_from_status(status),
            Err(error) => {
                log_warn(
                    "waiting for a job's process failed",
                    &[("job_id", &exit_job_id), ("reason", &error.to_string())],
                );
                ProcessExit::Code(-1)
            }
        };
        // The end of a job reads the run's whole event log and writes job.json with an fsync: blocking file work
        // that must not hold a runtime thread (the rule rastai-store's blocking reads follow too).
        let finish_job_id = exit_job_id.clone();
        let finished = tokio::task::spawn_blocking(move || finish_after_exit(&exit_state, &finish_job_id, exit)).await;
        if let Err(error) = finished {
            log_warn(
                "ending a job failed",
                &[("job_id", &exit_job_id), ("reason", &error.to_string())],
            );
        }
    });
    let timer_state = Arc::clone(state);
    let timer_job_id = job_id.to_string();
    tokio::spawn(async move {
        let timeout = timer_state.job_options.worker_register_timeout;
        tokio::time::sleep(timeout).await;
        if job_info(&timer_state, &timer_job_id).map(|info| info.state()) == Some(JobState::Starting) {
            let kill = NodeKill::RegisterTimeout {
                timeout_seconds: timeout.as_secs(),
            };
            let _ = kill_job_group(&timer_state, &timer_job_id, kill).await;
        }
    });
}

/// Follows the job's stdout.log and stderr.log onto the live queues (log_lines.rs), from where its own output starts.
fn start_log_follower(state: &Arc<NodeState>, job_id: &str) {
    let found = {
        let table = lock_jobs(state);
        find_record(&table, job_id).map(|record| JobLogs {
            stdout_path: record.run_folder.join(STDOUT_LOG_FILE),
            stderr_path: record.run_folder.join(STDERR_LOG_FILE),
            stdout_offset: record.stdout_start_offset,
            stderr_offset: record.stderr_start_offset,
        })
    };
    if let Some(logs) = found {
        tokio::spawn(follow_job_logs(Arc::clone(state), job_id.to_string(), logs));
    }
}

/// Classifies the exit and ends the job.
fn finish_after_exit(state: &NodeState, job_id: &str, exit: ProcessExit) {
    let Some(facts) = exit_facts(state, job_id, exit) else {
        return;
    };
    let outcome = classify_exit(&facts);
    finish_job(state, job_id, outcome, Some(exit_code_for_info(exit)));
}

/// What `classify_exit` decides from: the record's own facts, the run's log for this job, stderr's tail, free space.
fn exit_facts(state: &NodeState, job_id: &str, exit: ProcessExit) -> Option<ExitFacts> {
    let (record_facts, run_id, run_folder, oom_watch, stderr_start_offset) = {
        let table = lock_jobs(state);
        let record = find_record(&table, job_id)?;
        let record_facts = ExitFacts {
            job_id: job_id.to_string(),
            exit,
            registered: record.worker.is_some(),
            node_kill: record.node_kill,
            oom_kill_rose: false,
            start_refusal: record.start_refusal.clone(),
            run_log: Default::default(),
            stderr: StderrTail::default(),
            event_log_path: String::new(),
            free_bytes: None,
        };
        (
            record_facts,
            record.spec.run_id.clone(),
            record.run_folder.clone(),
            record.oom_watch.clone(),
            record.stderr_start_offset,
        )
    };
    let event_log = run_events_log_path(&state.paths, &run_id);
    // After exit 5 the log's frames are the running copy's, so nothing is read from it for this job (contracts 9.1).
    let run_log = if exit == ProcessExit::Code(EXIT_EVENT_LOG_LOCKED) {
        RunLogFacts::default()
    } else {
        read_run_log_facts(&event_log, job_id)
    };
    Some(ExitFacts {
        oom_kill_rose: oom_watch.as_ref().is_some_and(oom_kill_rose),
        run_log,
        stderr: StderrTail {
            path: stderr_path_for_diagnostic(state, &run_folder),
            lines: read_log_tail(
                &run_folder.join(STDERR_LOG_FILE),
                stderr_start_offset,
                STDERR_TAIL_LINES,
            ),
        },
        event_log_path: path_text(&event_log),
        free_bytes: rastai_store::disk_space::free_bytes(&run_folder).ok(),
        ..record_facts
    })
}

/// `runs/<run_id>/stderr.log`, relative to the data folder as the JobInfo golden shows it.
fn stderr_path_for_diagnostic(state: &NodeState, run_folder: &Path) -> String {
    let stderr = run_folder.join(STDERR_LOG_FILE);
    match stderr.strip_prefix(&state.paths.data) {
        Ok(relative) => path_text(relative),
        Err(_) => path_text(&stderr),
    }
}

// ---- The end of a job ----

/// Ends the job: writes the terminal `job.json` first, then puts the job in its terminal state and gives the run
/// back in one step (contracts 9.1). So a job that `ListJobs` shows as ended has already released its run, and the
/// daemon can start the run's next job at once. The process is gone, so its end is recorded even where 9.1's table
/// has no arrow for it (a paused job that crashed); that is logged.
fn finish_job(state: &NodeState, job_id: &str, outcome: JobOutcome, exit_code: Option<i32>) {
    match terminal_job_file_to_write(state, job_id, outcome.state) {
        TerminalWrite::AlreadyEnded => return,
        TerminalWrite::NothingWritten => {}
        TerminalWrite::Write(run_folder, job_file) => write_terminal_job_file(job_id, &run_folder, &job_file),
    }
    let _ = std::fs::remove_file(job_socket_path(&state.paths, job_id));
    apply_terminal_outcome(state, job_id, &outcome, exit_code);
    // The heartbeat that reports this end at once joins with M1-45 (heartbeat.rs).
}

/// The terminal `job.json` to write, if the job wrote one (a refused job never did).
fn terminal_job_file_to_write(state: &NodeState, job_id: &str, final_state: JobState) -> TerminalWrite {
    let table = lock_jobs(state);
    let Some(record) = find_record(&table, job_id) else {
        return TerminalWrite::AlreadyEnded;
    };
    if is_terminal_job_state(record.info.state()) {
        return TerminalWrite::AlreadyEnded;
    }
    if !record.holds_run {
        return TerminalWrite::NothingWritten;
    }
    let job_file = terminal_job_file(state, record, final_state);
    TerminalWrite::Write(record.run_folder.clone(), Box::new(job_file))
}

/// A terminal `job.json` that can't be written leaves the last one in place, with no `final_state`; a restarted node
/// then checks its process, finds it gone and takes the job's end from the run's log (layout 7.4, "Reattach").
fn write_terminal_job_file(job_id: &str, run_folder: &Path, job_file: &JobFile) {
    if let Err(reason) = write_job_file(run_folder, job_file) {
        log_warn(
            "the terminal job.json couldn't be written",
            &[("job_id", job_id), ("reason", &reason)],
        );
    }
}

/// Records the outcome on the job and releases its run, under one hold of the table. A job that has ended keeps its
/// first end, even if two ends race to here.
fn apply_terminal_outcome(state: &NodeState, job_id: &str, outcome: &JobOutcome, exit_code: Option<i32>) {
    let mut table = lock_jobs(state);
    let Some(record) = find_record_mut(&mut table, job_id) else {
        return;
    };
    let from = record.info.state();
    if is_terminal_job_state(from) {
        return;
    }
    if !job_transition_allowed(from, outcome.state) {
        let names = format!("{} -> {}", job_state_name(from), job_state_name(outcome.state));
        log_warn(
            "a job ended by a transition contracts 9.1 doesn't list",
            &[("job_id", job_id), ("transition", &names)],
        );
    }
    record.info.state = outcome.state as i32;
    record.info.exit_code = exit_code;
    record.info.last_error = outcome.diagnostic.as_ref().map(diagnostic_to_proto);
    record.reason = outcome.reason.clone();
    record.worker = None;
    record.holds_run = false;
    forget_old_ended_jobs(&mut table);
}

/// The last `job.json` of a job: as written at the spawn, with the final state.
fn terminal_job_file(state: &NodeState, record: &JobRecord, final_state: JobState) -> JobFile {
    let mut job_file = first_job_file(state, &record.spec, &record.token_hash);
    if let Some(process) = &record.process {
        job_file.pid = process.pid;
        job_file.process_group_id = process.process_group_id;
        job_file.process_start_ticks = process.start_ticks;
        job_file.boot_id = process.boot_id.clone();
    }
    job_file.started_unix_ms = record.info.started_unix_ms;
    job_file.state = job_state_name(final_state).to_string();
    job_file.final_state = job_file.state.clone();
    job_file
}

// ---- Signals ----

fn signal_target(state: &NodeState, job_id: &str) -> Result<SignalTarget, String> {
    let table = lock_jobs(state);
    let Some(record) = find_record(&table, job_id) else {
        return Err("the node has no job with this id".to_string());
    };
    let job_state = record.info.state();
    if is_terminal_job_state(job_state) {
        return Err(format!("the job has already ended ({})", job_state_name(job_state)));
    }
    Ok(SignalTarget {
        worker: record.worker.clone(),
        process: record.process.clone(),
    })
}

/// CANCEL: `Control(CANCEL)`, or SIGINT to the group when there is no worker to call or the call fails (the
/// worker's cancel flag either way); then the grace timer.
async fn cancel_job(
    state: &Arc<NodeState>,
    job_id: &str,
    target: &SignalTarget,
) -> Result<Option<ProtoDiagnostic>, String> {
    let control_result = match &target.worker {
        Some(link) => send_control(link, JobSignal::Cancel).await,
        None => Err("the worker hasn't registered yet".to_string()),
    };
    match control_result {
        Ok(Some(worker_refusal)) => return Ok(Some(worker_refusal)),
        Ok(None) => {}
        Err(_) => interrupt_group(target).await?,
    }
    begin_stopping(state, job_id, JobSignal::Cancel);
    Ok(None)
}

/// STOP: `Control(STOP)` only, so SIGTERM keeps meaning preemption (contracts 4.4); then the grace timer.
async fn stop_job(
    state: &Arc<NodeState>,
    job_id: &str,
    target: &SignalTarget,
) -> Result<Option<ProtoDiagnostic>, String> {
    let link = target.worker.as_ref().ok_or("the worker hasn't registered yet")?;
    if let Some(worker_refusal) = send_control(link, JobSignal::Stop).await? {
        return Ok(Some(worker_refusal));
    }
    begin_stopping(state, job_id, JobSignal::Stop);
    Ok(None)
}

/// PAUSE, RESUME, CHECKPOINT_NOW and STEP: `Control` only, then the state they lead to (contracts 4.4).
async fn control_job(
    state: &Arc<NodeState>,
    job_id: &str,
    target: &SignalTarget,
    signal: JobSignal,
) -> Result<Option<ProtoDiagnostic>, String> {
    let link = target.worker.as_ref().ok_or("the worker hasn't registered yet")?;
    if let Some(worker_refusal) = send_control(link, signal).await? {
        return Ok(Some(worker_refusal));
    }
    let next_state = match signal {
        JobSignal::Pause | JobSignal::Step => Some(JobState::Paused),
        JobSignal::Resume => Some(JobState::Running),
        _ => None,
    };
    if let Some(to) = next_state {
        set_job_state(state, job_id, to, "", None);
    }
    Ok(None)
}

/// TERMINATE: SIGTERM to the group, preemption (exit 75); then the grace timer.
async fn terminate_job(
    state: &Arc<NodeState>,
    job_id: &str,
    target: &SignalTarget,
) -> Result<Option<ProtoDiagnostic>, String> {
    let process = target.process.as_ref().ok_or("the job has no process yet")?;
    signal_process_group(process, GroupSignal::Terminate).await?;
    begin_stopping(state, job_id, JobSignal::Terminate);
    Ok(None)
}

async fn interrupt_group(target: &SignalTarget) -> Result<(), String> {
    let process = target.process.as_ref().ok_or("the job has no process yet")?;
    signal_process_group(process, GroupSignal::Interrupt).await
}

/// A running or paused job moves to `stopping` (a starting one stays `starting`: 9.1 has no arrow for it), and the
/// group is killed if the job is still alive after the grace period.
fn begin_stopping(state: &Arc<NodeState>, job_id: &str, signal: JobSignal) {
    set_job_state(state, job_id, JobState::Stopping, "", None);
    let timer_state = Arc::clone(state);
    let timer_job_id = job_id.to_string();
    tokio::spawn(async move {
        let grace = timer_state.job_options.stop_grace;
        tokio::time::sleep(grace).await;
        let still_live = job_info(&timer_state, &timer_job_id).is_some_and(|info| !is_terminal_job_state(info.state()));
        if still_live {
            let kill = NodeKill::GraceExpired {
                signal,
                grace_seconds: grace.as_secs(),
            };
            let _ = kill_job_group(&timer_state, &timer_job_id, kill).await;
        }
    });
}

/// SIGKILL to the job's group. The cause is recorded first, so the exit watcher, which may see the exit before
/// this returns, already knows the node did it. When no SIGKILL went out, the cause this call recorded is cleared
/// again, so a later crash isn't read as the node's kill (`RRS-J1003`, cancelled).
async fn kill_job_group(state: &NodeState, job_id: &str, cause: NodeKill) -> Result<(), String> {
    let (process, recorded_cause) = {
        let mut table = lock_jobs(state);
        let record = find_record_mut(&mut table, job_id).ok_or("the node has no job with this id")?;
        let recorded_cause = record.node_kill.is_none();
        if recorded_cause {
            record.node_kill = Some(cause);
        }
        (record.process.clone(), recorded_cause)
    };
    let signalled = match process {
        Some(process) => signal_process_group(&process, GroupSignal::Kill).await,
        None => Err("the job has no process yet".to_string()),
    };
    if signalled.is_err() && recorded_cause {
        clear_node_kill(state, job_id);
    }
    signalled
}

fn clear_node_kill(state: &NodeState, job_id: &str) {
    let mut table = lock_jobs(state);
    if let Some(record) = find_record_mut(&mut table, job_id) {
        record.node_kill = None;
    }
}

fn not_delivered(job_id: &str, signal: JobSignal, reason: &str) -> ProtoDiagnostic {
    let signal_text = signal.as_str_name().trim_start_matches(JOB_SIGNAL_NAME_PREFIX);
    let found = diagnostic(
        "RRS-J0004",
        &[("signal", signal_text), ("job_id", job_id), ("reason", reason)],
    );
    diagnostic_to_proto(&found)
}

// ---- Register ----

/// The job's token hash and state, for a job that may still register (one that hasn't ended).
fn registration_target(state: &NodeState, job_id: &str) -> Option<(String, JobState)> {
    let table = lock_jobs(state);
    let record = find_record(&table, job_id)?;
    let job_state = record.info.state();
    if is_terminal_job_state(job_state) || record.token_hash.is_empty() {
        return None;
    }
    Some((record.token_hash.clone(), job_state))
}

/// `WorkerService.Hello` on the job socket, with the token. The worker must answer for this job.
#[allow(clippy::result_large_err)]
async fn hello_worker(channel: &Channel, job_token: &str, job_id: &str) -> Result<WorkerInfo, Diagnostic> {
    let couldnt_start = |reason: String| diagnostic("RRS-J2001", &[("job_id", job_id), ("reason", &reason)]);
    let request = request_with_job_token(HelloRequest {}, job_token).map_err(couldnt_start)?;
    let mut client = WorkerServiceClient::new(channel.clone());
    let response = client
        .hello(request)
        .await
        .map_err(|status| couldnt_start(format!("its Hello failed: {}", status.message())))?;
    let worker = response.into_inner();
    if worker.job_id != job_id {
        return Err(couldnt_start(format!(
            "its socket answered for job {:?}",
            worker.job_id
        )));
    }
    Ok(worker)
}

/// A worker that speaks another protocol is sent TERMINATE, which every version honors with a checkpoint (contracts
/// 9.2), and the job keeps `RRS-J2003` as the reason it ended.
async fn refuse_worker_protocol(state: &NodeState, job_id: &str, worker_version: u32) -> Diagnostic {
    let version = worker_version.to_string();
    let refusal = diagnostic("RRS-J2003", &[("job_id", job_id), ("version", &version)]);
    let process = {
        let mut table = lock_jobs(state);
        let Some(record) = find_record_mut(&mut table, job_id) else {
            return refusal;
        };
        record.start_refusal = Some(refusal.clone());
        record.process.clone()
    };
    let Some(process) = process else {
        return refusal;
    };
    if let Err(reason) = signal_process_group(&process, GroupSignal::Terminate).await {
        log_warn(
            "a worker of another protocol couldn't be stopped",
            &[("job_id", job_id), ("reason", &reason)],
        );
    }
    refusal
}

// ---- What the event-log tail and the log follower ask ----

/// True while the run has a writer that may still append to its event log (run_events.rs): a job in the table that
/// holds the run, or a `job.json` with no `final_state` whose process is still the one it names, such as a job from
/// before a node restart that nothing has reattached yet (M1-45). It may read that small file, so it is called off the
/// runtime's threads.
pub(crate) fn run_has_live_writer(state: &NodeState, run_id: &str) -> bool {
    let held_in_table = {
        let table = lock_jobs(state);
        table
            .records
            .iter()
            .any(|record| record.holds_run && record.spec.run_id == run_id)
    };
    held_in_table || job_file_is_live(&job_file_path(&run_dir(&state.paths, run_id)))
}

/// True once the job has ended, or the table no longer holds it.
pub(crate) fn job_has_ended(state: &NodeState, job_id: &str) -> bool {
    job_info(state, job_id).is_none_or(|info| is_terminal_job_state(info.state()))
}

// ---- Small table helpers ----

fn lock_jobs(state: &NodeState) -> MutexGuard<'_, JobTable> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state.jobs.lock().unwrap_or_else(PoisonError::into_inner)
}

fn find_record<'a>(table: &'a JobTable, job_id: &str) -> Option<&'a JobRecord> {
    table.records.iter().find(|record| record.info.job_id == job_id)
}

fn find_record_mut<'a>(table: &'a mut JobTable, job_id: &str) -> Option<&'a mut JobRecord> {
    table.records.iter_mut().find(|record| record.info.job_id == job_id)
}

fn job_info(state: &NodeState, job_id: &str) -> Option<JobInfo> {
    let table = lock_jobs(state);
    find_record(&table, job_id).map(|record| record.info.clone())
}

fn record_token_hash(state: &NodeState, job_id: &str, token_hash: &str) {
    let mut table = lock_jobs(state);
    if let Some(record) = find_record_mut(&mut table, job_id) {
        record.token_hash = token_hash.to_string();
    }
}

fn record_process(
    state: &NodeState,
    job_id: &str,
    identity: &ProcessIdentity,
    stdout_start_offset: u64,
    stderr_start_offset: u64,
) {
    let oom_watch = start_oom_watch(identity.pid);
    let mut table = lock_jobs(state);
    if let Some(record) = find_record_mut(&mut table, job_id) {
        record.info.pid = identity.pid;
        record.info.process_group_id = identity.process_group_id;
        record.info.started_unix_ms = now_unix_ms();
        record.process = Some(identity.clone());
        record.stdout_start_offset = stdout_start_offset;
        record.stderr_start_offset = stderr_start_offset;
        record.oom_watch = oom_watch;
    }
}

/// Stores the worker's link and moves a job that is still starting to `running`, under one hold of the table. The
/// state is read under that same hold, not before the `Hello` call: a `PAUSE` that lands while a second `Register`
/// waits on `Hello` must not be undone by it.
fn attach_worker(state: &NodeState, job_id: &str, link: WorkerLink) {
    let mut table = lock_jobs(state);
    let Some(record) = find_record_mut(&mut table, job_id) else {
        return;
    };
    record.worker = Some(link);
    if record.info.state() == JobState::Starting {
        record.info.state = JobState::Running as i32;
    }
}

fn failed_outcome(found: Diagnostic) -> JobOutcome {
    JobOutcome {
        state: JobState::Failed,
        reason: String::new(),
        diagnostic: Some(found),
    }
}

/// The kind's name in contracts 9.1: `JOB_KIND_DOCTOR_SELFTEST` → `doctor_selftest`.
fn job_kind_text(kind: JobKind) -> String {
    kind.as_str_name()
        .trim_start_matches(JOB_KIND_NAME_PREFIX)
        .to_ascii_lowercase()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::jobs::exit_cause::PREEMPTED_REASON;
    use rastai_core::ids::new_id;

    /// A stand-in `bin/python` with a child in its group. `$5` is the job file (`-m rastai_worker run --job-file`).
    #[cfg(target_os = "linux")]
    const WORKER_WITH_CHILD: &str = r#"#!/bin/sh
sleep 60 &
echo $! > "$(dirname "$5")/child.pid"
exec sleep 60
"#;
    #[cfg(target_os = "linux")]
    const CHILD_PID_FILE: &str = "child.pid";
    #[cfg(target_os = "linux")]
    const EXECUTABLE_MODE: u32 = 0o755;
    #[cfg(target_os = "linux")]
    const WAIT_LIMIT: Duration = Duration::from_secs(10);
    #[cfg(target_os = "linux")]
    const POLL: Duration = Duration::from_millis(20);

    struct TestFolder {
        path: PathBuf,
    }

    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A node state in a fresh folder, with no hardware, no environments and the product's job times.
    fn test_state(name: &str) -> (TestFolder, NodeState) {
        use rastai_core::paths::DataPaths;
        use rastai_core::settings::{default_settings, node_settings, resolve_settings};
        use rastai_proto::common::v1::HardwareProfile;

        use crate::hardware::system_hardware_sources;
        use crate::identity::load_or_create_identity;
        use crate::node::{NodeMode, NodeOptions, build_node_state};

        let path = std::env::temp_dir().join(format!("rastai-supervisor-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let folder = TestFolder { path };
        let options = NodeOptions {
            mode: NodeMode::Embedded,
            paths: DataPaths {
                config: folder.path.join("config"),
                data: folder.path.join("data"),
                cache: folder.path.join("cache"),
                state: folder.path.join("state"),
                runtime: folder.path.join("run"),
            },
            settings: node_settings(&resolve_settings(&[default_settings()])),
            hardware_sources: system_hardware_sources(),
            jobs: default_job_options(Vec::new()),
        };
        let identity = load_or_create_identity(&folder.path.join("node.key")).unwrap();
        let state = build_node_state(options, identity, HardwareProfile::default());
        (folder, state)
    }

    /// An admitted job that is running and holds its run, with its run folder made.
    fn running_job(state: &NodeState) -> String {
        let spec = JobSpec {
            job_id: new_id(IdPrefix::Job).unwrap(),
            run_id: new_id(IdPrefix::Run).unwrap(),
            kind: JobKind::Train as i32,
            environment_id: "cpu".to_string(),
            ..JobSpec::default()
        };
        let mut record = new_record(state, &spec);
        record.info.state = JobState::Running as i32;
        record.holds_run = true;
        std::fs::create_dir_all(&record.run_folder).unwrap();
        lock_jobs(state).records.push(record);
        spec.job_id
    }

    fn exit_facts_for(job_id: &str, code: i32) -> ExitFacts {
        ExitFacts {
            job_id: job_id.to_string(),
            exit: ProcessExit::Code(code),
            registered: true,
            node_kill: None,
            oom_kill_rose: false,
            start_refusal: None,
            run_log: RunLogFacts::default(),
            stderr: StderrTail::default(),
            event_log_path: String::new(),
            free_bytes: None,
        }
    }

    // Exit 75 ends the job cancelled with reason preempted; the terminal job.json is written and the run released.
    #[test]
    fn a_preempted_job_ends_cancelled_with_its_reason_and_releases_its_run() {
        let (_folder, state) = test_state("preempted");
        let job_id = running_job(&state);
        let outcome = classify_exit(&exit_facts_for(&job_id, 75));
        finish_job(&state, &job_id, outcome, Some(75));
        let table = lock_jobs(&state);
        let record = find_record(&table, &job_id).unwrap();
        assert_eq!(record.info.state(), JobState::Cancelled);
        assert_eq!(record.reason, PREEMPTED_REASON);
        assert_eq!(record.info.exit_code, Some(75));
        assert!(record.info.last_error.is_none());
        assert!(!record.holds_run);
        let written = read_job_file(&job_file_path(&record.run_folder)).unwrap();
        assert_eq!(written.final_state, "cancelled");
        assert_eq!(written.job_id, job_id);
    }

    // A job ends once: a second end (a late timer, say) changes nothing.
    #[test]
    fn an_ended_job_keeps_its_first_end() {
        let (_folder, state) = test_state("ends-once");
        let job_id = running_job(&state);
        finish_job(&state, &job_id, classify_exit(&exit_facts_for(&job_id, 3)), Some(3));
        finish_job(&state, &job_id, classify_exit(&exit_facts_for(&job_id, 4)), Some(4));
        let info = list_jobs(&state)
            .into_iter()
            .find(|info| info.job_id == job_id)
            .unwrap();
        assert_eq!(info.state(), JobState::Cancelled);
        assert_eq!(info.exit_code, Some(3));
    }

    /// Writes a `diagnostic.reported` of `job_id` into the run's event log, as a worker of that run would.
    fn log_a_diagnostic_of(state: &NodeState, job_id: &str) {
        use rastai_proto::events::v1::event_payload::Payload;
        use rastai_proto::events::v1::{DiagnosticReported, EventPayload, EventRecord};
        use rastai_store::event_log::writer::{SyncPolicy, append_event, create_event_log};

        let run_id = job_info(state, job_id).unwrap().run_id;
        let mut writer = create_event_log(&run_events_log_path(&state.paths, &run_id)).unwrap();
        let reported = DiagnosticReported {
            diagnostic: Some(diagnostic_to_proto(&diagnostic(
                "RRS-T3001",
                &[("run_id", &run_id), ("count", "3")],
            ))),
            subject_id: String::new(),
        };
        let mut record = EventRecord {
            run_id,
            job_id: job_id.to_string(),
            event_type: "diagnostic.reported".to_string(),
            payload: Some(EventPayload {
                payload: Some(Payload::DiagnosticReported(reported)),
            }),
            ..EventRecord::default()
        };
        append_event(&mut writer, &mut record, SyncPolicy::Now).unwrap();
    }

    // Exit 5: the log's frames are the running copy's, so nothing is read from it, even a record under this job's id.
    #[test]
    fn exit_five_reads_nothing_from_the_run_log() {
        let (_folder, state) = test_state("exit-five");
        let job_id = running_job(&state);
        log_a_diagnostic_of(&state, &job_id);
        let after_exit_one = exit_facts(&state, &job_id, ProcessExit::Code(1)).unwrap();
        assert!(after_exit_one.run_log.last_diagnostic.is_some(), "exit 1 reads the log");
        let after_exit_five = exit_facts(&state, &job_id, ProcessExit::Code(5)).unwrap();
        assert_eq!(after_exit_five.run_log, RunLogFacts::default());
    }

    // Two ends racing past the first check: the first one applied stays.
    #[test]
    fn a_second_terminal_outcome_changes_nothing() {
        let (_folder, state) = test_state("second-outcome");
        let job_id = running_job(&state);
        let preempted = classify_exit(&exit_facts_for(&job_id, 75));
        let disk_full = classify_exit(&exit_facts_for(&job_id, 4));
        apply_terminal_outcome(&state, &job_id, &preempted, Some(75));
        apply_terminal_outcome(&state, &job_id, &disk_full, Some(4));
        let info = job_info(&state, &job_id).unwrap();
        assert_eq!(info.state(), JobState::Cancelled);
        assert_eq!(info.exit_code, Some(75));
        assert!(info.last_error.is_none());
    }

    fn ended_record(number: usize) -> JobRecord {
        JobRecord {
            info: JobInfo {
                job_id: format!("job_{number}"),
                state: JobState::Succeeded as i32,
                ..JobInfo::default()
            },
            spec: JobSpec::default(),
            run_folder: PathBuf::new(),
            token_hash: String::new(),
            worker: None,
            reason: String::new(),
            permissions: JobPermissions::default(),
            process: None,
            stdout_start_offset: 0,
            stderr_start_offset: 0,
            oom_watch: None,
            node_kill: None,
            start_refusal: None,
            holds_run: false,
        }
    }

    #[test]
    fn only_the_oldest_ended_jobs_are_forgotten() {
        let mut table = JobTable::default();
        for number in 0..ENDED_JOBS_KEPT + 3 {
            table.records.push(ended_record(number));
        }
        let mut live = ended_record(9999);
        live.info.state = JobState::Running as i32;
        live.holds_run = true;
        table.records.insert(0, live);
        forget_old_ended_jobs(&mut table);
        assert_eq!(table.records.len(), ENDED_JOBS_KEPT + 1);
        assert_eq!(table.records[0].info.job_id, "job_9999");
        assert_eq!(table.records[1].info.job_id, "job_3");
    }

    /// An environment folder whose `bin/python` is `script`.
    #[cfg(target_os = "linux")]
    fn stand_in_environment(folder: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let environment_folder = folder.join("env");
        std::fs::create_dir_all(environment_folder.join("bin")).unwrap();
        let python = environment_folder.join("bin/python");
        std::fs::write(&python, script).unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
        environment_folder
    }

    #[cfg(target_os = "linux")]
    async fn wait_for_pid_file(path: &Path) -> u32 {
        let started = std::time::Instant::now();
        loop {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
            assert!(started.elapsed() < WAIT_LIMIT, "{} didn't appear", path.display());
            tokio::time::sleep(POLL).await;
        }
    }

    /// Gone, or a zombie nobody has reaped yet: either way it no longer runs.
    #[cfg(target_os = "linux")]
    fn process_has_stopped(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
            Err(_) => true,
        }
    }

    // The rewrite of job.json after the spawn fails: the worker and its child are killed and waited on, so nothing
    // runs on once the job has failed and released its run.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_worker_whose_job_file_cant_be_rewritten_is_killed_with_its_group() {
        let (folder, state) = test_state("rewrite-fails");
        let job_id = running_job(&state);
        let run_folder = folder.path.join("run");
        std::fs::create_dir_all(&run_folder).unwrap();
        let launch = WorkerLaunch {
            environment_folder: stand_in_environment(&folder.path, WORKER_WITH_CHILD),
            job_file: job_file_path(&run_folder),
            run_folder: run_folder.clone(),
            variables: node_variables()
                .into_iter()
                .filter(|(name, _)| name == "PATH")
                .collect(),
        };
        let spawned = spawn_worker(&launch).await.unwrap();
        let leader = spawned.identity.clone();
        let child_pid = wait_for_pid_file(&run_folder.join(CHILD_PID_FILE)).await;

        // A run folder that isn't there makes the rewrite fail.
        let missing_folder = folder.path.join("no-run-folder");
        let recorded = record_spawned_worker(&state, &job_id, &missing_folder, JobFile::default(), spawned).await;
        assert!(recorded.is_err());
        assert!(!is_same_process(&leader), "the worker was killed and waited on");
        let started = std::time::Instant::now();
        while !process_has_stopped(child_pid) {
            assert!(
                started.elapsed() < WAIT_LIMIT,
                "the worker's child {child_pid} still runs"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// A link to a worker that is never dialed: a lazy channel connects only on its first call.
    fn unconnected_link(job_token: &str) -> WorkerLink {
        let channel = tonic::transport::Endpoint::from_static("http://localhost").connect_lazy();
        WorkerLink {
            job_token: job_token.to_string(),
            channel,
        }
    }

    fn set_state_in_table(state: &NodeState, job_id: &str, to: JobState) {
        let mut table = lock_jobs(state);
        find_record_mut(&mut table, job_id).unwrap().info.state = to as i32;
    }

    // A worker that registers again with the right token (it reconnected) gets its link replaced, but a paused job
    // stays paused; only a starting job moves to running.
    #[tokio::test]
    async fn a_second_register_keeps_a_paused_job_paused() {
        let (_folder, state) = test_state("register-paused");
        let job_id = running_job(&state);
        set_state_in_table(&state, &job_id, JobState::Paused);
        attach_worker(&state, &job_id, unconnected_link(&"cd".repeat(32)));
        assert_eq!(job_info(&state, &job_id).unwrap().state(), JobState::Paused);
        let table = lock_jobs(&state);
        let link = find_record(&table, &job_id).unwrap().worker.as_ref().unwrap();
        assert_eq!(link.job_token, "cd".repeat(32), "the new link replaced the old one");
        drop(table);

        set_state_in_table(&state, &job_id, JobState::Starting);
        attach_worker(&state, &job_id, unconnected_link(&"cd".repeat(32)));
        assert_eq!(job_info(&state, &job_id).unwrap().state(), JobState::Running);
    }

    // The token lets whoever holds it act as the job's worker, so {:?} of the link, or of a record holding it, never
    // prints it.
    #[tokio::test]
    async fn debug_never_prints_a_workers_token() {
        let token = "ef".repeat(32);
        let link = unconnected_link(&token);
        let mut record = ended_record(1);
        record.worker = Some(link.clone());
        for text in [format!("{link:?}"), format!("{link:#?}"), format!("{record:?}")] {
            assert!(!text.contains(&token), "{text}");
            assert!(text.contains(REDACTED_TOKEN), "{text}");
        }
    }

    #[test]
    fn kind_names_are_contracts_names() {
        assert_eq!(job_kind_text(JobKind::DoctorSelftest), "doctor_selftest");
        assert_eq!(job_kind_text(JobKind::Train), "train");
    }

    #[test]
    fn a_spec_with_a_path_for_an_id_is_refused() {
        let spec = JobSpec {
            job_id: "job_01JA2B3C4D5E6F7G8H9J0K1M2Q".to_string(),
            run_id: "../../etc".to_string(),
            ..JobSpec::default()
        };
        let problem = id_problem(&spec).unwrap_or_default();
        assert!(problem.contains("run id"), "{problem}");
    }
}
