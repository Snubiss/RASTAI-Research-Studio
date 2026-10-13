//! `rastai serve [--headless] [--port N] [--set key=value]` (layout: rust-control-plane.md 9.1): parses its flags
//! and runs the daemon, `rastai_server::daemon::run_daemon`, until it stops. Without `--headless` it also opens the
//! browser signed in once the daemon answers, when `daemon.open_browser` is on (open_ui.rs).

use rastai_core::paths::DataPaths;
use rastai_server::daemon::{DaemonOptions, run_daemon};
use rastai_server::error::server_error_diagnostic;

use crate::error::CliError;
use crate::modes::ServeFlags;
use crate::open_ui::open_browser_when_healthy;

/// Runs the daemon until SIGTERM, SIGINT or `system.shutdown`. A daemon that can't start says why, with the
/// diagnostic of its startup error.
pub async fn run_serve(paths: &DataPaths, flags: &ServeFlags, set_flags: &[String]) -> Result<(), CliError> {
    let options = DaemonOptions {
        headless: flags.headless,
        port: flags.port,
        set_flags: set_flags.to_vec(),
        open_browser: !flags.headless,
    };
    if options.open_browser {
        tokio::spawn(open_browser_when_healthy(paths.clone()));
    }
    run_daemon(options)
        .await
        .map_err(|error| CliError::Failed(vec![server_error_diagnostic(&error)]))
}
