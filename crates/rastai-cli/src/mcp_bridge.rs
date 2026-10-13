//! `rastai mcp` (layout: rust-control-plane.md 9.1; flows/platform.md 4.3): the MCP stdio bridge,
//! `rastai_mcp::bridge::run_stdio_bridge`, with the scoped token from `RASTAI_TOKEN`. The bridge never falls back
//! to the owner: without a token it answers every request with `RRS-C3002`. stdout is the MCP channel, so nothing
//! else is printed there (main.rs sends this mode's final report to stderr).

use rastai_core::paths::DataPaths;
use rastai_diag::diagnostic::Diagnostic;
use rastai_mcp::bridge::{DaemonEndpoint, run_stdio_bridge};

use crate::daemon_client::ensure_daemon_running;
use crate::error::CliError;

/// The scoped token an MCP client is given (`token.create`), passed in its server configuration.
const TOKEN_VARIABLE: &str = "RASTAI_TOKEN";

/// Bridges stdin and stdout to the daemon's `/mcp` until the client closes stdin. Returns the warnings to report.
pub async fn run_mcp_bridge(paths: &DataPaths, no_start: bool) -> Result<Vec<Diagnostic>, CliError> {
    let client = ensure_daemon_running(paths, no_start).await?;
    let token = std::env::var(TOKEN_VARIABLE).unwrap_or_default();
    let endpoint = DaemonEndpoint {
        socket_path: client.socket_path.clone(),
    };
    run_stdio_bridge(&endpoint, &token).await;
    Ok(client.warnings)
}
