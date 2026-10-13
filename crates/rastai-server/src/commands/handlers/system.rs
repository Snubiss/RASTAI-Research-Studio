//! The `system` commands built so far (contracts 6.4): `system.status` and `system.shutdown`. The others
//! (`system.doctor`, `system.hardware`, `system.env.*`, updates, crash reports) answer their area's "not available
//! yet" code from dispatch.rs until their milestones.

use rastai_core::commands::output::CommandOutput;
use rastai_core::states::parse_job_state;
use rastai_core::versions::{
    API_PACKAGES, APP_VERSION, NODE_PROTOCOL_VERSION, PLUGIN_API_VERSION, WORKER_PROTOCOL_VERSION,
};
use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::commands::result_json;
use rastai_formats::commands::system::{
    SystemShutdownArgs, SystemShutdownResult, SystemStatusArgs, SystemStatusResult,
};
use rastai_proto::common::v1::JobState;
use rastai_store::index::runs::JobFilter;

use crate::commands::context::{CommandContext, sync_result};
use crate::commands::handlers::job::read_jobs;
use crate::commands::handlers::node::node_summaries;
use crate::shutdown::request_shutdown;

/// The job states of a job whose process runs on (contracts 9.1), which a daemon stop leaves running.
const RUNNING_JOB_STATES: &[JobState] = &[
    JobState::Starting,
    JobState::Running,
    JobState::Paused,
    JobState::Stopping,
    JobState::Lost,
];

/// The daemon's versions and its nodes, the embedded one first.
pub(crate) async fn status(
    context: &CommandContext<'_>,
    _arguments: SystemStatusArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let result = SystemStatusResult {
        app_version: APP_VERSION.to_string(),
        plugin_api_version: PLUGIN_API_VERSION.to_string(),
        node_protocol_version: NODE_PROTOCOL_VERSION,
        worker_protocol_version: WORKER_PROTOCOL_VERSION,
        api_packages: API_PACKAGES.iter().map(|package| package.to_string()).collect(),
        nodes: node_summaries(context.server)?,
    };
    Ok(sync_result(result_json(&result)))
}

/// Asks the daemon to stop, the same way SIGTERM does (shutdown.rs). The answer still goes out: the listeners finish
/// the requests they hold. Jobs keep running in their own process groups, and the answer names the ones that run
/// on: every job that has started and not ended. A dry run stops nothing.
pub(crate) async fn shutdown(
    context: &CommandContext<'_>,
    _arguments: SystemShutdownArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let running_job_ids = read_jobs(context.server, &JobFilter::default())?
        .into_iter()
        .filter(|job| parse_job_state(&job.state).is_some_and(|state| RUNNING_JOB_STATES.contains(&state)))
        .map(|job| job.job_id)
        .collect();
    if !context.dry_run {
        request_shutdown(context.server);
    }
    let result = SystemShutdownResult { running_job_ids };
    Ok(sync_result(result_json(&result)))
}
