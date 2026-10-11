//! The only place data paths are built (contracts 8.1, ADR-0014). No other code reads `RASTAI_HOME`, `HOME`,
//! `XDG_*`, `HF_HUB_CACHE` or `HF_HOME`, or joins a well-known folder name.
//!
//! `read_path_environment` reads the variables once; `resolve_paths` turns them into the five roots, without
//! touching the disk, so tests resolve any layout from a plain struct. Every other function here is one row of the
//! contracts 8.1 table: one root and fixed names, plus the ids the row names. Callers pass ids they have checked
//! (`id_has_prefix`, the plugin id grammar), because an id is joined into the path as it is.
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

// The Hugging Face hub cache: HF_HUB_CACHE, else HF_HOME/hub, else ~/.cache/huggingface/hub (spike hf_cache.py).
const HF_HUB_FOLDER: &str = "hub";
const DEFAULT_HF_HOME: &str = ".cache/huggingface";

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

/// The path variables of this process. The home folder comes from `HOME`, else the user database; with neither,
/// this fails with `HomeNotFound` (set `RASTAI_HOME` to run without a home folder's defaults).
pub fn read_path_environment() -> Result<PathEnvironment, CoreError> {
    let home = std::env::home_dir().filter(|folder| !folder.as_os_str().is_empty());
    let Some(home) = home else {
        return Err(CoreError::HomeNotFound);
    };
    Ok(PathEnvironment {
        rastai_home: read_path_variable(RASTAI_HOME_VARIABLE),
        home,
        xdg_config_home: read_path_variable(XDG_CONFIG_HOME_VARIABLE),
        xdg_data_home: read_path_variable(XDG_DATA_HOME_VARIABLE),
        xdg_cache_home: read_path_variable(XDG_CACHE_HOME_VARIABLE),
        xdg_state_home: read_path_variable(XDG_STATE_HOME_VARIABLE),
        xdg_runtime_dir: read_path_variable(XDG_RUNTIME_DIR_VARIABLE),
        hf_hub_cache: read_path_variable(HF_HUB_CACHE_VARIABLE),
        hf_home: read_path_variable(HF_HOME_VARIABLE),
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
        runtime: runtime_root(&environment.xdg_runtime_dir, &state),
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
    paths.config.join("settings.json")
}

pub fn keybindings_path(paths: &DataPaths) -> PathBuf {
    paths.config.join("keybindings.json")
}

pub fn layouts_dir(paths: &DataPaths) -> PathBuf {
    paths.config.join("layouts")
}

/// `<config>/chat-templates/<model_hash12>.jinja`: the user's chat template override for one model.
pub fn chat_template_override_path(paths: &DataPaths, model_hash12: &str) -> PathBuf {
    paths
        .config
        .join("chat-templates")
        .join(format!("{model_hash12}.jinja"))
}

pub fn desktop_window_path(paths: &DataPaths) -> PathBuf {
    paths.config.join("desktop-window.json")
}

/// `<config>/auth/owner.token`, mode 0600.
pub fn owner_token_path(paths: &DataPaths) -> PathBuf {
    paths.config.join("auth").join("owner.token")
}

/// `<config>/auth/secrets.enc`: the encrypted secret store on headless Linux (contracts 8.5).
pub fn encrypted_secrets_path(paths: &DataPaths) -> PathBuf {
    paths.config.join("auth").join("secrets.enc")
}

// ---- <data> ----

pub fn rastai_db_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("rastai.db")
}

pub fn vault_db_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("vault.db")
}

pub fn blobs_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join("blobs")
}

/// `<data>/runs/<run_id>/` (contracts 8.4).
pub fn run_dir(paths: &DataPaths, run_id: &str) -> PathBuf {
    paths.data.join("runs").join(run_id)
}

pub fn run_events_log_path(paths: &DataPaths, run_id: &str) -> PathBuf {
    run_dir(paths, run_id).join("events.log")
}

/// `<data>/daemon/events.log`: the daemon's durable event log; its rolled segments sit beside it.
pub fn daemon_events_log_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("daemon").join("events.log")
}

/// `<data>/identity/daemon.key`, mode 0600.
pub fn daemon_key_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("identity").join("daemon.key")
}

/// `<data>/identity/node.key`, mode 0600.
pub fn node_key_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("identity").join("node.key")
}

pub fn paired_daemons_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("node").join("paired_daemons.json")
}

pub fn known_nodes_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("node").join("known_nodes.json")
}

/// `<data>/node/ssh_known_hosts`: an OpenSSH known-hosts file with the pinned host keys.
pub fn ssh_known_hosts_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("node").join("ssh_known_hosts")
}

pub fn hardware_profile_path(paths: &DataPaths) -> PathBuf {
    paths.data.join("node").join("hardware_profile.json")
}

/// `<data>/envs/<environment_id>-<lock_hash12>/`: one uv environment.
pub fn environment_dir(paths: &DataPaths, environment_id: &str, lock_hash12: &str) -> PathBuf {
    paths.data.join("envs").join(format!("{environment_id}-{lock_hash12}"))
}

/// `<data>/python/`: the uv-managed CPython 3.12.
pub fn python_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join("python")
}

/// `<data>/bin/`: the node bundle on a box.
pub fn bin_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join("bin")
}

/// `<data>/plugins/<plugin_id>/<version>/`: unpacked plugin code. Replaceable; holds no user data.
pub fn plugin_dir(paths: &DataPaths, plugin_id: &str, version: &str) -> PathBuf {
    paths.data.join("plugins").join(plugin_id).join(version)
}

/// `<data>/plugin-data/<plugin_id>/`: the plugin's private files, kept across versions.
pub fn plugin_data_dir(paths: &DataPaths, plugin_id: &str) -> PathBuf {
    paths.data.join("plugin-data").join(plugin_id)
}

/// `<data>/engines/llama-server/<build>/`: one verified llama-server build.
pub fn llama_server_build_dir(paths: &DataPaths, build: &str) -> PathBuf {
    paths.data.join("engines").join("llama-server").join(build)
}

/// `<data>/projects/`: where new projects go by default.
pub fn projects_dir(paths: &DataPaths) -> PathBuf {
    paths.data.join("projects")
}

// ---- <cache> ----

pub fn torch_cache_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join("torch")
}

pub fn uv_cache_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join("uv")
}

/// `<cache>/tokenizers/<model_hash12>/`: tokenizer files for a model on another node.
pub fn tokenizer_cache_dir(paths: &DataPaths, model_hash12: &str) -> PathBuf {
    paths.cache.join("tokenizers").join(model_hash12)
}

/// `<cache>/tokenizers/plugins/<plugin_id>/<tokenizer_id>/`: a plugin's tokenizer files.
pub fn plugin_tokenizer_cache_dir(paths: &DataPaths, plugin_id: &str, tokenizer_id: &str) -> PathBuf {
    paths
        .cache
        .join("tokenizers")
        .join("plugins")
        .join(plugin_id)
        .join(tokenizer_id)
}

/// `<cache>/hf-headers/<org>--<name>/<revision>/<file>.json`: a cached safetensors header.
pub fn hf_header_cache_path(paths: &DataPaths, org: &str, name: &str, revision: &str, file_name: &str) -> PathBuf {
    let repository_folder = format!("{org}--{name}");
    let header_file = format!("{file_name}.json");
    paths
        .cache
        .join("hf-headers")
        .join(repository_folder)
        .join(revision)
        .join(header_file)
}

/// `<cache>/uploads/`: private data uploaded to a node.
pub fn uploads_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join("uploads")
}

/// `<cache>/updates/`: downloaded update packages.
pub fn updates_dir(paths: &DataPaths) -> PathBuf {
    paths.cache.join("updates")
}

/// `<cache>/commands-<app version>.json`: the CLI's help cache.
pub fn command_help_cache_path(paths: &DataPaths, app_version: &str) -> PathBuf {
    paths.cache.join(format!("commands-{app_version}.json"))
}

/// `<cache>/.locks/<repo folder>/<etag>.lock`: huggingface_hub's own per-file download lock, shared with HF tools.
pub fn hf_lock_path(paths: &DataPaths, repository_folder: &str, etag: &str) -> PathBuf {
    paths
        .cache
        .join(".locks")
        .join(repository_folder)
        .join(format!("{etag}.lock"))
}

// ---- <state> ----

pub fn daemon_log_path(paths: &DataPaths) -> PathBuf {
    paths.state.join("logs").join("daemon.jsonl")
}

pub fn node_log_path(paths: &DataPaths) -> PathBuf {
    paths.state.join("logs").join("node.jsonl")
}

pub fn llama_server_log_path(paths: &DataPaths, job_id: &str) -> PathBuf {
    paths
        .state
        .join("logs")
        .join("llama-server")
        .join(format!("{job_id}.log"))
}

/// `<state>/daemon.session.json`: deleted on a clean stop, so finding one at start means a crash.
pub fn daemon_session_path(paths: &DataPaths) -> PathBuf {
    paths.state.join("daemon.session.json")
}

/// `<state>/crash-reports/`: local crash reports and panic files; never uploaded.
pub fn crash_reports_dir(paths: &DataPaths) -> PathBuf {
    paths.state.join("crash-reports")
}

// ---- <runtime> ----

pub fn daemon_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("daemon.sock")
}

pub fn daemon_lock_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("daemon.lock")
}

/// `<runtime>/daemon.port`: the port the daemon chose.
pub fn daemon_port_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("daemon.port")
}

pub fn node_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("node.sock")
}

pub fn node_hub_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("node-hub.sock")
}

pub fn worker_host_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("worker-host.sock")
}

pub fn job_socket_path(paths: &DataPaths, job_id: &str) -> PathBuf {
    paths.runtime.join("jobs").join(format!("{job_id}.sock"))
}

/// `<runtime>/remote/<node_id>/node.sock`: the daemon's end of `ssh -L` to a remote node.
pub fn remote_node_socket_path(paths: &DataPaths, node_id: &str) -> PathBuf {
    paths.runtime.join("remote").join(node_id).join("node.sock")
}

/// `<runtime>/node-hub-<session_id>.sock` on a remote box: the node's end of `ssh -R`, fresh per SSH session.
pub fn remote_node_hub_socket_path(paths: &DataPaths, session_id: &str) -> PathBuf {
    paths.runtime.join(format!("node-hub-{session_id}.sock"))
}

pub fn desktop_lock_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("desktop.lock")
}

pub fn desktop_socket_path(paths: &DataPaths) -> PathBuf {
    paths.runtime.join("desktop.sock")
}

// ---- Resolution ----

fn read_path_variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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

/// `$XDG_RUNTIME_DIR/rastai`. Without it (containers, some SSH sessions) the sockets go in `<state>/runtime`: the
/// folder must be private to the user, which a shared /tmp isn't.
fn runtime_root(xdg_runtime_dir: &Option<PathBuf>, state: &Path) -> PathBuf {
    match xdg_runtime_dir {
        Some(folder) if folder.is_absolute() => folder.join(APP_FOLDER),
        _ => state.join(RUNTIME_FOLDER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn roots(paths: &DataPaths) -> [&Path; 5] {
        [&paths.config, &paths.data, &paths.cache, &paths.state, &paths.runtime]
    }

    #[test]
    fn rastai_home_puts_all_five_roots_under_one_folder() {
        let environment = PathEnvironment {
            rastai_home: Some(PathBuf::from("/tmp/portable")),
            ..full_xdg_environment()
        };
        let paths = resolve_paths(&environment);
        let expected =
            ["config", "data", "cache", "state", "runtime"].map(|name| Path::new("/tmp/portable").join(name));
        assert_eq!(roots(&paths), expected.each_ref().map(PathBuf::as_path));
        // A relative RASTAI_HOME is kept as given: `just serve-dev` uses `.dev`.
        let relative = PathEnvironment {
            rastai_home: Some(PathBuf::from(".dev")),
            ..bare_environment("/home/ada")
        };
        assert_eq!(resolve_paths(&relative).data, Path::new(".dev/data"));
    }

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
        assert_eq!(paths.runtime, Path::new("/home/ada/.local/state/rastai/runtime"));
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

    #[test]
    fn rows_join_under_their_root() {
        let paths = resolve_paths(&PathEnvironment {
            rastai_home: Some(PathBuf::from("/r")),
            ..bare_environment("/home/ada")
        });
        let cases = [
            (user_settings_path(&paths), "/r/config/settings.json"),
            (owner_token_path(&paths), "/r/config/auth/owner.token"),
            (
                chat_template_override_path(&paths, "abcdef012345"),
                "/r/config/chat-templates/abcdef012345.jinja",
            ),
            (rastai_db_path(&paths), "/r/data/rastai.db"),
            (run_events_log_path(&paths, "run_01"), "/r/data/runs/run_01/events.log"),
            (daemon_events_log_path(&paths), "/r/data/daemon/events.log"),
            (daemon_key_path(&paths), "/r/data/identity/daemon.key"),
            (
                environment_dir(&paths, "cpu", "0123456789ab"),
                "/r/data/envs/cpu-0123456789ab",
            ),
            (plugin_dir(&paths, "eval", "1.2.0"), "/r/data/plugins/eval/1.2.0"),
            (plugin_data_dir(&paths, "eval"), "/r/data/plugin-data/eval"),
            (
                hf_header_cache_path(&paths, "org", "tiny", "main", "model.safetensors"),
                "/r/cache/hf-headers/org--tiny/main/model.safetensors.json",
            ),
            (
                hf_lock_path(&paths, "models--org--tiny", "e1"),
                "/r/cache/.locks/models--org--tiny/e1.lock",
            ),
            (command_help_cache_path(&paths, "0.1.0"), "/r/cache/commands-0.1.0.json"),
            (daemon_log_path(&paths), "/r/state/logs/daemon.jsonl"),
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
        ];
        for (built, expected) in cases {
            assert_eq!(built, Path::new(expected));
        }
    }
}
