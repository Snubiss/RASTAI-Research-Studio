//! The `job` commands (contracts 6.4 and 9.1): `job.list`, `job.get` and `job.cancel`. Jobs are rows of `jobs`,
//! kept in step with the nodes by the node links (nodes/reconcile.rs).
//!
//! `job.cancel` checks two scopes: its own row's (`training`, checked before the handler, flows/platform.md 3.2) and
//! the scope of the command that started the job, so a caller can't cancel a kind of job it couldn't start. A queued
//! job is cancelled on the spot; a started one is sent `CANCEL` and stops at its next step boundary, without a new
//! checkpoint (contracts 4.4).

use rastai_core::commands::output::CommandOutput;
use rastai_core::commands::registry::{check_scope, find_command};
use rastai_core::principal::{PrincipalKind, principal_label};
use rastai_core::scopes::{Scope, scope_name, scopes_allow};
use rastai_core::states::{is_terminal_job_state, job_state_name, parse_job_state};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::commands::job::{
    JobCancelArgs, JobCancelResult, JobGetArgs, JobGetResult, JobListArgs, JobListResult, JobSummary,
};
use rastai_formats::commands::result_json;
use rastai_proto::common::v1::JobState;
use rastai_proto::node::v1::JobSignal;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{JobFilter, JobRow, get_job, list_jobs};

use crate::commands::context::{CommandContext, store_failure, sync_result};
use crate::nodes::node_links::signal_job_on_node;
use crate::nodes::reconcile::cancel_queued_job;
use crate::server_state::Server;

const CODE_NO_SUCH_JOB: &str = "RRS-J0005";
const CODE_SIGNAL_NOT_DELIVERED: &str = "RRS-J0004";
const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
const CANCEL_COMMAND_ID: &str = "job.cancel";
const CANCEL_SIGNAL_NAME: &str = "CANCEL";
const JOB_ENDED_REASON: &str = "the job has already ended";
/// The command that starts each job kind (contracts 6.4). Cancelling a job needs that command's scope too. A kind
/// not listed here is one no command of this build starts, so only a caller with `system.admin` may cancel it.
const STARTING_COMMANDS: &[(&str, &str)] = &[("train", "train.start")];

/// The jobs that match the filter, newest first.
pub(crate) async fn list(
    context: &CommandContext<'_>,
    arguments: JobListArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let filter = JobFilter {
        run_id: arguments.run_id,
        state: arguments.state,
        node_id: String::new(),
    };
    let rows = read_jobs(context.server, &filter)?;
    let result = JobListResult {
        jobs: rows.iter().map(job_summary).collect(),
    };
    Ok(sync_result(result_json(&result)))
}

/// One job; `RRS-J0005` for an id the daemon doesn't know.
pub(crate) async fn get(context: &CommandContext<'_>, arguments: JobGetArgs) -> Result<CommandOutput, Vec<Diagnostic>> {
    let row = find_job(context.server, &arguments.job_id)?;
    let result = JobGetResult { job: job_summary(&row) };
    Ok(sync_result(result_json(&result)))
}

/// Cancels a job (see the top). The answer is the job right after the request: `cancelled` when it hadn't started,
/// else `stopping` until it exits. An ended job can't be cancelled (`RRS-J0004`). A dry run checks and answers the
/// job as it is.
pub(crate) async fn cancel(
    context: &CommandContext<'_>,
    arguments: JobCancelArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let row = find_job(context.server, &arguments.job_id)?;
    check_starting_scope(context, &row.kind)?;
    let state = parse_job_state(&row.state).unwrap_or(JobState::Unspecified);
    if is_terminal_job_state(state) {
        return Err(vec![signal_refused(&row.job_id, JOB_ENDED_REASON)]);
    }
    if context.dry_run {
        return Ok(cancel_answer(job_summary(&row)));
    }
    let mut summary = job_summary(&row);
    if state == JobState::Queued && cancel_queued_job(context.server, &row.job_id).await {
        summary.state = job_state_name(JobState::Cancelled).to_string();
        return Ok(cancel_answer(summary));
    }
    signal_job_on_node(context.server, &row.node_id, &row.job_id, JobSignal::Cancel)
        .await
        .map_err(|problem| vec![problem])?;
    // The node moved the job to stopping when it took the signal (contracts 9.1); the row follows with the next
    // heartbeat.
    summary.state = job_state_name(JobState::Stopping).to_string();
    Ok(cancel_answer(summary))
}

/// A `jobs` row as the commands return it (contracts 6.5).
pub(crate) fn job_summary(row: &JobRow) -> JobSummary {
    JobSummary {
        job_id: row.job_id.clone(),
        run_id: row.run_id.clone(),
        node_id: row.node_id.clone(),
        kind: row.kind.clone(),
        state: row.state.clone(),
        error_code: row.error_code.clone(),
    }
}

/// The jobs that match `filter`, newest first.
pub(crate) fn read_jobs(server: &Server, filter: &JobFilter) -> Result<Vec<JobRow>, Vec<Diagnostic>> {
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    list_jobs(&reader, filter).map_err(|error| store_failure(&error))
}

fn find_job(server: &Server, job_id: &str) -> Result<JobRow, Vec<Diagnostic>> {
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    match get_job(&reader, job_id).map_err(|error| store_failure(&error))? {
        Some(row) => Ok(row),
        None => Err(vec![diagnostic(CODE_NO_SUCH_JOB, &[("job_id", job_id)])]),
    }
}

/// The caller must hold the scope of the command that started a job of this kind (contracts 6.4).
fn check_starting_scope(context: &CommandContext<'_>, kind: &str) -> Result<(), Vec<Diagnostic>> {
    let starting = STARTING_COMMANDS.iter().find(|(job_kind, _)| *job_kind == kind);
    let registry = match context.server.registry.read() {
        Ok(registry) => registry,
        // A panic while the registry changed leaves it whole (each change is a push or a retain).
        Err(poisoned) => poisoned.into_inner(),
    };
    let command = starting.and_then(|(_, command_id)| find_command(&registry, command_id));
    match command {
        Some(command) => check_scope(context.principal, command).map_err(|refusal| vec![refusal]),
        None => check_system_admin(context),
    }
}

/// The fallback of `check_starting_scope` for a kind no command of this build starts.
fn check_system_admin(context: &CommandContext<'_>) -> Result<(), Vec<Diagnostic>> {
    let principal = context.principal;
    if principal.kind == PrincipalKind::Owner || scopes_allow(&principal.scopes, &Scope::SystemAdmin) {
        return Ok(());
    }
    let label = principal_label(principal);
    let scope = scope_name(&Scope::SystemAdmin);
    let values = [
        ("command_id", CANCEL_COMMAND_ID),
        ("scope", scope.as_str()),
        ("principal", label.as_str()),
    ];
    Err(vec![diagnostic(CODE_SCOPE_NOT_GRANTED, &values)])
}

fn signal_refused(job_id: &str, reason: &str) -> Diagnostic {
    let values = [("signal", CANCEL_SIGNAL_NAME), ("job_id", job_id), ("reason", reason)];
    diagnostic(CODE_SIGNAL_NOT_DELIVERED, &values)
}

fn cancel_answer(job: JobSummary) -> CommandOutput {
    sync_result(result_json(&JobCancelResult { job }))
}
