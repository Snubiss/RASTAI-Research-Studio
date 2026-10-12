//! Finding plugins and reading their manifests (layout: rust-control-plane.md 12.1). Discovery starts nothing: it
//! fills `PluginHost` with what it found, and `load_all_plugins` (load.rs) decides what each plugin becomes.
//!
//! | Source | Where | Granted permissions |
//! |---|---|---|
//! | installed and preinstalled | a `plugins` row and its folder `<data>/plugins/<plugin_id>/<version>/` | the row's |
//! | dev | each folder in `plugins.dev_paths` | what its manifest asks for |
//!
//! A dev folder runs with what its manifest asks for because only the owner can name one (`plugins.dev_paths` is an
//! owner-only setting, contracts 11.1). Unpacking the archives shipped under the install prefix into preinstalled rows
//! arrives with the first-party plugins (M4-E02).
//!
//! Every manifest is read through rastai-formats' four layers. One that can't be read, or isn't a plugin manifest,
//! is kept with `RRS-P0001` and its `RRS-E` details; its plugin id is then the folder's name, so `plugin.list` can
//! still show it.

use std::path::Path;

use rastai_core::paths::{DataPaths, plugin_dir};
use rastai_core::settings::ResolvedSettings;
use rastai_core::states::parse_plugin_state;
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::Document;
use rastai_formats::documents::plugin_manifest::PluginManifest;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::meaning::plugin_manifest::is_plugin_id;
use rastai_formats::validate::read_document;
use rastai_proto::common::v1::PluginState;
use rastai_store::index::kept_tables::{PluginRow, get_plugin};
use rastai_store::index::reads::open_index_reader;

use crate::error::ServerError;
use crate::plugin_host::load::{LoadedPlugin, PluginSource, write_plugin_host};
use crate::server_state::Server;

const MANIFEST_FILE_NAME: &str = "rastai-plugin.json";
const PLUGINS_SECTION: &str = "plugins";
const DEV_PATHS_KEY: &str = "dev_paths";
/// The folder `plugin_dir` puts every installed plugin's version folders in.
const INSTALLED_PLUGINS_FOLDER: &str = "plugins";
const PREINSTALLED_SOURCE: &str = "preinstalled";
const CODE_MANIFEST_INVALID: &str = "RRS-P0001";
const CODE_PLUGIN_ID_TAKEN: &str = "RRS-P0002";
const NOT_A_MANIFEST: &str = "the file isn't a rastai.plugin document";
const OTHER_PLUGIN_ID: &str = "the manifest names a different plugin id than its installed row";

/// Reads every plugin's manifest and saved state into `server.plugins`: installed and preinstalled ones first, then
/// the dev folders. A plugin id found twice keeps its first place; the later copy is logged with `RRS-P0002`.
pub fn discover_plugins(server: &Server) -> Result<(), ServerError> {
    let mut found = find_installed_plugins(server)?;
    let dev_paths = match server.settings.read() {
        Ok(settings) => dev_paths_setting(&settings),
        Err(poisoned) => dev_paths_setting(&poisoned.into_inner()),
    };
    found.extend(dev_paths.iter().map(|folder| read_dev_plugin(Path::new(folder))));
    let mut host = write_plugin_host(server);
    for plugin in found {
        if host.plugins.iter().any(|known| known.plugin_id == plugin.plugin_id) {
            log_duplicate(&plugin);
            continue;
        }
        host.plugins.push(plugin);
    }
    Ok(())
}

/// `plugins.dev_paths`: the folders as written; anything in the list that isn't text is left out.
pub(crate) fn dev_paths_setting(settings: &ResolvedSettings) -> Vec<String> {
    let listed = settings.values[PLUGINS_SECTION][DEV_PATHS_KEY].as_array();
    listed
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| path.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A dev folder's plugin, ready for its first load: `installed`, granted what its manifest asks for.
pub(crate) fn read_dev_plugin(folder: &Path) -> LoadedPlugin {
    let (manifest, manifest_problems) = read_manifest(folder);
    let mut plugin = new_plugin(folder, PluginSource::Dev, manifest, manifest_problems);
    if let Some(manifest) = &plugin.manifest {
        plugin.granted_permissions = manifest.permissions.clone();
    }
    plugin
}

/// The plugins with a `plugins` row and a folder under `<data>/plugins/`. A folder without a row is an install that
/// never finished (M8 cleans it up) and is left alone, and so is a row that says `uninstalled`.
fn find_installed_plugins(server: &Server) -> Result<Vec<LoadedPlugin>, ServerError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    let mut found = Vec::new();
    for plugin_id in installed_plugin_ids(&server.paths) {
        let Some(row) = get_plugin(&reader, &plugin_id)? else {
            continue;
        };
        if parse_plugin_state(&row.state) == Some(PluginState::Uninstalled) {
            continue;
        }
        found.push(read_installed_plugin(&server.paths, row));
    }
    Ok(found)
}

/// The folder names under `<data>/plugins/` that are plugin ids, sorted so discovery's order is the same every start.
fn installed_plugin_ids(paths: &DataPaths) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(paths.data.join(INSTALLED_PLUGINS_FOLDER)) else {
        return Vec::new();
    };
    let mut plugin_ids: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_plugin_id(name))
        .collect();
    plugin_ids.sort();
    plugin_ids
}

fn read_installed_plugin(paths: &DataPaths, row: PluginRow) -> LoadedPlugin {
    let folder = plugin_dir(paths, &row.plugin_id, &row.version);
    let (mut manifest, mut manifest_problems) = read_manifest(&folder);
    // The row decides who the plugin is: a manifest that claims another id would take that plugin's place.
    if manifest.as_ref().is_some_and(|read| read.plugin_id != row.plugin_id) {
        manifest = None;
        manifest_problems = vec![manifest_invalid(&row.plugin_id, OTHER_PLUGIN_ID)];
    }
    let source = if row.source == PREINSTALLED_SOURCE {
        PluginSource::Preinstalled
    } else {
        PluginSource::Installed
    };
    let mut plugin = new_plugin(&folder, source, manifest, manifest_problems);
    plugin.plugin_id = row.plugin_id;
    plugin.state = parse_plugin_state(&row.state).unwrap_or(PluginState::Installed);
    plugin.granted_permissions = row.granted_permissions;
    plugin.content_hash = row.content_hash;
    if plugin.version.is_empty() {
        plugin.version = row.version;
    }
    plugin
}

fn new_plugin(
    folder: &Path,
    source: PluginSource,
    manifest: Option<PluginManifest>,
    manifest_problems: Vec<Diagnostic>,
) -> LoadedPlugin {
    let (plugin_id, version, event_handlers) = match &manifest {
        Some(manifest) => (
            manifest.plugin_id.clone(),
            manifest.version.clone(),
            manifest.contributes.event_handlers.clone(),
        ),
        None => (folder_name(folder), String::new(), Vec::new()),
    };
    LoadedPlugin {
        plugin_id,
        version,
        source,
        folder: folder.to_path_buf(),
        manifest,
        state: PluginState::Installed,
        granted_permissions: Vec::new(),
        content_hash: String::new(),
        diagnostics: Vec::new(),
        manifest_problems,
        event_handlers,
    }
}

/// The manifest and every problem reading it found. None with `RRS-P0001` first when there is no document to load:
/// the file is missing or unreadable, or layers 1 and 2 refused it. Layer-3 problems come back with the manifest,
/// and the load refuses it for them (`incompatible`).
fn read_manifest(folder: &Path) -> (Option<PluginManifest>, Vec<Diagnostic>) {
    let path = folder.join(MANIFEST_FILE_NAME);
    let fallback_id = folder_name(folder);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let reason = format!("{} can't be read: {error}", path.display());
            return (None, vec![manifest_invalid(&fallback_id, &reason)]);
        }
    };
    let file_name = path.display().to_string();
    let outcome = read_document(&file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
    match outcome.document {
        Some(Document::PluginManifest(manifest)) => (Some(manifest), outcome.diagnostics),
        Some(_) => (None, vec![manifest_invalid(&fallback_id, NOT_A_MANIFEST)]),
        None => (None, refused_manifest(&fallback_id, outcome.diagnostics)),
    }
}

/// `RRS-P0001` naming the first error, followed by every finding.
fn refused_manifest(plugin_id: &str, findings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let first_error = findings.iter().find(|item| item.severity == Severity::Error);
    let reason = first_error.map_or(NOT_A_MANIFEST.to_string(), |item| item.message.clone());
    let mut diagnostics = vec![manifest_invalid(plugin_id, &reason)];
    diagnostics.extend(findings);
    diagnostics
}

fn manifest_invalid(plugin_id: &str, reason: &str) -> Diagnostic {
    diagnostic(CODE_MANIFEST_INVALID, &[("plugin_id", plugin_id), ("reason", reason)])
}

fn folder_name(folder: &Path) -> String {
    let name = folder.file_name().map(|name| name.to_string_lossy().to_string());
    name.unwrap_or_else(|| folder.display().to_string())
}

fn log_duplicate(plugin: &LoadedPlugin) {
    log_warn(
        "a second plugin with the same id was left out",
        &[
            ("plugin_id", &plugin.plugin_id),
            ("folder", &plugin.folder.display().to_string()),
            ("code", CODE_PLUGIN_ID_TAKEN),
        ],
    );
}
