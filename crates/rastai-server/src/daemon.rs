//! `run_daemon`, the steps of flows/platform.md 2.1 one call each (layout section 8.2). `rastai serve` in rastai-cli
//! parses its flags and calls it.
//!
//! The order matters: the daemon lock first, so two daemons never share a data directory; then the owner token, so
//! a too-open or unusable token (`RRS-C1008`, `RRS-C1010`) refuses the start before a database opens or a listener
//! binds; the listeners last, so `/api/health` answers only once everything behind it is ready.
//!
//! `start_daemon` is everything up to serving, and `wait_for_shutdown` (shutdown.rs) the rest, so tests can start a
//! daemon on temporary folders and stop it again without signals.

use std::net::IpAddr;
use std::sync::Arc;

use rastai_core::paths::{DataPaths, daemon_log_path, read_path_environment, resolve_paths, user_settings_path};
use rastai_core::scopes::{Scope, parse_scope};
use rastai_core::settings::{
    ResolvedSettings, SettingsLayer, SettingsSource, setting_text, setting_u64, settings_from_environment,
    settings_from_flags,
};
use rastai_diag::diagnostic::Diagnostic;
use rastai_diag::logger::{LogLevel, LoggerSettings, init_logger, log_info, log_warn};
use rastai_formats::hashing::hash_bytes;
use rastai_store::data_dir_lock::{DaemonLock, take_daemon_lock};
use rastai_store::index::kept_tables::list_tokens;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, open_store};
use rastai_store::writer_thread::write_index;
use tokio::task::JoinHandle;

use crate::auth_files::ensure_owner_token;
use crate::commands::handlers::settings::{read_settings_layer, resolve_with_files};
use crate::error::ServerError;
use crate::http::listeners::{FIRST_DEFAULT_PORT, LOOPBACK_ADDRESS, listen_http};
use crate::http::sessions::{AuthState, ScopedToken, add_scoped_token, new_auth_state};
use crate::plugin_host::discovery::discover_plugins;
use crate::plugin_host::load::load_all_plugins;
use crate::server_state::{Server, build_server};
use crate::shutdown::{stop_on_signals, wait_for_shutdown};

/// The name in every line of `<state>/logs/daemon.jsonl`.
const PROCESS_NAME: &str = "daemon";
const PORT_SETTING: &str = "daemon.port";
const LISTEN_SETTING: &str = "daemon.listen";
const LOG_LEVEL_SETTING: &str = "logging.level";
const LOG_MAX_FILE_BYTES_SETTING: &str = "logging.max_file_bytes";
const LOG_KEEP_FILES_SETTING: &str = "logging.keep_files";
/// Characters of the daemon id in the session cookie's name (`rastai_session_<prefix>`): enough to tell a handful
/// of daemons apart in one browser.
const DAEMON_ID_PREFIX_LENGTH: usize = 8;
/// `hash_bytes` writes `blake3:<hex>`; the prefix is taken from the hex.
const HASH_ALGORITHM_SEPARATOR: char = ':';

/// The settings the daemon starts with, and what it noticed on the way.
struct StartupSettings {
    resolved: ResolvedSettings,
    /// The environment and `--set` layers, kept for `settings.set` (server_state.rs).
    overrides: Vec<SettingsLayer>,
    /// The user's `settings.json` when it doesn't parse (`RRS-C0009`): logged once the logger runs, and left out.
    user_file_problem: Option<Diagnostic>,
}

/// What `rastai serve` passes in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DaemonOptions {
    /// `--headless`: no browser is ever opened (rastai-cli acts on it).
    pub headless: bool,
    /// `--port`: tried before the `daemon.port` setting and the default range.
    pub port: Option<u16>,
    /// The text after each `--set`.
    pub set_flags: Vec<String>,
    /// Open the browser with a login code once healthy (rastai-cli acts on it, `daemon.open_browser`).
    pub open_browser: bool,
}

/// A started daemon: serving, and holding the lock until `wait_for_shutdown` releases it.
pub struct RunningDaemon {
    pub server: Arc<Server>,
    /// The loopback port it serves on, also in `<runtime>/daemon.port`.
    pub http_port: u16,
    pub serve_tasks: Vec<JoinHandle<()>>,
    pub daemon_lock: DaemonLock,
}

/// The daemon process: start, then serve until SIGTERM, SIGINT or `system.shutdown`.
pub async fn run_daemon(options: DaemonOptions) -> Result<(), ServerError> {
    let paths = resolve_paths(&read_path_environment()?);
    let running = start_daemon(paths, &options).await?;
    stop_on_signals(&running.server);
    wait_for_shutdown(running).await
}

/// Everything up to serving, in the order of flows/platform.md 2.1.
pub async fn start_daemon(paths: DataPaths, options: &DaemonOptions) -> Result<RunningDaemon, ServerError> {
    let settings = load_settings_layers(&paths, options)?;
    let daemon_lock = take_daemon_lock(&paths)?; // RRS-S0001, RRS-S0007
    start_logger(&paths, &settings.resolved); // after the lock, so two daemons never rotate one log file
    log_settings_problem(settings.user_file_problem.as_ref());
    let owner_token_hash = ensure_owner_token(&paths)?; // RRS-C1008, RRS-C1010: before any database or listener
    let store = open_store(&paths)?; // migrations, writer thread
    close_interrupted_commands(&store).await?; // RRS-C0005 on the rows a crash left unfinished
    start_embedded_node();
    let auth = new_auth_state(owner_token_hash.clone(), &daemon_id_prefix(&owner_token_hash));
    load_scoped_tokens(&store, &auth)?;
    let preferred_port = preferred_port(options, &settings.resolved);
    let listen_address = listen_address(&settings.resolved);
    let server = Arc::new(build_server(paths, settings.resolved, settings.overrides, store, auth));
    discover_plugins(&server)?; // manifests and saved states only; nothing starts
    load_all_plugins(&server).await; // failures disable or fail a plugin, never the daemon
    let http = listen_http(&server, preferred_port, listen_address).await?; // writes daemon.port; health ok from here
    let _ = server.http_port.set(http.tcp_port);
    log_info("daemon started", &[("port", &http.tcp_port.to_string())]);
    Ok(RunningDaemon {
        server,
        http_port: http.tcp_port,
        serve_tasks: http.serve_tasks,
        daemon_lock,
    })
}

/// The defaults, the user's `settings.json`, the environment and `--set` flags (contracts 11.1). A project's file
/// applies only to requests inside that project (`settings.get`). A malformed `--set` refuses the start
/// (`RRS-C0002`); a user file that doesn't parse doesn't: it is left out, reported, and never overwritten.
fn load_settings_layers(paths: &DataPaths, options: &DaemonOptions) -> Result<StartupSettings, ServerError> {
    let flags = settings_from_flags(&options.set_flags).map_err(|diagnostic| ServerError::InvalidSetFlag {
        message: diagnostic.message,
    })?;
    let environment = settings_from_environment(&environment_variables());
    let overrides = vec![environment, flags];
    let (user_layer, user_file_problem) = match read_settings_layer(&user_settings_path(paths), SettingsSource::User) {
        Ok(layer) => (layer, None),
        Err(problem) => (None, Some(problem)),
    };
    Ok(StartupSettings {
        resolved: resolve_with_files(user_layer, None, &overrides),
        overrides,
        user_file_problem,
    })
}

fn log_settings_problem(problem: Option<&Diagnostic>) {
    if let Some(problem) = problem {
        log_warn(
            "the user settings file was left out",
            &[("code", &problem.code), ("message", &problem.message)],
        );
    }
}

/// Journal rows started and never finished belong to commands the last daemon was running when it stopped; they
/// are closed as interrupted (`RRS-C0005`) before anything is served. The daemon log's replay (M1-28) goes first,
/// so a command whose `command.finished` record made it to the log keeps its real outcome.
async fn close_interrupted_commands(store: &Store) -> Result<(), ServerError> {
    write_index(&store.index, IndexWrite::MarkUnfinishedJournalRows).await?;
    Ok(())
}

/// The gate's copy of the scoped tokens' hashes and scopes, from `rastai.db`. A stored scope this build can't read
/// is left out of the token, so it never grants more than it says.
fn load_scoped_tokens(store: &Store, auth: &AuthState) -> Result<(), ServerError> {
    let reader = open_index_reader(&store.index_readers)?;
    for row in list_tokens(&reader)? {
        let scopes: Vec<Scope> = row.scopes.iter().filter_map(|name| parse_scope(name).ok()).collect();
        let token = ScopedToken {
            token_id: row.token_id,
            token_hash: row.token_hash,
            scopes,
            revoked: row.revoked_unix_ms.is_some(),
        };
        add_scoped_token(auth, token);
    }
    Ok(())
}

/// The process's variables that are text. `std::env::vars` would panic on one that isn't Unicode; such a variable
/// can't be a `RASTAI_` setting anyway.
fn environment_variables() -> Vec<(String, String)> {
    let mut variables = Vec::new();
    for (name, value) in std::env::vars_os() {
        if let (Ok(name), Ok(value)) = (name.into_string(), value.into_string()) {
            variables.push((name, value));
        }
    }
    variables
}

fn start_logger(paths: &DataPaths, settings: &ResolvedSettings) {
    init_logger(LoggerSettings {
        process_name: PROCESS_NAME.to_string(),
        file_path: Some(daemon_log_path(paths)),
        level: log_level(&setting_text(settings, LOG_LEVEL_SETTING)),
        max_file_bytes: setting_u64(settings, LOG_MAX_FILE_BYTES_SETTING),
        keep_files: u32::try_from(setting_u64(settings, LOG_KEEP_FILES_SETTING)).unwrap_or(u32::MAX),
    });
}

/// `logging.level`; an unknown word reads as info, the default.
fn log_level(name: &str) -> LogLevel {
    match name {
        "debug" => LogLevel::Debug,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// The embedded node starts here, on its own runtime, and the daemon reaches it over node.sock (M1-47). Until then
/// the daemon runs without a node: the commands that need one answer their "not available yet" code.
fn start_embedded_node() {}

/// Until the daemon key exists, the cookie's daemon id prefix is the start of a hash of the owner token's hash:
/// stable for this installation, different on another machine, and it says nothing about the token. M1-47 loads
/// `daemon.key` here (`load_or_create_identity`, from `rastai_node::identity`, M1-42), keeps it in the server's
/// `identity` field and takes this prefix from the daemon id instead.
fn daemon_id_prefix(owner_token_hash: &str) -> String {
    let hash = hash_bytes(owner_token_hash.as_bytes());
    let hex = match hash.split_once(HASH_ALGORITHM_SEPARATOR) {
        Some((_algorithm, hex)) => hex,
        None => hash.as_str(),
    };
    hex.chars().take(DAEMON_ID_PREFIX_LENGTH).collect()
}

/// `--port`, else `daemon.port`, else 47820. The default range is tried after it either way (listeners.rs).
fn preferred_port(options: &DaemonOptions, settings: &ResolvedSettings) -> u16 {
    if let Some(port) = options.port {
        return port;
    }
    u16::try_from(setting_u64(settings, PORT_SETTING)).unwrap_or(FIRST_DEFAULT_PORT)
}

/// `daemon.listen`. A value that isn't an IP address falls back to loopback, never to every interface; a
/// non-loopback address is used as asked, with a warning at every start (layout section 8.5).
fn listen_address(settings: &ResolvedSettings) -> IpAddr {
    let text = setting_text(settings, LISTEN_SETTING);
    match text.parse::<IpAddr>() {
        Ok(address) if address.is_loopback() => address,
        Ok(address) => {
            log_warn(
                "daemon.listen is not a loopback address; the API is reachable from the network",
                &[("listen", &text)],
            );
            address
        }
        Err(_) => {
            log_warn(
                "daemon.listen is not an IP address; listening on loopback",
                &[("listen", &text)],
            );
            LOOPBACK_ADDRESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rastai_core::settings::{default_settings, resolve_settings};
    use serde_json::json;

    fn settings_with(values: serde_json::Value) -> ResolvedSettings {
        let layer = SettingsLayer {
            source: SettingsSource::Flag,
            values,
        };
        resolve_settings(&[default_settings(), layer])
    }

    #[test]
    fn the_port_flag_wins_over_the_setting() {
        let settings = settings_with(json!({"daemon": {"port": 8899}}));
        let with_flag = DaemonOptions {
            port: Some(9000),
            ..DaemonOptions::default()
        };
        assert_eq!(preferred_port(&with_flag, &settings), 9000);
        assert_eq!(preferred_port(&DaemonOptions::default(), &settings), 8899);
        assert_eq!(
            preferred_port(&DaemonOptions::default(), &settings_with(json!({}))),
            47820
        );
        let too_big = settings_with(json!({"daemon": {"port": 70000}}));
        assert_eq!(preferred_port(&DaemonOptions::default(), &too_big), 47820);
    }

    #[test]
    fn a_bad_listen_address_falls_back_to_loopback() {
        assert_eq!(listen_address(&settings_with(json!({}))), LOOPBACK_ADDRESS);
        assert_eq!(
            listen_address(&settings_with(json!({"daemon": {"listen": "every"}}))),
            LOOPBACK_ADDRESS
        );
        let ipv6 = listen_address(&settings_with(json!({"daemon": {"listen": "::1"}})));
        assert!(ipv6.is_loopback());
    }

    #[test]
    fn the_daemon_id_prefix_is_eight_hex_digits_and_stable() {
        let prefix = daemon_id_prefix("blake3:00");
        assert_eq!(prefix.len(), 8);
        assert!(prefix.chars().all(|character| character.is_ascii_hexdigit()));
        assert_eq!(prefix, daemon_id_prefix("blake3:00"));
        assert_ne!(prefix, daemon_id_prefix("blake3:01"));
    }

    #[test]
    fn log_levels_read_their_words() {
        assert_eq!(log_level("debug"), LogLevel::Debug);
        assert_eq!(log_level("warn"), LogLevel::Warn);
        assert_eq!(log_level("error"), LogLevel::Error);
        assert_eq!(log_level("info"), LogLevel::Info);
        assert_eq!(log_level("loud"), LogLevel::Info);
    }
}
