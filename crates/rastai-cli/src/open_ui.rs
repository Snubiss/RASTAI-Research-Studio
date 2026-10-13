//! `rastai` and `rastai open` (layout: rust-control-plane.md 9.1; flows/platform.md 4.1): make sure the daemon
//! runs, mint a one-time login code with `token.login_code.create` over daemon.sock (the CLI is the owner there),
//! and open the browser at `http://127.0.0.1:<port>/#login=<code>`. `--print-url` prints one `DesktopLaunchInfo`
//! JSON line instead, which the desktop shell reads. The code travels in the URL fragment, which browsers never send
//! to a server.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rastai_core::paths::DataPaths;
use rastai_core::versions::APP_VERSION;
use rastai_core::wire::diagnostic_from_proto;
use rastai_diag::diagnostic::Diagnostic;
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use serde_json::{Value, json};

use crate::daemon_client::{DaemonClient, ensure_daemon_running, execute, read_health};
use crate::error::CliError;
use crate::exit_codes::{EXIT_NOT_PERMITTED, EXIT_USAGE, exit_code_for_failure};
use crate::modes::OpenFlags;
use crate::output::{OutputSettings, print_line};

const LOGIN_CODE_COMMAND: &str = "token.login_code.create";
const SETTINGS_GET_COMMAND: &str = "settings.get";
const SHUTDOWN_COMMAND: &str = "system.shutdown";
const UPDATES_SECTION: &str = "updates";
const OPEN_BROWSER_SETTING: &str = "daemon.open_browser";
const LOOPBACK_ADDRESS: &str = "127.0.0.1";
/// The fragment the web app reads the code from (contracts 4.7).
const LOGIN_FRAGMENT: &str = "#login=";
/// How long a replaced daemon has to stop before this version starts.
const SHUTDOWN_WAIT_SECONDS: u64 = 10;
const SHUTDOWN_POLL_MS: u64 = 100;
/// How long `rastai serve` waits for its own health before opening the browser.
const SERVE_HEALTH_WAIT_SECONDS: u64 = 30;

/// `rastai open`. Returns the warnings to report at the end, such as a daemon of another version.
pub async fn run_open(
    paths: &DataPaths,
    flags: &OpenFlags,
    no_start: bool,
    settings: OutputSettings,
) -> Result<Vec<Diagnostic>, CliError> {
    let mut client = ensure_daemon_running(paths, no_start).await?;
    if flags.replace_older_daemon && client.daemon_version != APP_VERSION {
        client = replace_daemon(paths, &client).await?;
    }
    let url = login_url(&client).await?;
    if flags.print_url {
        let launch_info = desktop_launch_info(paths, &client, &url).await;
        print_line(settings, &launch_info.to_string(), &launch_info);
    } else {
        print_line(settings, &url, &json!({ "url": url }));
        open_browser(&url);
    }
    Ok(client.warnings)
}

/// `rastai serve` without `--headless`: once the daemon answers, open the browser signed in, when
/// `daemon.open_browser` is on (flows/platform.md 2.1). Nothing is reported if it can't: the daemon runs on.
pub async fn open_browser_when_healthy(paths: DataPaths) {
    let deadline = Instant::now() + Duration::from_secs(SERVE_HEALTH_WAIT_SECONDS);
    let socket_path = rastai_core::paths::daemon_socket_path(&paths);
    while read_health(&socket_path).await.is_err() {
        if Instant::now() > deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(SHUTDOWN_POLL_MS)).await;
    }
    let Ok(client) = ensure_daemon_running(&paths, true).await else {
        return;
    };
    let wanted = setting_value(&client, OPEN_BROWSER_SETTING).await;
    if wanted.as_bool() != Some(true) {
        return;
    }
    if let Ok(url) = login_url(&client).await {
        open_browser(&url);
    }
}

/// A new login code as the UI's start URL.
async fn login_url(client: &DaemonClient) -> Result<String, CliError> {
    let result = run_sync_command(client, LOGIN_CODE_COMMAND, json!({})).await?;
    let code = result["code"].as_str().unwrap_or_default();
    let port = result["http_port"].as_u64().unwrap_or_default();
    Ok(format!("http://{LOOPBACK_ADDRESS}:{port}/{LOGIN_FRAGMENT}{code}"))
}

/// `{url, cli_version, daemon_version, runtime_dir, config_dir, updates: {channel, auto_check}}` (contracts 4.7).
async fn desktop_launch_info(paths: &DataPaths, client: &DaemonClient, url: &str) -> Value {
    let updates = setting_value(client, UPDATES_SECTION).await;
    json!({
        "url": url,
        "cli_version": APP_VERSION,
        "daemon_version": client.daemon_version,
        "runtime_dir": paths.runtime.display().to_string(),
        "config_dir": paths.config.display().to_string(),
        "updates": {
            "channel": updates["channel"],
            "auto_check": updates["auto_check"],
        },
    })
}

/// One setting or section from `settings.get`; null when it can't be read.
async fn setting_value(client: &DaemonClient, key: &str) -> Value {
    let Ok(result) = run_sync_command(client, SETTINGS_GET_COMMAND, json!({ "key": key })).await else {
        return Value::Null;
    };
    let mut value = &result["values"];
    for segment in key.split('.') {
        value = &value[segment];
    }
    value.clone()
}

/// Stops a daemon of another version with `system.shutdown` and starts this one.
async fn replace_daemon(paths: &DataPaths, older: &DaemonClient) -> Result<DaemonClient, CliError> {
    run_sync_command(older, SHUTDOWN_COMMAND, json!({})).await?;
    let deadline = Instant::now() + Duration::from_secs(SHUTDOWN_WAIT_SECONDS);
    while read_health(&older.socket_path).await.is_ok() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(SHUTDOWN_POLL_MS)).await;
    }
    ensure_daemon_running(paths, false).await
}

/// Runs a sync command with these arguments and returns its result; a refusal is an error.
async fn run_sync_command(client: &DaemonClient, command_id: &str, arguments: Value) -> Result<Value, CliError> {
    let request = ExecuteRequest {
        command_id: command_id.to_string(),
        arguments_json: arguments.to_string(),
        ..ExecuteRequest::default()
    };
    let response = execute(client, &request).await?;
    if !response.ok {
        return Err(failed_command(&response));
    }
    Ok(serde_json::from_str(&response.result_json).unwrap_or_default())
}

fn failed_command(response: &ExecuteResponse) -> CliError {
    let diagnostics: Vec<Diagnostic> = response.diagnostics.iter().map(diagnostic_from_proto).collect();
    match exit_code_for_failure(&diagnostics) {
        EXIT_NOT_PERMITTED => CliError::NotPermitted(diagnostics),
        EXIT_USAGE => CliError::Usage(diagnostics),
        _ => CliError::Failed(diagnostics),
    }
}

/// Opens the user's browser. A machine without one (a headless box) still has the URL, printed before.
fn open_browser(url: &str) {
    let mut command = browser_command(url);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let _ = command.spawn();
}

#[cfg(target_os = "macos")]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("explorer");
    command.arg(url);
    command
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}
