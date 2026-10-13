//! The `node` commands built so far (contracts 6.4 and 9.2): `node.list` and `node.status`. Nodes are rows of
//! `nodes`, which the node links write (nodes/node_links.rs); the telemetry of the last heartbeat is kept in memory
//! only, since it is ephemeral (contracts 7.3). Pairing, unpairing and updates join with remote nodes (M9).

use rastai_core::commands::output::CommandOutput;
use rastai_core::proto_json::node_telemetry_to_json;
use rastai_core::states::{is_terminal_job_state, parse_job_state};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::commands::node::{NodeListArgs, NodeListResult, NodeStatusArgs, NodeStatusResult, NodeSummary};
use rastai_formats::commands::result_json;
use rastai_store::index::kept_tables::{NodeRow, get_node, list_nodes};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::JobFilter;
use serde_json::Value;

use crate::commands::context::{CommandContext, store_failure, sync_result};
use crate::commands::handlers::job::{job_summary, read_jobs};
use crate::nodes::node_links::link_telemetry;
use crate::server_state::Server;

const CODE_NO_SUCH_NODE: &str = "RRS-N0008";

/// Every node the daemon knows, the embedded one first.
pub(crate) async fn list(
    context: &CommandContext<'_>,
    _arguments: NodeListArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let result = NodeListResult {
        nodes: node_summaries(context.server)?,
    };
    Ok(sync_result(result_json(&result)))
}

/// One node: its row, its last telemetry (absent before the first heartbeat) and its jobs that haven't ended.
/// `RRS-N0008` for an id the daemon doesn't know.
pub(crate) async fn status(
    context: &CommandContext<'_>,
    arguments: NodeStatusArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let server = context.server;
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    let row = get_node(&reader, &arguments.node_id).map_err(|error| store_failure(&error))?;
    drop(reader);
    let Some(row) = row else {
        return Err(vec![diagnostic(CODE_NO_SUCH_NODE, &[("node_id", &arguments.node_id)])]);
    };
    let telemetry = match link_telemetry(server, &row.node_id) {
        Some(telemetry) => node_telemetry_to_json(&telemetry),
        None => Value::Null,
    };
    let filter = JobFilter {
        node_id: row.node_id.clone(),
        ..JobFilter::default()
    };
    let jobs = read_jobs(server, &filter)?
        .iter()
        .filter(|job| !parse_job_state(&job.state).is_some_and(is_terminal_job_state))
        .map(job_summary)
        .collect();
    let result = NodeStatusResult {
        node: node_summary(&row),
        telemetry,
        jobs,
    };
    Ok(sync_result(result_json(&result)))
}

/// Every `nodes` row as the commands return it, the embedded node first: `node.list` and `system.status`.
pub(crate) fn node_summaries(server: &Server) -> Result<Vec<NodeSummary>, Vec<Diagnostic>> {
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    let rows = list_nodes(&reader).map_err(|error| store_failure(&error))?;
    Ok(rows.iter().map(node_summary).collect())
}

fn node_summary(row: &NodeRow) -> NodeSummary {
    NodeSummary {
        node_id: row.node_id.clone(),
        name: row.name.clone(),
        transport: row.transport.clone(),
        address: row.address.clone(),
        state: row.state.clone(),
        protocol_version: u32::try_from(row.protocol_version).unwrap_or_default(),
        capabilities: row.capabilities.clone(),
        last_heartbeat_unix_ms: row
            .last_heartbeat_unix_ms
            .and_then(|time| u64::try_from(time).ok())
            .unwrap_or_default(),
    }
}
