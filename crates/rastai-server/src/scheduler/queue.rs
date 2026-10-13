//! The job queue (layout: rust-control-plane.md 8.3; flows/platform.md 5.1): `submit_job` records a job `queued`,
//! and `run_scheduler` launches queued jobs, first in, first out per node, whenever something may have changed.
//!
//! - **Queued jobs are rows**, so they survive a daemon restart: `submit_job` writes the run's `run.json` (with the
//!   job's `JobSpec`), its `runs` row and the job's `jobs` row, in that order, before it answers. The launch reads the
//!   spec back from `run.json` (launch.rs), so a job queued before a restart starts after it the same way.
//! - **One loop** launches every queued job whose node is connected. It wakes on a submit, a connect and a job's
//!   end (`wake_scheduler`), and every `SCHEDULER_TICK_SECONDS` in case a wake was missed.
//! - **The job lock** (`Scheduler.job_state_lock`) is held while job states are compared and recorded and while a
//!   run's end is settled (nodes/reconcile.rs), so a heartbeat, a launch and a settle never record one move twice.
//!   It is never held across a node call, so a heartbeat never waits on a `StartJob` that takes long.
//! - **The launch lock** (`Scheduler.launch_lock`) is held by a launch from its queue check to its recorded answer,
//!   `StartJob` included (launch.rs). The cancel of a queued job and a run's settle take it before the job lock, so
//!   neither lands while a node may be starting the job.
//!
//! The job is the generated `JobSpec` (contracts 2.4 rule 3); there is no second job struct.

use std::sync::Arc;
use std::time::Duration;

use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::states::{job_kind_name, job_state_name, run_kind_name};
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::Diagnostic;
use rastai_diag::logger::log_warn;
use rastai_proto::common::v1::JobState;
use rastai_proto::node::v1::JobSpec;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{JobFilter, JobRow, RunRow, list_jobs};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use tokio::sync::Notify;

use crate::commands::context::{core_failure, store_failure};
use crate::scheduler::launch::{RunStart, launch_job, write_run_record};
use crate::scheduler::placement::node_is_ready;
use crate::server_state::Server;
use crate::shutdown::wait_until_stopping;

/// How often the loop looks at the queue without being woken: a safety net, since every change that can let a job
/// start wakes it.
const SCHEDULER_TICK_SECONDS: u64 = 5;
/// `runs.state` of a run that has been recorded but whose job hasn't written a record yet (contracts 9.5).
const RUN_CREATED_STATE: &str = "created";

/// The scheduler's shared parts, a field of `Server`.
pub struct Scheduler {
    /// Wakes `run_scheduler`: a job was submitted, a node connected, or a job ended.
    pub wake: Notify,
    /// Held while job states are compared and recorded, and while a run's end is settled (see the top).
    pub job_state_lock: tokio::sync::Mutex<()>,
    /// Held by a launch across its `StartJob`; taken before `job_state_lock` when both are needed (see the top).
    pub launch_lock: tokio::sync::Mutex<()>,
}

pub fn new_scheduler() -> Scheduler {
    Scheduler {
        wake: Notify::new(),
        job_state_lock: tokio::sync::Mutex::new(()),
        launch_lock: tokio::sync::Mutex::new(()),
    }
}

/// Asks the scheduler loop to look at the queue now. It never waits.
pub fn wake_scheduler(server: &Server) {
    server.scheduler.wake.notify_one();
}

/// Records a new job `queued` on `node_id` and returns its id, which it also writes into `spec.job_id`. With `run`,
/// the job starts a new run: its `run.json` and `runs` row (state `created`) are written first.
pub async fn submit_job(
    server: &Server,
    mut spec: JobSpec,
    node_id: &str,
    run: Option<RunStart>,
) -> Result<String, Vec<Diagnostic>> {
    spec.job_id = new_id(IdPrefix::Job).map_err(|error| core_failure(&error))?;
    let mut writes = Vec::new();
    if let Some(run) = run {
        // One reading of the clock for run.json and the row, so a rebuild gives the row this same time.
        let created_unix_ms = now_unix_ms();
        let record_hash =
            write_run_record(&server.paths, &spec, &run, created_unix_ms).map_err(|problem| vec![problem])?;
        let row = new_run_row(&spec, node_id, &run, record_hash, created_unix_ms);
        writes.push(IndexWrite::UpsertRun(row));
    }
    writes.push(IndexWrite::UpsertJob(queued_job_row(&spec, node_id)));
    write_index(&server.store.index, IndexWrite::Batch(writes))
        .await
        .map_err(|error| store_failure(&error))?;
    wake_scheduler(server);
    Ok(spec.job_id)
}

/// Launches queued jobs until the daemon stops.
pub async fn run_scheduler(server: Arc<Server>) {
    let mut shutdown = server.shutdown.subscribe();
    loop {
        launch_queued_jobs(&server).await;
        tokio::select! {
            () = server.scheduler.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(SCHEDULER_TICK_SECONDS)) => {}
            () = wait_until_stopping(&mut shutdown) => return,
        }
    }
}

/// Every queued job whose node is connected, oldest first. A job whose node isn't stays queued for the next pass.
async fn launch_queued_jobs(server: &Arc<Server>) {
    let queued = match queued_jobs(server) {
        Ok(queued) => queued,
        Err(problem) => {
            log_warn("the job queue can't be read", &[("code", &problem.code)]);
            return;
        }
    };
    for job in queued {
        if *server.shutdown.borrow() {
            return;
        }
        if node_is_ready(server, &job.node_id) {
            launch_job(server, &job).await;
        }
    }
}

/// The queued jobs, oldest first: ids begin with the time they were made (contracts 8.6).
#[allow(clippy::result_large_err)]
fn queued_jobs(server: &Server) -> Result<Vec<JobRow>, Diagnostic> {
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| first(store_failure(&error)))?;
    let filter = JobFilter {
        state: job_state_name(JobState::Queued).to_string(),
        ..JobFilter::default()
    };
    let mut jobs = list_jobs(&reader, &filter).map_err(|error| first(store_failure(&error)))?;
    jobs.reverse();
    Ok(jobs)
}

fn first(mut diagnostics: Vec<Diagnostic>) -> Diagnostic {
    diagnostics.remove(0)
}

fn new_run_row(spec: &JobSpec, node_id: &str, run: &RunStart, record_hash: String, created_unix_ms: i64) -> RunRow {
    RunRow {
        run_id: spec.run_id.clone(),
        kind: run_kind_name(run.kind),
        state: RUN_CREATED_STATE.to_string(),
        project_id: run.project_id.clone(),
        node_id: node_id.to_string(),
        parent_run_id: String::new(),
        record_hash,
        acked_sequence: 0,
        created_unix_ms,
        finished_unix_ms: None,
        pinned: false,
        // UpsertRun never writes it: a new run takes the column's default (store layout 6.4).
        archive_state: String::new(),
    }
}

fn queued_job_row(spec: &JobSpec, node_id: &str) -> JobRow {
    JobRow {
        job_id: spec.job_id.clone(),
        run_id: spec.run_id.clone(),
        node_id: node_id.to_string(),
        kind: job_kind_name(spec.kind),
        state: job_state_name(JobState::Queued).to_string(),
        pid: None,
        process_group_id: None,
        exit_code: None,
        error_code: String::new(),
    }
}
