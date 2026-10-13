//! Keeping the daemon's job and run rows in step with a node (layout: rust-control-plane.md 8.1; flows/platform.md
//! 2.3, 5.2 and 5.4; contracts 9.1 and 9.5).
//!
//! - **After a connect**, `reconcile_node` starts the tail of every open run on the node (each claimed before the
//!   next step), then compares `ListJobs` with the `jobs` rows. A live job the node no longer knows ended while no
//!   node watched and nothing recorded its end: it fails with `RRS-J3002`.
//! - **Job states come from the node** (its `ListJobs` and heartbeats); they are in no log (contracts 7.3). Each
//!   difference from a `jobs` row becomes one `job.state_changed` record in the daemon log, which indexes the row.
//!   A row in a final state never moves again.
//! - **A run's end, written by the daemon** (contracts 9.5, "Who writes the final state"): when a job ends without
//!   the worker's final record (a `KILL`, a SIGKILL from elsewhere, a crash before the first record, a spawn error,
//!   `job.cancel` of a queued job), the daemon records the run `cancelled` or `failed` in its own log. It does so only
//!   once the job's state is final and the run's tail has read the log to its end (`settle_run` runs when the tail
//!   ends, and skips a run that still has one), only when the run's indexed state hasn't already ended, and only along
//!   a move 9.5 allows. A job that failed with `RRS-J3006` gets nothing: the run belongs to the copy still running.
//! - **One at a time.** Every comparison and record here happens under `Scheduler.job_state_lock`, so a heartbeat, a
//!   launch, a cancel and a settle never record one move twice.

use std::sync::Arc;

use rastai_core::states::{
    is_terminal_job_state, job_state_name, parse_job_state, parse_run_state, run_transition_allowed,
};
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_proto::common::v1::{JobState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{EventPayload, EventRecord, JobStateChanged, RunStateChanged};
use rastai_proto::node::v1::JobInfo;
use rastai_store::error::StoreError;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{JobFilter, JobRow, RunRow, get_run, list_jobs};

use crate::events::daemon_events::record_daemon_event;
use crate::indexing::run_tail::{open_run_ids, spawn_run_tail};
use crate::nodes::node_links::{list_jobs_on_node, run_event_source};
use crate::scheduler::queue::wake_scheduler;
use crate::server_state::Server;

const JOB_STATE_CHANGED_EVENT: &str = "job.state_changed";
const RUN_STATE_CHANGED_EVENT: &str = "run.state_changed";
/// The worker's exit after SIGTERM (contracts 9.1): the job is `cancelled`, reason `preempted`.
const EXIT_PREEMPTED: i32 = 75;
const PREEMPTED_REASON: &str = "preempted";
const CODE_GONE_WITHOUT_RECORD: &str = "RRS-J3002";
/// Another worker writes the run's log (contracts 9.1): the run's state is that worker's, never this job's.
const CODE_EVENT_LOG_LOCKED: &str = "RRS-J3006";
/// The job states of a job that runs on its node: the node must list it.
const LIVE_JOB_STATES: &[JobState] = &[
    JobState::Starting,
    JobState::Running,
    JobState::Paused,
    JobState::Stopping,
    JobState::Lost,
];
/// Run states after which a job has nothing left to write: a final state, or preempted (resumable, contracts 9.5).
const RUN_STATES_THAT_END_A_JOB: &[RunState] = &[
    RunState::Completed,
    RunState::Failed,
    RunState::Cancelled,
    RunState::Preempted,
];

/// How much of a node's jobs a snapshot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotScope {
    /// `ListJobs`: every job the node knows, so a live job it doesn't list is gone.
    EveryJob,
    /// A heartbeat or a launch: the jobs it names.
    TheseJobs,
}

/// After a connect: tail every open run on the node, then take the node's job states (see the top).
pub async fn reconcile_node(server: &Arc<Server>, node_id: &str) {
    let listed = list_jobs_on_node(server, node_id).await;
    tail_open_runs(server, node_id);
    match listed {
        Ok(jobs) => apply_job_snapshot(server, node_id, &jobs, SnapshotScope::EveryJob).await,
        Err(problem) => log_warn(
            "a node's jobs couldn't be listed; the next heartbeat brings them",
            &[("node_id", node_id), ("code", &problem.code)],
        ),
    }
    wake_scheduler(server);
}

/// Records every job state the snapshot changes, then settles the runs whose jobs ended.
pub async fn apply_job_snapshot(server: &Arc<Server>, node_id: &str, jobs: &[JobInfo], scope: SnapshotScope) {
    let ended_runs = {
        let _held = server.scheduler.job_state_lock.lock().await;
        record_job_changes(server, node_id, jobs, scope).await
    };
    settle_runs_later(server, ended_runs);
}

/// The records of `apply_job_snapshot`, for a caller that already holds `Scheduler.job_state_lock` (the launch).
/// Returns the runs whose job just reached a final state.
pub async fn record_job_changes(server: &Server, node_id: &str, jobs: &[JobInfo], scope: SnapshotScope) -> Vec<String> {
    let rows = match jobs_on_node(server, node_id) {
        Ok(rows) => rows,
        Err(error) => {
            log_warn("a node's job rows can't be read", &[("code", error.code())]);
            return Vec::new();
        }
    };
    let mut changes = Vec::new();
    for info in jobs {
        let row = rows.iter().find(|row| row.job_id == info.job_id);
        if let Some(change) = row.and_then(|row| job_change(row, info)) {
            changes.push((row.map(|row| row.run_id.clone()).unwrap_or_default(), change));
        }
    }
    if scope == SnapshotScope::EveryJob {
        for row in rows.iter().filter(|row| is_vanished(row, jobs)) {
            changes.push((row.run_id.clone(), vanished_job_change(row)));
        }
    }
    record_changes(server, changes).await
}

/// The move from a `jobs` row to what the node reports, or None when there is nothing to record: the same state, a
/// row already final, or a state this build doesn't know.
pub fn job_change(row: &JobRow, info: &JobInfo) -> Option<JobStateChanged> {
    let from = parse_job_state(&row.state)?;
    let to = JobState::try_from(info.state).ok()?;
    if from == to || is_terminal_job_state(from) || to == JobState::Unspecified {
        return None;
    }
    let reason = match info.exit_code {
        Some(EXIT_PREEMPTED) => PREEMPTED_REASON.to_string(),
        _ => String::new(),
    };
    Some(JobStateChanged {
        job_id: info.job_id.clone(),
        from_state: from as i32,
        to_state: to as i32,
        reason,
        exit_code: info.exit_code,
        diagnostic: info.last_error.clone(),
    })
}

/// A node that stopped answering: its running jobs are lost until it is back (contracts 9.1, running → lost).
pub async fn mark_running_jobs_lost(server: &Arc<Server>, node_id: &str) {
    let _held = server.scheduler.job_state_lock.lock().await;
    let rows = match jobs_on_node(server, node_id) {
        Ok(rows) => rows,
        Err(error) => {
            log_warn("a node's job rows can't be read", &[("code", error.code())]);
            return;
        }
    };
    let mut changes = Vec::new();
    for row in rows.iter().filter(|row| row.state == job_state_name(JobState::Running)) {
        changes.push((row.run_id.clone(), moved(row, JobState::Lost, None)));
    }
    record_changes(server, changes).await;
}

/// A queued job that can't be launched (its `run.json` is missing or damaged) fails with the reason.
pub async fn fail_unlaunchable_job(server: &Arc<Server>, job: &JobRow, problem: Diagnostic) {
    log_warn(
        "a queued job can't be launched",
        &[
            ("job_id", &job.job_id),
            ("code", &problem.code),
            ("message", &problem.message),
        ],
    );
    let ended_runs = {
        let _held = server.scheduler.job_state_lock.lock().await;
        let change = moved(job, JobState::Failed, Some(&problem));
        record_changes(server, vec![(job.run_id.clone(), change)]).await
    };
    settle_runs_later(server, ended_runs);
}

/// `job.cancel` of a job that hasn't started: no process, so no signal (contracts 9.1, queued → cancelled), and the
/// run, which no record reached, ends `cancelled` too. False when the job left the queue meanwhile; the caller then
/// signals the node.
pub async fn cancel_queued_job(server: &Server, job_id: &str) -> bool {
    let ended_runs = {
        let _held = server.scheduler.job_state_lock.lock().await;
        let Some(row) = read_job(server, job_id) else {
            return false;
        };
        if row.state != job_state_name(JobState::Queued) {
            return false;
        }
        let change = moved(&row, JobState::Cancelled, None);
        record_changes(server, vec![(row.run_id.clone(), change)]).await
    };
    for run_id in ended_runs {
        settle_run(server, &run_id).await;
    }
    true
}

/// Tails the run, then settles it once the tail has read its log to the end. Nothing happens when the run already
/// has a tail: that one settles it.
pub fn tail_then_settle(server: &Arc<Server>, node_id: &str, run_id: &str) {
    let Some(source) = run_event_source(server, node_id) else {
        return;
    };
    let Some(tail) = spawn_run_tail(server, &source, run_id) else {
        return;
    };
    let server = server.clone();
    let run_id = run_id.to_string();
    tokio::spawn(async move {
        let _ = tail.await;
        settle_run(&server, &run_id).await;
    });
}

/// The daemon-written end of a run whose job ended without one (see the top). Safe to call at any time and twice:
/// it writes only what is missing.
pub async fn settle_run(server: &Server, run_id: &str) {
    if run_id.is_empty() || *server.shutdown.borrow() {
        return;
    }
    let _held = server.scheduler.job_state_lock.lock().await;
    if run_has_tail(server, run_id) {
        return;
    }
    let Some((run, job)) = run_and_newest_job(server, run_id) else {
        return;
    };
    let (Some(current), Some(job_state)) = (parse_run_state(&run.state), parse_job_state(&job.state)) else {
        return;
    };
    let Some(end) = daemon_written_run_end(current, job_state, &job.error_code) else {
        return;
    };
    let change = RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(end as i32)),
    };
    let record = EventRecord {
        run_id: run_id.to_string(),
        job_id: job.job_id.clone(),
        event_type: RUN_STATE_CHANGED_EVENT.to_string(),
        payload: Some(EventPayload {
            payload: Some(Payload::RunStateChanged(change)),
        }),
        ..EventRecord::default()
    };
    if let Err(error) = record_daemon_event(server, record).await {
        log_warn(
            "a run's end couldn't be recorded",
            &[("run_id", run_id), ("code", error.code())],
        );
    }
}

/// The run state the daemon writes for a run whose job ended in `job_state`, or None: the run already ended, the job
/// didn't end badly (a succeeded job's worker wrote `completed` itself), the run belongs to another writer
/// (`RRS-J3006`), or contracts 9.5 has no such move.
pub fn daemon_written_run_end(current: RunState, job_state: JobState, error_code: &str) -> Option<RunState> {
    if RUN_STATES_THAT_END_A_JOB.contains(&current) {
        return None;
    }
    let end = match job_state {
        JobState::Cancelled => RunState::Cancelled,
        JobState::Failed if error_code != CODE_EVENT_LOG_LOCKED => RunState::Failed,
        _ => return None,
    };
    run_transition_allowed(current, end).then_some(end)
}

// ---- Helpers ----

fn tail_open_runs(server: &Arc<Server>, node_id: &str) {
    match open_run_ids(server, node_id) {
        Ok(run_ids) => {
            for run_id in run_ids {
                tail_then_settle(server, node_id, &run_id);
            }
        }
        Err(error) => log_warn(
            "the open runs can't be listed; no run is tailed",
            &[("node_id", node_id), ("code", error.code())],
        ),
    }
}

/// Settles each run on a task of its own, after the caller let go of the job lock.
pub fn settle_runs_later(server: &Arc<Server>, run_ids: Vec<String>) {
    if run_ids.is_empty() {
        return;
    }
    for run_id in run_ids {
        let server = server.clone();
        tokio::spawn(async move { settle_run(&server, &run_id).await });
    }
    wake_scheduler(server);
}

/// Records each change; returns the runs whose job reached a final state.
async fn record_changes(server: &Server, changes: Vec<(String, JobStateChanged)>) -> Vec<String> {
    let mut ended_runs = Vec::new();
    for (run_id, change) in changes {
        let ended = JobState::try_from(change.to_state).is_ok_and(is_terminal_job_state);
        let record = EventRecord {
            run_id: run_id.clone(),
            job_id: change.job_id.clone(),
            event_type: JOB_STATE_CHANGED_EVENT.to_string(),
            payload: Some(EventPayload {
                payload: Some(Payload::JobStateChanged(change)),
            }),
            ..EventRecord::default()
        };
        match record_daemon_event(server, record).await {
            Ok(()) if ended && !run_id.is_empty() => ended_runs.push(run_id),
            Ok(()) => {}
            Err(error) => log_warn("a job's state change couldn't be recorded", &[("code", error.code())]),
        }
    }
    ended_runs
}

/// A live job (9.1) that a full snapshot doesn't list.
fn is_vanished(row: &JobRow, listed: &[JobInfo]) -> bool {
    let live = parse_job_state(&row.state).is_some_and(|state| LIVE_JOB_STATES.contains(&state));
    live && !listed.iter().any(|info| info.job_id == row.job_id)
}

fn vanished_job_change(row: &JobRow) -> JobStateChanged {
    let gone = diagnostic(CODE_GONE_WITHOUT_RECORD, &[("job_id", &row.job_id)]);
    moved(row, JobState::Failed, Some(&gone))
}

/// The row's job moved by the daemon itself, with the reason when there is one.
fn moved(row: &JobRow, to: JobState, problem: Option<&Diagnostic>) -> JobStateChanged {
    let from = parse_job_state(&row.state).unwrap_or(JobState::Unspecified);
    JobStateChanged {
        job_id: row.job_id.clone(),
        from_state: from as i32,
        to_state: to as i32,
        reason: String::new(),
        exit_code: None,
        diagnostic: problem.map(diagnostic_to_proto),
    }
}

fn jobs_on_node(server: &Server, node_id: &str) -> Result<Vec<JobRow>, StoreError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    let filter = JobFilter {
        node_id: node_id.to_string(),
        ..JobFilter::default()
    };
    list_jobs(&reader, &filter)
}

/// The job's row as it is now; None when it can't be read.
pub fn read_job(server: &Server, job_id: &str) -> Option<JobRow> {
    let reader = open_index_reader(&server.store.index_readers).ok()?;
    rastai_store::index::runs::get_job(&reader, job_id).ok().flatten()
}

/// The run's row and its newest job, the one whose end decides the run's.
fn run_and_newest_job(server: &Server, run_id: &str) -> Option<(RunRow, JobRow)> {
    let reader = open_index_reader(&server.store.index_readers).ok()?;
    let run = get_run(&reader, run_id).ok().flatten()?;
    let filter = JobFilter {
        run_id: run_id.to_string(),
        ..JobFilter::default()
    };
    let newest = list_jobs(&reader, &filter).ok()?.into_iter().next()?;
    Some((run, newest))
}

fn run_has_tail(server: &Server, run_id: &str) -> bool {
    let tailed = server
        .tailed_run_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    tailed.contains(run_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn row(state: &str) -> JobRow {
        JobRow {
            job_id: "job_a".to_string(),
            run_id: "run_a".to_string(),
            node_id: "node_a".to_string(),
            kind: "train".to_string(),
            state: state.to_string(),
            pid: None,
            process_group_id: None,
            exit_code: None,
            error_code: String::new(),
        }
    }

    fn info(state: JobState, exit_code: Option<i32>) -> JobInfo {
        JobInfo {
            job_id: "job_a".to_string(),
            state: state as i32,
            exit_code,
            ..JobInfo::default()
        }
    }

    #[test]
    fn a_job_moves_only_when_its_state_changes_and_never_out_of_an_end() {
        let started = job_change(&row("queued"), &info(JobState::Starting, None)).unwrap();
        assert_eq!(
            (started.from_state, started.to_state),
            (JobState::Queued as i32, JobState::Starting as i32)
        );
        assert_eq!(job_change(&row("running"), &info(JobState::Running, None)), None);
        assert_eq!(job_change(&row("cancelled"), &info(JobState::Running, None)), None);
        assert_eq!(job_change(&row("running"), &info(JobState::Unspecified, None)), None);
        let preempted = job_change(&row("stopping"), &info(JobState::Cancelled, Some(75))).unwrap();
        assert_eq!(preempted.reason, "preempted");
        assert_eq!(preempted.exit_code, Some(75));
    }

    #[test]
    fn only_a_live_job_the_node_no_longer_lists_has_vanished() {
        let listed = [info(JobState::Running, None)];
        assert!(!is_vanished(&row("running"), &listed));
        assert!(is_vanished(&row("running"), &[]));
        assert!(is_vanished(&row("lost"), &[]));
        assert!(!is_vanished(&row("queued"), &[]), "a queued job isn't on its node yet");
        assert!(!is_vanished(&row("succeeded"), &[]));
        let gone = vanished_job_change(&row("running"));
        assert_eq!(gone.to_state, JobState::Failed as i32);
        assert_eq!(gone.diagnostic.unwrap().code, "RRS-J3002");
    }

    // The daemon writes a run's end only for a job that ended badly, from a state that hasn't ended, along a 9.5 edge.
    #[test]
    fn the_daemon_writes_only_the_end_a_worker_could_not() {
        let cases = [
            (
                RunState::Running,
                JobState::Cancelled,
                "RRS-J1003",
                Some(RunState::Cancelled),
            ),
            (RunState::Running, JobState::Failed, "RRS-J1001", Some(RunState::Failed)),
            (RunState::Paused, JobState::Failed, "RRS-J1001", Some(RunState::Failed)),
            (
                RunState::Paused,
                JobState::Cancelled,
                "RRS-J1003",
                Some(RunState::Cancelled),
            ),
            (
                RunState::Checkpointing,
                JobState::Cancelled,
                "RRS-J1003",
                Some(RunState::Cancelled),
            ),
            (RunState::Created, JobState::Failed, "RRS-J0001", Some(RunState::Failed)),
            (RunState::Created, JobState::Cancelled, "", Some(RunState::Cancelled)),
            (
                RunState::Preparing,
                JobState::Failed,
                "RRS-J1002",
                Some(RunState::Failed),
            ),
            (RunState::Resuming, JobState::Cancelled, "", Some(RunState::Cancelled)),
            // The worker already wrote an end, or the job didn't end badly.
            (RunState::Completed, JobState::Failed, "RRS-J1004", None),
            (RunState::Preempted, JobState::Cancelled, "", None),
            (RunState::Failed, JobState::Failed, "RRS-J1001", None),
            (RunState::Running, JobState::Succeeded, "", None),
            (RunState::Running, JobState::Running, "", None),
            // Exit 5: the run's log is another job's.
            (RunState::Running, JobState::Failed, "RRS-J3006", None),
        ];
        for (current, job_state, code, expected) in cases {
            assert_eq!(
                daemon_written_run_end(current, job_state, code),
                expected,
                "{current:?} after a {job_state:?} job ({code})"
            );
        }
    }
}
