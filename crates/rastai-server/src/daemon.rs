//! `run_daemon`, the steps of flows/platform.md 2.1 one call each (layout section 8.2). `rastai serve` in rastai-cli
//! parses its flags and calls it.
//!
//! The order matters: the daemon lock first, so two daemons never share a data directory; then the owner token, so
//! a too-open or unusable token (`RRS-C1008`, `RRS-C1010`) refuses the start before a database opens or a listener
//! binds; the daemon log's replay before the listeners, so the first request sees every record the daemon logged;
//! the sweep of tokens whose plugin is gone after the plugins load, so a dropped dev path's tokens are refused from
//! the first request (contracts 8.2); the node hub before the handshake that names it, and the handshake and
//! reconciliation before the listeners; the listeners last, so `/api/health` answers only once everything behind it
//! is ready.
//!
//! **The embedded node** (flows/platform.md 2.1) runs `rastai_node::node::run_node` on its own thread and tokio
//! runtime, and the daemon reaches it over node.sock with the handshake a remote node gets, so local and remote never
//! drift. Its key is `<data>/identity/node.key`; the daemon pins its own key for it on every start. A node that can't
//! be reached at start doesn't stop the daemon: the link's watch keeps trying (nodes/node_links.rs). Stopping the
//! daemon stops the embedded node and never its jobs, which run in process groups of their own and register again
//! with the next node.
//!
//! `start_daemon` is everything up to serving, and `wait_for_shutdown` (shutdown.rs) the rest, so tests can start a
//! daemon on temporary folders and stop it again without signals.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rastai_core::ids::node_id_from_public_key;
use rastai_core::paths::{
    DataPaths, daemon_key_path, daemon_log_path, environments_dir, node_key_path, node_socket_path,
    paired_daemons_path, read_path_environment, resolve_paths, user_settings_path,
};
use rastai_core::scopes::{Scope, parse_scope};
use rastai_core::settings::{
    ResolvedSettings, SettingsLayer, SettingsSource, node_settings, setting_text, setting_u64,
    settings_from_environment, settings_from_flags,
};
use rastai_diag::diagnostic::Diagnostic;
use rastai_diag::logger::{LogLevel, LoggerSettings, init_logger, log_diagnostic, log_info, log_warn};
use rastai_node::error::node_error_diagnostic;
use rastai_node::handshake::pin_daemon_key;
use rastai_node::hardware::system_hardware_sources;
use rastai_node::identity::{NodeIdentity, daemon_id_from_public_key, load_or_create_identity, public_key_bytes};
use rastai_node::jobs::supervisor::{JobEnvironment, default_job_options};
use rastai_node::node::{NodeMode, NodeOptions, run_node};
use rastai_store::data_dir_lock::{DaemonLock, take_daemon_lock};
use rastai_store::index::kept_tables::list_tokens;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, open_store};
use rastai_store::writer_thread::write_index;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::auth_files::ensure_owner_token;
use crate::commands::handlers::settings::{read_settings_layer, resolve_with_files};
use crate::error::ServerError;
use crate::http::listeners::{FIRST_DEFAULT_PORT, LOOPBACK_ADDRESS, listen_http};
use crate::http::sessions::{AuthState, ScopedToken, add_scoped_token, new_auth_state};
use crate::indexing::daemon_log_replay::replay_daemon_log;
use crate::nodes::node_hub::serve_node_hub;
use crate::nodes::node_links::{
    connect_node, embedded_node_id, embedded_node_links, watch_node_links, write_node_rows,
};
use crate::plugin_host::discovery::discover_plugins;
use crate::plugin_host::lifecycle::revoke_tokens_of_gone_plugins;
use crate::plugin_host::load::load_all_plugins;
use crate::scheduler::queue::run_scheduler;
use crate::server_state::{Server, ServerParts, build_server};
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
/// A daemon id is `daemon_<base32 of its public key>` (contracts 8.6); the cookie's prefix comes from the base32.
const DAEMON_ID_KIND_SEPARATOR: char = '_';
const EMBEDDED_NODE_THREAD_NAME: &str = "rastai-embedded-node";
/// The embedded node's runtime: its services, its job watchers and its telemetry need few threads.
const EMBEDDED_NODE_WORKER_THREADS: usize = 2;
/// How long the embedded node's tasks get to finish when the daemon stops it.
const EMBEDDED_NODE_STOP_SECONDS: u64 = 5;
/// How long the first connect waits for node.sock to appear: the node detects the hardware and reattaches its jobs
/// first (flows/platform.md 2.1).
const NODE_START_WAIT_SECONDS: u64 = 20;
const NODE_START_POLL_MS: u64 = 50;
/// What a dial reports while nothing listens on node.sock yet.
const CODE_SOCKET_NOT_REACHABLE: &str = "RRS-N2004";
/// `<data>/envs/<environment_id>-<lock_hash12>/`: the id is the folder name up to its first `-` (contracts 8.1).
const ENVIRONMENT_FOLDER_SEPARATOR: char = '-';

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
    /// The listeners, the node hub, the scheduler and the node links' watch; each ends on the shutdown flag.
    pub serve_tasks: Vec<JoinHandle<()>>,
    pub daemon_lock: DaemonLock,
    pub embedded_node: EmbeddedNode,
}

/// The embedded node's thread. Dropping it asks the node to stop; `stop_embedded_node` also waits for it.
pub struct EmbeddedNode {
    pub node_id: String,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for EmbeddedNode {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
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
    let identity = load_or_create_identity(&daemon_key_path(&paths))?; // RRS-N0006
    let embedded_node = start_embedded_node(&paths, &settings.resolved, &identity)?;
    let auth = new_auth_state(owner_token_hash, &daemon_id_prefix(&identity));
    load_scoped_tokens(&store, &auth)?;
    let preferred_port = preferred_port(options, &settings.resolved);
    let listen_address = listen_address(&settings.resolved);
    let nodes = embedded_node_links(&embedded_node.node_id, &node_socket_path(&paths));
    let server = Arc::new(build_server(ServerParts {
        paths,
        settings: settings.resolved,
        settings_overrides: settings.overrides,
        store,
        auth,
        identity,
        nodes,
    }));
    replay_daemon_log(&server).await?; // index the daemon-log records after the cursor, before anything is served
    close_interrupted_commands(&server.store).await?; // RRS-C0005 on the rows a crash left unfinished
    discover_plugins(&server)?; // manifests and saved states only; nothing starts
    let mut serve_tasks = vec![serve_node_hub(&server)?]; // node-hub.sock, which the handshake names
    connect_to_node(&server).await; // handshake, ListJobs, the run tails; the watch retries a failure
    load_all_plugins(&server).await; // failures disable or fail a plugin, never the daemon
    revoke_tokens_of_gone_plugins(&server).await?; // contracts 8.2: before any request, so no gone plugin's token works
    serve_tasks.extend(start_background_tasks(&server));
    let http = listen_http(&server, preferred_port, listen_address).await?; // writes daemon.port; health ok from here
    let _ = server.http_port.set(http.tcp_port);
    log_info("daemon started", &[("port", &http.tcp_port.to_string())]);
    serve_tasks.extend(http.serve_tasks);
    Ok(RunningDaemon {
        server,
        http_port: http.tcp_port,
        serve_tasks,
        daemon_lock,
        embedded_node,
    })
}

/// Starts the embedded node on its own thread and runtime (see the top) and returns its handle. The node's key is
/// made on the first start; the daemon's key is pinned for it, so the node takes this daemon's handshake.
fn start_embedded_node(
    paths: &DataPaths,
    settings: &ResolvedSettings,
    identity: &NodeIdentity,
) -> Result<EmbeddedNode, ServerError> {
    let node_identity = load_or_create_identity(&node_key_path(paths))?;
    let node_id = node_id_from_public_key(&public_key_bytes(&node_identity));
    let daemon_id = daemon_id_from_public_key(&public_key_bytes(identity));
    pin_daemon_key(&paired_daemons_path(paths), &daemon_id)?;
    let options = NodeOptions {
        mode: NodeMode::Embedded,
        paths: paths.clone(),
        settings: node_settings(settings),
        hardware_sources: system_hardware_sources(),
        jobs: default_job_options(local_environments(paths)),
    };
    let (stop, stopped) = oneshot::channel();
    let thread = spawn_node_thread(options, stopped);
    Ok(EmbeddedNode {
        node_id,
        stop: Some(stop),
        thread,
    })
}

/// Stops the embedded node and waits for its thread. Its jobs keep running.
pub async fn stop_embedded_node(mut node: EmbeddedNode) {
    if let Some(stop) = node.stop.take() {
        let _ = stop.send(());
    }
    if let Some(thread) = node.thread.take() {
        let _ = tokio::task::spawn_blocking(move || thread.join()).await;
    }
}

/// The environments the embedded node runs jobs in, one per folder of `<data>/envs/`. uv environments replace this
/// with M2-16; until then a developer links the cpu environment there (`<data>/envs/cpu-dev` → `.dev/envs/cpu`).
fn local_environments(paths: &DataPaths) -> Vec<JobEnvironment> {
    let Ok(entries) = std::fs::read_dir(environments_dir(paths)) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    folders.sort();
    let mut environments: Vec<JobEnvironment> = Vec::new();
    for folder in folders {
        let Some(environment_id) = environment_id_of(&folder) else {
            continue;
        };
        if environments.iter().all(|known| known.environment_id != environment_id) {
            environments.push(JobEnvironment { environment_id, folder });
        }
    }
    environments
}

/// `cpu` for `<data>/envs/cpu-0123456789ab`; None for a name without the separator.
fn environment_id_of(folder: &std::path::Path) -> Option<String> {
    let name = folder.file_name()?.to_str()?;
    let (environment_id, _lock_hash) = name.split_once(ENVIRONMENT_FOLDER_SEPARATOR)?;
    (!environment_id.is_empty()).then(|| environment_id.to_string())
}

fn spawn_node_thread(options: NodeOptions, stopped: oneshot::Receiver<()>) -> Option<std::thread::JoinHandle<()>> {
    let builder = std::thread::Builder::new().name(EMBEDDED_NODE_THREAD_NAME.to_string());
    match builder.spawn(move || run_embedded_node(options, stopped)) {
        Ok(thread) => Some(thread),
        Err(error) => {
            log_warn(
                "the embedded node's thread couldn't start; the daemon runs without a node",
                &[("reason", &error.to_string())],
            );
            None
        }
    }
}

/// The embedded node's thread: `run_node` until it ends or the daemon asks it to stop.
fn run_embedded_node(options: NodeOptions, stopped: oneshot::Receiver<()>) {
    let built = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(EMBEDDED_NODE_WORKER_THREADS)
        .thread_name(EMBEDDED_NODE_THREAD_NAME)
        .enable_all()
        .build();
    let runtime = match built {
        Ok(runtime) => runtime,
        Err(error) => {
            log_warn(
                "the embedded node's runtime couldn't start; the daemon runs without a node",
                &[("reason", &error.to_string())],
            );
            return;
        }
    };
    runtime.block_on(async move {
        tokio::select! {
            ended = run_node(options) => {
                if let Err(error) = ended {
                    log_diagnostic(&node_error_diagnostic(&error), &[]);
                }
            }
            _ = stopped => {}
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(EMBEDDED_NODE_STOP_SECONDS));
}

/// The first connect to the embedded node, waiting for node.sock while the node starts. A node that can't be
/// reached in time, or refuses the handshake, is left to the link's watch, which retries (nodes/node_links.rs).
async fn connect_to_node(server: &Arc<Server>) {
    write_node_rows(server).await;
    let node_id = embedded_node_id(server);
    let deadline = Instant::now() + Duration::from_secs(NODE_START_WAIT_SECONDS);
    loop {
        let problem = match connect_node(server, &node_id).await {
            Ok(()) => return,
            Err(problem) => problem,
        };
        if problem.code != CODE_SOCKET_NOT_REACHABLE || Instant::now() > deadline {
            log_warn(
                "the embedded node isn't connected yet; it is tried again",
                &[("code", &problem.code), ("message", &problem.message)],
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(NODE_START_POLL_MS)).await;
    }
}

/// The tasks that run while the daemon serves: the scheduler and the node links' watch. Each ends on the shutdown
/// flag.
fn start_background_tasks(server: &Arc<Server>) -> Vec<JoinHandle<()>> {
    vec![
        tokio::spawn(run_scheduler(server.clone())),
        tokio::spawn(watch_node_links(server.clone())),
    ]
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
/// are closed as interrupted (`RRS-C0005`) before anything is served. A command whose `command.finished` record made
/// it to the log keeps its real outcome whichever runs first: the replay writes that outcome over the mark.
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

/// The session cookie's daemon id prefix (`rastai_session_<prefix>`, contracts 3.3): the first characters of the
/// daemon id's base32, in lower case. Stable for this installation, different on another machine, and public anyway.
fn daemon_id_prefix(identity: &NodeIdentity) -> String {
    let daemon_id = daemon_id_from_public_key(&public_key_bytes(identity));
    let body = match daemon_id.split_once(DAEMON_ID_KIND_SEPARATOR) {
        Some((_kind, body)) => body,
        None => daemon_id.as_str(),
    };
    body.chars()
        .take(DAEMON_ID_PREFIX_LENGTH)
        .collect::<String>()
        .to_ascii_lowercase()
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

    fn temporary_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rsv-daemon-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap_or_default();
        folder
    }

    // The cookie names this daemon by its key (contracts 3.3): eight lower-case base32 characters, the same for the
    // same key and different for another.
    #[test]
    fn the_daemon_id_prefix_comes_from_the_daemon_key() {
        let folder = temporary_folder("prefix");
        let identity = load_or_create_identity(&folder.join("daemon.key")).ok();
        let other = load_or_create_identity(&folder.join("other.key")).ok();
        let (Some(identity), Some(other)) = (identity, other) else {
            panic!("the test keys couldn't be made in {}", folder.display());
        };
        let prefix = daemon_id_prefix(&identity);
        assert_eq!(prefix.len(), 8);
        assert!(
            prefix
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        );
        let daemon_id = daemon_id_from_public_key(&public_key_bytes(&identity));
        assert!(
            daemon_id.to_ascii_lowercase().starts_with(&format!("daemon_{prefix}")),
            "{daemon_id}"
        );
        assert_eq!(prefix, daemon_id_prefix(&identity));
        assert_ne!(prefix, daemon_id_prefix(&other));
        let _ = std::fs::remove_dir_all(&folder);
    }

    // `<data>/envs/<environment_id>-<lock_hash12>/` (contracts 8.1): one environment per id, the first folder of it.
    #[test]
    fn each_environment_folder_names_its_environment() {
        let folder = temporary_folder("envs");
        let paths = DataPaths {
            config: folder.join("config"),
            data: folder.join("data"),
            cache: folder.join("cache"),
            state: folder.join("state"),
            runtime: folder.join("run"),
        };
        assert!(local_environments(&paths).is_empty(), "no envs folder, no environments");
        for name in ["cpu-0123456789ab", "cpu-ffffffffffff", "cuda13-0123456789ab", "stray"] {
            std::fs::create_dir_all(environments_dir(&paths).join(name)).unwrap_or_default();
        }
        let environments = local_environments(&paths);
        let ids: Vec<&str> = environments.iter().map(|known| known.environment_id.as_str()).collect();
        assert_eq!(ids, ["cpu", "cuda13"]);
        assert!(environments[0].folder.ends_with("cpu-0123456789ab"));
        let _ = std::fs::remove_dir_all(&folder);
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
