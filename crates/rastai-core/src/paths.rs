//! The only place data paths are built (contracts 8.1, ADR-0014). No other code reads `RASTAI_HOME`, `HOME`,
//! `XDG_*`, `HF_HUB_CACHE` or `HF_HOME`, or joins a well-known folder name.
//!
//! `read_path_environment` reads the variables once; `resolve_paths` turns them into the five roots, without
//! touching the disk, so tests resolve any layout from a plain struct. Every other function here is one row of the
//! contracts 8.1 table: one root and fixed names, plus the ids the row names. Callers pass ids they have checked
//! (`id_has_prefix`, the plugin id grammar), because an id is joined into the path as it is.
//!
//! Rows without a function yet come with the code that needs them (contracts 15.1 lists them with their
//! milestones): `vault.db.damaged-<unix_ms>`, `backups/` and the node self-update files (L2), the crash and panic
//! report files inside `crash-reports/` (M6-E05), and the two paths outside the five roots, the preinstalled
//! plugins under the install prefix (M4-E02) and the node's systemd unit (M9-E02).
//!
//! Linux (XDG) only, as in the prototype; the macOS and Windows rows of 8.1 come with those builds.

use std::path::{Path, PathBuf};

use crate::error::CoreError;

// Environment variables. HF_* are huggingface_hub's own, so we read the cache their tools write.
const RASTAI_HOME_VARIABLE: &str = "RASTAI_HOME";
const XDG_CONFIG_HOME_VARIABLE: &str = "XDG_CONFIG_HOME";
const XDG_DATA_HOME_VARIABLE: &str = "XDG_DATA_HOME";
const XDG_CACHE_HOME_VARIABLE: &str = "XDG_CACHE_HOME";
const XDG_STATE_HOME_VARIABLE: &str = "XDG_STATE_HOME";
const XDG_RUNTIME_DIR_VARIABLE: &str = "XDG_RUNTIME_DIR";
const HF_HUB_CACHE_VARIABLE: &str = "HF_HUB_CACHE";
const HF_HOME_VARIABLE: &str = "HF_HOME";

// The XDG defaults under the home folder (XDG Base Directory specification).
const DEFAULT_CONFIG_HOME: &str = ".config";
const DEFAULT_DATA_HOME: &str = ".local/share";
const DEFAULT_CACHE_HOME: &str = ".cache";
const DEFAULT_STATE_HOME: &str = ".local/state";

// Our folder inside each XDG root, and the five root folders inside RASTAI_HOME.
const APP_FOLDER: &str = "rastai";
const CONFIG_FOLDER: &str = "config";
const DATA_FOLDER: &str = "data";
const CACHE_FOLDER: &str = "cache";
const STATE_FOLDER: &str = "state";
const RUNTIME_FOLDER: &str = "runtime";

// Without XDG_RUNTIME_DIR the sockets go in /tmp/rastai-<uid> (contracts 8.1). It is /tmp itself, not $TMPDIR, so
// the longest socket path stays within the 107 bytes a Unix socket address holds. The user id is read as the owner
// of /proc/self, because std has no getuid and libc is rastai-store's alone (ADR-0033).
const SHARED_TEMP_FOLDER: &str = "/tmp";
#[cfg(unix)]
const PROCESS_FOLDER: &str = "/proc/self";

// The Hugging Face hub cache: HF_HUB_CACHE, else HF_HOME/hub, else ~/.cache/huggingface/hub (spike hf_cache.py).
const HF_HUB_FOLDER: &str = "hub";
const DEFAULT_HF_HOME: &str = ".cache/huggingface";

// File extensions shared by several roots, written after the dot.
const JSON_EXTENSION: &str = "json";
const LOG_EXTENSION: &str = "log";
const SOCKET_EXTENSION: &str = "sock";

// <config>
const SETTINGS_FILE: &str = "settings.json";
const KEYBINDINGS_FILE: &str = "keybindings.json";
const LAYOUTS_FOLDER: &str = "layouts";
const CHAT_TEMPLATES_FOLDER: &str = "chat-templates";
const CHAT_TEMPLATE_EXTENSION: &str = "jinja";
const DESKTOP_WINDOW_FILE: &str = "desktop-window.json";
const AUTH_FOLDER: &str = "auth";
const OWNER_TOKEN_FILE: &str = "owner.token";
const ENCRYPTED_SECRETS_FILE: &str = "secrets.enc";

// <data>
const RASTAI_DB_FILE: &str = "rastai.db";
const VAULT_DB_FILE: &str = "vault.db";
// Database copies sit beside the database: `rastai.db.pre-<version>`, `rastai.db.broken-<unix_ms>`, the index a
// rebuild is still writing, `rastai.db.rebuilding`, and the copy of the old index it reads, `rastai.db.salvage`.
const PRE_UPDATE_MARK: &str = "pre";
const BROKEN_MARK: &str = "broken";
const REBUILDING_MARK: &str = "rebuilding";
const SALVAGE_MARK: &str = "salvage";
const BLOBS_FOLDER: &str = "blobs";
const RUNS_FOLDER: &str = "runs";
const DAEMON_FOLDER: &str = "daemon";
const EVENTS_LOG_FILE: &str = "events.log";
// Rolled segments of the daemon log: `events-<n>.log`.
const EVENTS_SEGMENT_STEM: &str = "events";
const IDENTITY_FOLDER: &str = "identity";
const DAEMON_KEY_FILE: &str = "daemon.key";
const NODE_KEY_FILE: &str = "node.key";
const NODE_FOLDER: &str = "node";
const PAIRED_DAEMONS_FILE: &str = "paired_daemons.json";
const KNOWN_NODES_FILE: &str = "known_nodes.json";
const SSH_KNOWN_HOSTS_FILE: &str = "ssh_known_hosts";
const HARDWARE_PROFILE_FILE: &str = "hardware_profile.json";
const ENVIRONMENTS_FOLDER: &str = "envs";
const PYTHON_FOLDER: &str = "python";
const BIN_FOLDER: &str = "bin";
const PLUGINS_FOLDER: &str = "plugins";
const PLUGIN_DATA_FOLDER: &str = "plugin-data";
const ENGINES_FOLDER: &str = "engines";
const LLAMA_SERVER_FOLDER: &str = "llama-server";
const PROJECTS_FOLDER: &str = "projects";

// <cache>
const TORCH_FOLDER: &str = "torch";
const UV_FOLDER: &str = "uv";
const TOKENIZERS_FOLDER: &str = "tokenizers";
const HF_HEADERS_FOLDER: &str = "hf-headers";
// huggingface_hub names a repository's folder `<org>--<name>`.
const HF_REPOSITORY_SEPARATOR: &str = "--";
const UPLOADS_FOLDER: &str = "uploads";
const UPDATES_FOLDER: &str = "updates";
const COMMAND_HELP_STEM: &str = "commands";
const HF_LOCKS_FOLDER: &str = ".locks";
const LOCK_EXTENSION: &str = "lock";

// <state>
const LOGS_FOLDER: &str = "logs";
const DAEMON_LOG_FILE: &str = "daemon.jsonl";
const NODE_LOG_FILE: &str = "node.jsonl";
const DAEMON_SESSION_FILE: &str = "daemon.session.json";
const CRASH_REPORTS_FOLDER: &str = "crash-reports";

// <runtime>
const DAEMON_SOCKET_FILE: &str = "daemon.sock";
const DAEMON_LOCK_FILE: &str = "daemon.lock";
const DAEMON_PORT_FILE: &str = "daemon.port";
const NODE_SOCKET_FILE: &str = "node.sock";
const NODE_HUB_SOCKET_FILE: &str = "node-hub.sock";
const NODE_HUB_SOCKET_STEM: &str = "node-hub";
const WORKER_HOST_SOCKET_FILE: &str = "worker-host.sock";
const JOBS_FOLDER: &str = "jobs";
const REMOTE_FOLDER: &str = "remote";
const DESKTOP_LOCK_FILE: &str = "desktop.lock";
const DESKTOP_SOCKET_FILE: &str = "desktop.sock";

/// The variables paths are resolved from. Unset and empty variables are both `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEnvironment {
    pub rastai_home: Option<PathBuf>,
    pub home: PathBuf,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_runtime_dir: Option<PathBuf>,
    pub hf_hub_cache: Option<PathBuf>,
    pub hf_home: Option<PathBuf>,
    /// The user id, which names `/tmp/rastai-<uid>` when `XDG_RUNTIME_DIR` is unset. None without `/proc`.
    pub user_id: Option<u32>,
}

/// The five roots of contracts 8.1: `<config>`, `<data>`, `<cache>`, `<state>` and `<runtime>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub runtime: PathBuf,
}

/// The path variables of this process. The home folder comes from `HOME`, else the user database, else
/// `RASTAI_HOME`; with none of the three, this fails with `HomeNotFound`.
pub fn read_path_environment() -> Result<PathEnvironment, CoreError> {
    let rastai_home = read_path_variable(RASTAI_HOME_VARIABLE);
    let home_folder = std::env::home_dir().filter(|folder| !folder.as_os_str().is_empty());
    let home = choose_home(home_folder, &rastai_home)?;
    Ok(PathEnvironment {
        rastai_home,
        home,
        xdg_config_home: read_path_variable(XDG_CONFIG_HOME_VARIABLE),
        xdg_data_home: read_path_variable(XDG_DATA_HOME_VARIABLE),
        xdg_cache_home: read_path_variable(XDG_CACHE_HOME_VARIABLE),
        xdg_state_home: read_path_variable(XDG_STATE_HOME_VARIABLE),
        xdg_runtime_dir: read_path_variable(XDG_RUNTIME_DIR_VARIABLE),
        hf_hub_cache: read_path_variable(HF_HUB_CACHE_VARIABLE),
        hf_home: read_path_variable(HF_HOME_VARIABLE),
        user_id: read_user_id(),
    })
}

/// The five roots. `RASTAI_HOME` puts all of them in one folder (tests, portable installs, `just serve-dev`).
/// Otherwise each is `rastai` inside its XDG folder, or inside the XDG default under the home folder.
pub fn resolve_paths(environment: &PathEnvironment) -> DataPaths {
    if let Some(rastai_home) = &environment.rastai_home {
        return paths_under_one_folder(rastai_home);
    }
    let state = xdg_root(&environment.xdg_state_home, &environment.home, DEFAULT_STATE_HOME);
    DataPaths {
        config: xdg_root(&environment.xdg_config_home, &environment.home, DEFAULT_CONFIG_HOME),
        data: xdg_root(&environment.xdg_data_home, &environment.home, DEFAULT_DATA_HOME),
        cache: xdg_root(&environment.xdg_cache_home, &environment.home, DEFAULT_CACHE_HOME),
        runtime: runtime_root(&environment.xdg_runtime_dir, environment.user_id, &state),
        state,
    }
}

/// The Hugging Face hub cache, read in place (contracts 8.1): `HF_HUB_CACHE`, else `HF_HOME/hub`, else
/// `~/.cache/huggingface/hub`, the order huggingface_hub uses, so we find what its tools downloaded.
pub fn hf_cache_dir(environment: &PathEnvironment) -> PathBuf {
    if let Some(hub_cache) = &environment.hf_hub_cache {
        return hub_cache.clone();
    }
    if let Some(hf_home) = &environment.hf_home {
        return hf_home.join(HF_HUB_FOLDER);
    }
    environment.home.join(DEFAULT_HF_HOME).join(HF_HUB_FOLDER)
}

// ---- <config> ----

/// `<config>/settings.json`: the user settings, their only store.
pub fn user_settings_path(paths: &DataPaths) -> PathBuf {
    paths.config.join(SETTINGS_FILE)
}

pub fn keybindings_path(paths: &DataPaths) -> PathBuf {
    paths.config.join(KEYBINDINGS_FILE)
}

pub fn layouts_dir(paths: &DataPaths) -> PathBuf {
    paths.config.join(LAYOUTS_FOLDER)
}

/// `<config>/chat-templates/<model_hash12>.jinja`: the user's chat template override for one model.
pub fn chat_template_override_path(paths: &DataPaths, model_hash12: &str) -> PathBuf {
    let template_file = format!("{model_hash12}.{CHAT_TEMPLATE_EXTENSION}");
    paths.config.join(CHAT_TEMPLATES_FOLDER).join(template_file)
}

pub fn desktop_window_path(paths: &DataPaths) -> PathBuf {
    paths.config.join(DESKTOP_WINDOW_FILE)
}

/// `<config>/auth/owner.token`, mode 0600.
pub fn owner_token_path(paths: &DataPaths) -> PathBuf {
    paths.config.join(AUTH_FOLDER).join(OWNER_TOKEN_FILE)
}

/// `<config>/auth/secrets.enc`: the encrypted secret store on headless Linux (contracts 8.5).
pub fn encrypted_secrets_path(paths: &DataPaths) -> PathBuf {
    paths.config.join(AUTH_FOLDER).join(ENCRYPTED_SECRETS_FILE)
}

// ---- <data> ----

pub fn rastai_db_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(RASTAI_DB_FILE)
}

pub fn vault_db_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(VAULT_DB_FILE)
}

/// `<data>/rastai.db.pre-<version>`: the index copy an update takes before it migrates the file.
pub fn rastai_db_pre_update_path(paths: &DataPaths, version: &str) -> PathBuf {
    let copy_file = format!("{RASTAI_DB_FILE}.{PRE_UPDATE_MARK}-{version}");
    paths.data.join(copy_file)
}

/// `<data>/vault.db.pre-<version>`: the vault copy an update takes when the vault's schema changes.
pub fn vault_db_pre_update_path(paths: &DataPaths, version: &str) -> PathBuf {
    let copy_file = format!("{VAULT_DB_FILE}.{PRE_UPDATE_MARK}-{version}");
    paths.data.join(copy_file)
}

/// `<data>/rastai.db.broken-<unix_ms>`: where `storage rebuild-index` moves the damaged index. The user deletes it.
pub fn rastai_db_broken_path(paths: &DataPaths, unix_ms: i64) -> PathBuf {
    let broken_file = format!("{RASTAI_DB_FILE}.{BROKEN_MARK}-{unix_ms}");
    paths.data.join(broken_file)
}

/// `<data>/rastai.db.rebuilding`: the index `storage rebuild-index` writes before it takes the place of `rastai.db`.
/// A crash leaves it behind with the old index untouched; the next rebuild starts it again.
pub fn rastai_db_rebuilding_path(paths: &DataPaths) -> PathBuf {
    let rebuilding_file = format!("{RASTAI_DB_FILE}.{REBUILDING_MARK}");
    paths.data.join(rebuilding_file)
}

/// `<data>/rastai.db.salvage`: the copy of the old index `storage rebuild-index` reads its kept tables from, so the
/// old file itself is never opened for writing and is kept as it was. Removed once read; the next rebuild removes one
/// a crash left.
pub fn rastai_db_salvage_path(paths: &DataPaths) -> PathBuf {
    let salvage_file = format!("{RASTAI_DB_FILE}.{SALVAGE_MARK}");
    paths.data.join(salvage_file)
}

pub fn blobs_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(BLOBS_FOLDER)
}

/// `<data>/runs/`, the folder that holds every run's folder: the rebuild of `rastai.db` lists it.
pub fn runs_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(RUNS_FOLDER)
}

/// `<data>/runs/<run_id>/` (contracts 8.4).
pub fn run_dir(paths: &DataPaths, run_id: &str) -> PathBuf {
    runs_dir(paths).join(run_id)
}

pub fn run_events_log_path(paths: &DataPaths, run_id: &str) -> PathBuf {
    run_dir(paths, run_id).join(EVENTS_LOG_FILE)
}

/// `<data>/daemon/events.log`: the daemon's durable event log, the segment being written.
pub fn daemon_events_log_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(DAEMON_FOLDER).join(EVENTS_LOG_FILE)
}

/// `<data>/daemon/events-<n>.log`: rolled segment `n` of the daemon log, beside the one being written.
pub fn daemon_events_segment_path(paths: &DataPaths, segment_number: u64) -> PathBuf {
    let segment_file = format!("{EVENTS_SEGMENT_STEM}-{segment_number}.{LOG_EXTENSION}");
    paths.data.join(DAEMON_FOLDER).join(segment_file)
}

/// `<data>/identity/daemon.key`, mode 0600.
pub fn daemon_key_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(IDENTITY_FOLDER).join(DAEMON_KEY_FILE)
}

/// `<data>/identity/node.key`, mode 0600.
pub fn node_key_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(IDENTITY_FOLDER).join(NODE_KEY_FILE)
}

pub fn paired_daemons_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(NODE_FOLDER).join(PAIRED_DAEMONS_FILE)
}

pub fn known_nodes_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(NODE_FOLDER).join(KNOWN_NODES_FILE)
}

/// `<data>/node/ssh_known_hosts`: an OpenSSH known-hosts file with the pinned host keys.
pub fn ssh_known_hosts_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(NODE_FOLDER).join(SSH_KNOWN_HOSTS_FILE)
}

pub fn hardware_profile_path(paths: &DataPaths) -> PathBuf {
    paths.data.join(NODE_FOLDER).join(HARDWARE_PROFILE_FILE)
}

/// `<data>/envs/`: every uv environment of this machine, one folder each.
pub fn environments_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(ENVIRONMENTS_FOLDER)
}

/// `<data>/envs/<environment_id>-<lock_hash12>/`: one uv environment.
pub fn environment_dir(paths: &DataPaths, environment_id: &str, lock_hash12: &str) -> PathBuf {
    let environment_folder = format!("{environment_id}-{lock_hash12}");
    environments_dir(paths).join(environment_folder)
}

/// `<data>/python/`: the uv-managed CPython 3.12.
pub fn python_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(PYTHON_FOLDER)
}

/// `<data>/bin/`: the node bundle on a box.
pub fn bin_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(BIN_FOLDER)
}

/// `<data>/plugins/<plugin_id>/<version>/`: unpacked plugin code. Replaceable; holds no user data.
pub fn plugin_dir(paths: &DataPaths, plugin_id: &str, version: &str) -> PathBuf {
    paths.data.join(PLUGINS_FOLDER).join(plugin_id).join(version)
}

/// `<data>/plugin-data/<plugin_id>/`: the plugin's private files, kept across versions.
pub fn plugin_data_dir(paths: &DataPaths, plugin_id: &str) -> PathBuf {
    paths.data.join(PLUGIN_DATA_FOLDER).join(plugin_id)
}

/// `<data>/engines/llama-server/<build>/`: one verified llama-server build.
pub fn llama_server_build_dir(paths: &DataPaths, build: &str) -> PathBuf {
    paths.data.join(ENGINES_FOLDER).join(LLAMA_SERVER_FOLDER).join(build)
}

/// `<data>/projects/`: where new projects go by default.
pub fn projects_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join(PROJECTS_FOLDER)
}

// ---- <cache> ----

pub fn torch_cache_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join(TORCH_FOLDER)
}

pub fn uv_cache_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join(UV_FOLDER)
}

/// `<cache>/tokenizers/<model_hash12>/`: tokenizer files for a model on another node.
pub fn tokenizer_cache_dir(paths: &DataPaths, model_hash12: &str) -> PathBuf {
    paths.cache.join(TOKENIZERS_FOLDER).join(model_hash12)
}

/// `<cache>/tokenizers/plugins/<plugin_id>/<tokenizer_id>/`: a plugin's tokenizer files.
pub fn plugin_tokenizer_cache_dir(paths: &DataPaths, plugin_id: &str, tokenizer_id: &str) -> PathBuf {
    let plugin_tokenizers = paths.cache.join(TOKENIZERS_FOLDER).join(PLUGINS_FOLDER);
    plugin_tokenizers.join(plugin_id).join(tokenizer_id)
}

/// `<cache>/hf-headers/<org>--<name>/<revision>/<file>.json`: a cached safetensors header.
pub fn hf_header_cache_path(paths: &DataPaths, org: &str, name: &str, revision: &str, file_name: &str) -> PathBuf {
    let repository_folder = format!("{org}{HF_REPOSITORY_SEPARATOR}{name}");
    let header_file = format!("{file_name}.{JSON_EXTENSION}");
    let repository_headers = paths.cache.join(HF_HEADERS_FOLDER).join(repository_folder);
    repository_headers.join(revision).join(header_file)
}

/// `<cache>/uploads/`: private data uploaded to a node.
pub fn uploads_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join(UPLOADS_FOLDER)
}

/// `<cache>/updates/`: downloaded update packages.
pub fn updates_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join(UPDATES_FOLDER)
}

/// `<cache>/commands-<app version>.json`: the CLI's help cache.
pub fn command_help_cache_path(paths: &DataPaths, app_version: &str) -> PathBuf {
    let help_file = format!("{COMMAND_HELP_STEM}-{app_version}.{JSON_EXTENSION}");
    paths.cache.join(help_file)
}

/// `<cache>/.locks/<repo folder>/<etag>.lock`: huggingface_hub's own per-file download lock, shared with HF tools.
pub fn hf_lock_path(paths: &DataPaths, repository_folder: &str, etag: &str) -> PathBuf {
    let lock_file = format!("{etag}.{LOCK_EXTENSION}");
    let repository_locks = paths.cache.join(HF_LOCKS_FOLDER).join(repository_folder);
    repository_locks.join(lock_file)
}

// ---- <state> ----

pub fn daemon_log_path(paths: &DataPaths) -> PathBuf {
    paths.state.join(LOGS_FOLDER).join(DAEMON_LOG_FILE)
}

pub fn node_log_path(paths: &DataPaths) -> PathBuf {
    paths.state.join(LOGS_FOLDER).join(NODE_LOG_FILE)
}

pub fn llama_server_log_path(paths: &DataPaths, job_id: &str) -> PathBuf {
    let log_file = format!("{job_id}.{LOG_EXTENSION}");
    paths.state.join(LOGS_FOLDER).join(LLAMA_SERVER_FOLDER).join(log_file)
}

/// `<state>/daemon.session.json`: deleted on a clean stop, so finding one at start means a crash.
pub fn daemon_session_path(paths: &DataPaths) -> PathBuf {
    paths.state.join(DAEMON_SESSION_FILE)
}

/// `<state>/crash-reports/`: local crash reports and panic files; never uploaded.
pub fn crash_reports_dir(paths: &DataPaths) -> PathBuf {
    paths.state.join(CRASH_REPORTS_FOLDER)
}

// ---- <runtime> ----

pub fn daemon_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(DAEMON_SOCKET_FILE)
}

pub fn daemon_lock_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(DAEMON_LOCK_FILE)
}

/// `<runtime>/daemon.port`: the port the daemon chose.
pub fn daemon_port_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(DAEMON_PORT_FILE)
}

pub fn node_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(NODE_SOCKET_FILE)
}

pub fn node_hub_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(NODE_HUB_SOCKET_FILE)
}

pub fn worker_host_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(WORKER_HOST_SOCKET_FILE)
}

pub fn job_socket_path(paths: &DataPaths, job_id: &str) -> PathBuf {
    let socket_file = format!("{job_id}.{SOCKET_EXTENSION}");
    paths.runtime.join(JOBS_FOLDER).join(socket_file)
}

/// `<runtime>/remote/<node_id>/node.sock`: the daemon's end of `ssh -L` to a remote node.
pub fn remote_node_socket_path(paths: &DataPaths, node_id: &str) -> PathBuf {
    paths.runtime.join(REMOTE_FOLDER).join(node_id).join(NODE_SOCKET_FILE)
}

/// `<runtime>/node-hub-<session_id>.sock` on a remote box: the node's end of `ssh -R`, fresh per SSH session.
pub fn remote_node_hub_socket_path(paths: &DataPaths, session_id: &str) -> PathBuf {
    let socket_file = format!("{NODE_HUB_SOCKET_STEM}-{session_id}.{SOCKET_EXTENSION}");
    paths.runtime.join(socket_file)
}

pub fn desktop_lock_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(DESKTOP_LOCK_FILE)
}

pub fn desktop_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join(DESKTOP_SOCKET_FILE)
}

// ---- Resolution ----

fn read_path_variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The home folder, else `RASTAI_HOME`: with it set, every root is under it, and the home folder only places the
/// default Hugging Face cache, which then sits under `RASTAI_HOME` too. With neither there is nowhere to put data.
fn choose_home(home_folder: Option<PathBuf>, rastai_home: &Option<PathBuf>) -> Result<PathBuf, CoreError> {
    match (home_folder, rastai_home) {
        (Some(home_folder), _) => Ok(home_folder),
        (None, Some(rastai_home)) => Ok(rastai_home.clone()),
        (None, None) => Err(CoreError::HomeNotFound),
    }
}

/// The owner of `/proc/self`, which proc(5) says is the process's effective user. None without `/proc`.
#[cfg(unix)]
fn read_user_id() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    let process_folder = std::fs::metadata(PROCESS_FOLDER).ok()?;
    Some(process_folder.uid())
}

/// Other platforms get their runtime rows of contracts 8.1 with their builds.
#[cfg(not(unix))]
fn read_user_id() -> Option<u32> {
    None
}

fn paths_under_one_folder(folder: &Path) -> DataPaths {
    DataPaths {
        config: folder.join(CONFIG_FOLDER),
        data: folder.join(DATA_FOLDER),
        cache: folder.join(CACHE_FOLDER),
        state: folder.join(STATE_FOLDER),
        runtime: folder.join(RUNTIME_FOLDER),
    }
}

/// `<XDG folder>/rastai`, or `<home>/<default>/rastai`. The XDG specification says a relative value is invalid
/// and must be ignored, so it falls back to the default too.
fn xdg_root(xdg_folder: &Option<PathBuf>, home: &Path, default_under_home: &str) -> PathBuf {
    match xdg_folder {
        Some(folder) if folder.is_absolute() => folder.join(APP_FOLDER),
        _ => home.join(default_under_home).join(APP_FOLDER),
    }
}

/// `$XDG_RUNTIME_DIR/rastai`. Without it (containers, `su`, some SSH sessions) `/tmp/rastai-<uid>`, which keeps
/// every socket path short; the transport creates it 0700 and refuses one another user owns (contracts 8.1).
/// `<state>/runtime` is the last resort, for a system without `/proc`, where the user id is unknown.
fn runtime_root(xdg_runtime_dir: &Option<PathBuf>, user_id: Option<u32>, state: &Path) -> PathBuf {
    match (xdg_runtime_dir, user_id) {
        (Some(folder), _) if folder.is_absolute() => folder.join(APP_FOLDER),
        (_, Some(user_id)) => Path::new(SHARED_TEMP_FOLDER).join(format!("{APP_FOLDER}-{user_id}")),
        (_, None) => state.join(RUNTIME_FOLDER),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::crockford::encode_base32;
    use crate::ids::node_id_from_public_key;

    // A Unix socket address (sockaddr_un) holds 108 bytes on Linux, the last one the terminating zero.
    const MAX_SOCKET_PATH_BYTES: usize = 107;
    // The longest values the socket rows take: the largest user id, ids of 26 base32 characters after their prefix.
    const LARGEST_USER_ID: u32 = u32::MAX;
    const LARGEST_RUNTIME_DIR: &str = "/run/user/4294967295";

    fn bare_environment(home: &str) -> PathEnvironment {
        PathEnvironment {
            rastai_home: None,
            home: PathBuf::from(home),
            xdg_config_home: None,
            xdg_data_home: None,
            xdg_cache_home: None,
            xdg_state_home: None,
            xdg_runtime_dir: None,
            hf_hub_cache: None,
            hf_home: None,
            user_id: Some(1000),
        }
    }

    fn full_xdg_environment() -> PathEnvironment {
        PathEnvironment {
            xdg_config_home: Some(PathBuf::from("/xdg/config")),
            xdg_data_home: Some(PathBuf::from("/xdg/data")),
            xdg_cache_home: Some(PathBuf::from("/xdg/cache")),
            xdg_state_home: Some(PathBuf::from("/xdg/state")),
            xdg_runtime_dir: Some(PathBuf::from("/run/user/1000")),
            ..bare_environment("/home/ada")
        }
    }

    /// Every socket path of contracts 8.1, with the longest ids each one takes.
    fn longest_socket_paths(paths: &DataPaths) -> Vec<PathBuf> {
        let longest_random_part = encode_base32(&[0xFF; 16]);
        let job_id = format!("job_{longest_random_part}");
        let session_id = format!("ses_{longest_random_part}");
        let node_id = node_id_from_public_key(&[0xFF; 32]);
        vec![
            daemon_socket_path(paths),
            node_socket_path(paths),
            node_hub_socket_path(paths),
            worker_host_socket_path(paths),
            desktop_socket_path(paths),
            job_socket_path(paths, &job_id),
            remote_node_socket_path(paths, &node_id),
            remote_node_hub_socket_path(paths, &session_id),
        ]
    }

    #[test]
    fn rastai_home_puts_all_five_roots_under_one_folder() {
        let environment = PathEnvironment {
            rastai_home: Some(PathBuf::from("/tmp/portable")),
            ..full_xdg_environment()
        };
        let paths = resolve_paths(&environment);
        assert_eq!(paths.config, Path::new("/tmp/portable/config"));
        assert_eq!(paths.data, Path::new("/tmp/portable/data"));
        assert_eq!(paths.cache, Path::new("/tmp/portable/cache"));
        assert_eq!(paths.state, Path::new("/tmp/portable/state"));
        assert_eq!(paths.runtime, Path::new("/tmp/portable/runtime"));
        // A relative RASTAI_HOME is kept as given: `just serve-dev` uses `.dev`.
        let relative = PathEnvironment {
            rastai_home: Some(PathBuf::from(".dev")),
            ..bare_environment("/home/ada")
        };
        assert_eq!(resolve_paths(&relative).data, Path::new(".dev/data"));
    }

    // Unix only: XDG folders are a Unix convention, and "/xdg/config" has no drive, so Windows reads it as relative
    // and ignores it, as it should. Windows gets its own rows of contracts 8.1 with its build.
    #[cfg(unix)]
    #[test]
    fn xdg_folders_are_used_when_set() {
        let paths = resolve_paths(&full_xdg_environment());
        assert_eq!(paths.config, Path::new("/xdg/config/rastai"));
        assert_eq!(paths.data, Path::new("/xdg/data/rastai"));
        assert_eq!(paths.cache, Path::new("/xdg/cache/rastai"));
        assert_eq!(paths.state, Path::new("/xdg/state/rastai"));
        assert_eq!(paths.runtime, Path::new("/run/user/1000/rastai"));
    }

    #[test]
    fn xdg_fallbacks_are_the_home_defaults() {
        let paths = resolve_paths(&bare_environment("/home/ada"));
        assert_eq!(paths.config, Path::new("/home/ada/.config/rastai"));
        assert_eq!(paths.data, Path::new("/home/ada/.local/share/rastai"));
        assert_eq!(paths.cache, Path::new("/home/ada/.cache/rastai"));
        assert_eq!(paths.state, Path::new("/home/ada/.local/state/rastai"));
        assert_eq!(paths.runtime, Path::new("/tmp/rastai-1000"));
    }

    #[test]
    fn runtime_without_a_user_id_is_under_state() {
        let environment = PathEnvironment {
            user_id: None,
            ..bare_environment("/home/ada")
        };
        let paths = resolve_paths(&environment);
        assert_eq!(paths.runtime, Path::new("/home/ada/.local/state/rastai/runtime"));
    }

    #[test]
    fn relative_xdg_folders_are_ignored() {
        let environment = PathEnvironment {
            xdg_data_home: Some(PathBuf::from("relative/data")),
            xdg_runtime_dir: Some(PathBuf::from("run")),
            ..bare_environment("/home/ada")
        };
        let paths = resolve_paths(&environment);
        assert_eq!(paths.data, Path::new("/home/ada/.local/share/rastai"));
        assert_eq!(paths.runtime, Path::new("/tmp/rastai-1000"));
    }

    #[test]
    fn socket_paths_fit_a_unix_socket_address() {
        // A long home folder must not matter when XDG_RUNTIME_DIR is unset: the sockets don't go under it.
        let without_runtime_dir = PathEnvironment {
            user_id: Some(LARGEST_USER_ID),
            ..bare_environment("/home/a-long-user-name-of-the-kind-some-sites-hand-out")
        };
        let with_runtime_dir = PathEnvironment {
            xdg_runtime_dir: Some(PathBuf::from(LARGEST_RUNTIME_DIR)),
            ..without_runtime_dir.clone()
        };
        for environment in [without_runtime_dir, with_runtime_dir] {
            for socket_path in longest_socket_paths(&resolve_paths(&environment)) {
                let length = socket_path.as_os_str().len();
                assert!(length <= MAX_SOCKET_PATH_BYTES, "{length} bytes: {socket_path:?}");
            }
        }
    }

    #[test]
    fn home_comes_from_home_then_rastai_home() {
        let home = Some(PathBuf::from("/home/ada"));
        let rastai_home = Some(PathBuf::from("/portable"));
        assert_eq!(choose_home(home.clone(), &None).unwrap(), Path::new("/home/ada"));
        assert_eq!(choose_home(home, &rastai_home).unwrap(), Path::new("/home/ada"));
        // No HOME and no user database entry (some containers): RASTAI_HOME is enough to run.
        assert_eq!(choose_home(None, &rastai_home).unwrap(), Path::new("/portable"));
        assert_eq!(choose_home(None, &None), Err(CoreError::HomeNotFound));
    }

    #[test]
    fn hf_cache_order() {
        let mut environment = bare_environment("/home/ada");
        assert_eq!(
            hf_cache_dir(&environment),
            Path::new("/home/ada/.cache/huggingface/hub")
        );
        environment.hf_home = Some(PathBuf::from("/models/hf"));
        assert_eq!(hf_cache_dir(&environment), Path::new("/models/hf/hub"));
        environment.hf_hub_cache = Some(PathBuf::from("/fast/hub"));
        assert_eq!(hf_cache_dir(&environment), Path::new("/fast/hub"));
        // RASTAI_HOME moves our folders, not the shared HF cache.
        environment.rastai_home = Some(PathBuf::from("/tmp/portable"));
        assert_eq!(hf_cache_dir(&environment), Path::new("/fast/hub"));
    }

    /// The roots under `/r`, so every expected path below reads as `<root folder>/<row>`.
    fn paths_under_r() -> DataPaths {
        resolve_paths(&PathEnvironment {
            rastai_home: Some(PathBuf::from("/r")),
            ..bare_environment("/home/ada")
        })
    }

    fn assert_rows(cases: &[(PathBuf, &str)]) {
        for (built, expected) in cases {
            assert_eq!(built, Path::new(expected));
        }
    }

    #[test]
    fn config_rows_join_under_config() {
        let paths = paths_under_r();
        assert_rows(&[
            (user_settings_path(&paths), "/r/config/settings.json"),
            (owner_token_path(&paths), "/r/config/auth/owner.token"),
            (encrypted_secrets_path(&paths), "/r/config/auth/secrets.enc"),
            (
                chat_template_override_path(&paths, "abcdef012345"),
                "/r/config/chat-templates/abcdef012345.jinja",
            ),
        ]);
    }

    #[test]
    fn data_rows_join_under_data() {
        let paths = paths_under_r();
        assert_rows(&[
            (rastai_db_path(&paths), "/r/data/rastai.db"),
            (
                rastai_db_pre_update_path(&paths, "0.2.0"),
                "/r/data/rastai.db.pre-0.2.0",
            ),
            (vault_db_pre_update_path(&paths, "0.2.0"), "/r/data/vault.db.pre-0.2.0"),
            (
                rastai_db_broken_path(&paths, 1_791_633_600_000),
                "/r/data/rastai.db.broken-1791633600000",
            ),
            (rastai_db_rebuilding_path(&paths), "/r/data/rastai.db.rebuilding"),
            (rastai_db_salvage_path(&paths), "/r/data/rastai.db.salvage"),
            (runs_dir(&paths), "/r/data/runs"),
            (run_events_log_path(&paths, "run_01"), "/r/data/runs/run_01/events.log"),
            (daemon_events_log_path(&paths), "/r/data/daemon/events.log"),
            (daemon_events_segment_path(&paths, 3), "/r/data/daemon/events-3.log"),
            (daemon_key_path(&paths), "/r/data/identity/daemon.key"),
            (paired_daemons_path(&paths), "/r/data/node/paired_daemons.json"),
            (environments_dir(&paths), "/r/data/envs"),
            (
                environment_dir(&paths, "cpu", "0123456789ab"),
                "/r/data/envs/cpu-0123456789ab",
            ),
            (plugin_dir(&paths, "eval", "1.2.0"), "/r/data/plugins/eval/1.2.0"),
            (plugin_data_dir(&paths, "eval"), "/r/data/plugin-data/eval"),
            (
                llama_server_build_dir(&paths, "b4000"),
                "/r/data/engines/llama-server/b4000",
            ),
        ]);
    }

    #[test]
    fn cache_rows_join_under_cache() {
        let paths = paths_under_r();
        assert_rows(&[
            (
                plugin_tokenizer_cache_dir(&paths, "eval", "words"),
                "/r/cache/tokenizers/plugins/eval/words",
            ),
            (
                hf_header_cache_path(&paths, "org", "tiny", "main", "model.safetensors"),
                "/r/cache/hf-headers/org--tiny/main/model.safetensors.json",
            ),
            (
                hf_lock_path(&paths, "models--org--tiny", "e1"),
                "/r/cache/.locks/models--org--tiny/e1.lock",
            ),
            (command_help_cache_path(&paths, "0.1.0"), "/r/cache/commands-0.1.0.json"),
        ]);
    }

    #[test]
    fn state_and_runtime_rows_join_under_their_roots() {
        let paths = paths_under_r();
        assert_rows(&[
            (daemon_log_path(&paths), "/r/state/logs/daemon.jsonl"),
            (
                llama_server_log_path(&paths, "job_01"),
                "/r/state/logs/llama-server/job_01.log",
            ),
            (daemon_session_path(&paths), "/r/state/daemon.session.json"),
            (daemon_socket_path(&paths), "/r/runtime/daemon.sock"),
            (job_socket_path(&paths, "job_01"), "/r/runtime/jobs/job_01.sock"),
            (
                remote_node_socket_path(&paths, "node_7Z"),
                "/r/runtime/remote/node_7Z/node.sock",
            ),
            (
                remote_node_hub_socket_path(&paths, "ses_01"),
                "/r/runtime/node-hub-ses_01.sock",
            ),
        ]);
    }
}
