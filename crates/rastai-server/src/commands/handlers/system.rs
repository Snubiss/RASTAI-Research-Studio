//! The `system` commands built so far (contracts 6.4): `system.status` and `system.shutdown`. The others
//! (`system.doctor`, `system.hardware`, `system.env.*`, updates, crash reports) answer their area's "not available
//! yet" code from dispatch.rs until their milestones.

use rastai_core::commands::output::CommandOutput;
use rastai_core::versions::{
    API_PACKAGES, APP_VERSION, NODE_PROTOCOL_VERSION, PLUGIN_API_VERSION, WORKER_PROTOCOL_VERSION,
};
use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::commands::result_json;
use rastai_formats::commands::system::{
    SystemShutdownArgs, SystemShutdownResult, SystemStatusArgs, SystemStatusResult,
};

use crate::commands::context::{CommandContext, sync_result};
use crate::shutdown::request_shutdown;

/// The daemon's versions and nodes. The embedded node and remote nodes join with the node links (M1-47); until then
/// the daemon has none to list.
pub(crate) async fn status(
    _context: &CommandContext<'_>,
    _arguments: SystemStatusArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let result = SystemStatusResult {
        app_version: APP_VERSION.to_string(),
        plugin_api_version: PLUGIN_API_VERSION.to_string(),
        node_protocol_version: NODE_PROTOCOL_VERSION,
        worker_protocol_version: WORKER_PROTOCOL_VERSION,
        api_packages: API_PACKAGES.iter().map(|package| package.to_string()).collect(),
        nodes: Vec::new(),
    };
    Ok(sync_result(result_json(&result)))
}

/// Asks the daemon to stop, the same way SIGTERM does (shutdown.rs). The answer still goes out: the listeners finish
/// the requests they hold. Jobs keep running in their own process groups; with no node links yet (M1-47) the daemon
/// knows of none. A dry run stops nothing.
pub(crate) async fn shutdown(
    context: &CommandContext<'_>,
    _arguments: SystemShutdownArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    if !context.dry_run {
        request_shutdown(context.server);
    }
    let result = SystemShutdownResult {
        running_job_ids: Vec::new(),
    };
    Ok(sync_result(result_json(&result)))
}
