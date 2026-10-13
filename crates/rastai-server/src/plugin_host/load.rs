//! Loading plugins: the decision chain of flows/platform.md 7.3 for each plugin discovery found, in load order
//! (layout: rust-control-plane.md 12.1 and 12.4). `PluginHost`, the server's table of plugins, lives here too.
//!
//! A load reads the manifest again (`RRS-P0001` when it couldn't be read, its layer-3 problems leave it
//! `incompatible`), negotiates capabilities (capabilities.rs), then registers the contributions of a plugin that may
//! load (contributions.rs) and moves it to its new state (lifecycle.rs). A failure never stops the loop or the daemon:
//! the plugin keeps its state and its diagnostics, and `plugin.list` shows them.
//!
//! Only the user's decisions reach a plugin's `plugins` row: a load that enables it, and `disable_plugin`. What the
//! host decides on a load (`incompatible`, `failed`, `awaiting_trust`, or `disabled` for `RRS-P1002`, `RRS-P1003` or
//! `RRS-P3001`) stays in memory and is decided again at the next start, so a plugin comes back once its fix (a setting,
//! a dependency, an app update) is in place.
//!
//! Starting a plugin's Python module is the M4 step after registration (`plugin_module` jobs, forward.rs); until then
//! a plugin with a `python` entry loads its declarations only, and its commands answer `RRS-P1004`. The isolation rule
//! that start follows (contracts 8.8) is `check_module_isolation`, ready for it.

use std::path::PathBuf;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use rastai_core::commands::registry::{CommandRegistry, remove_plugin_commands};
use rastai_core::scopes::Scope;
use rastai_core::settings::setting_bool;
use rastai_diag::diagnostic::{Diagnostic, diagnostic, has_errors};
use rastai_formats::documents::plugin_manifest::{EventHandlerContribution, PluginManifest};
use rastai_formats::semver::{Version, parse_version};
use rastai_proto::common::v1::PluginState;
use rastai_proto::events::v1::PluginStateChanged;
use rastai_store::index::kept_tables::PluginRow;

use crate::plugin_host::capabilities::{
    available_capabilities, is_first_party_plugin_id, negotiate_capabilities, negotiation_diagnostics,
    state_after_negotiation,
};
use crate::plugin_host::contributions::{plugin_scopes, register_plugin_commands};
use crate::plugin_host::lifecycle::{
    change_plugin_state, plugin_row, record_plugin_state_changes, record_state_change, save_disabled_plugin,
    save_plugin_row,
};
use crate::server_state::Server;

const EXPERIMENTAL_APIS_SETTING: &str = "plugins.experimental_apis";
const CODE_DEPENDENCY_MISSING: &str = "RRS-P3001";
const CODE_NO_NAMESPACES_THIRD_PARTY: &str = "RRS-P2001";
const CODE_NO_NAMESPACES_FIRST_PARTY: &str = "RRS-P2005";
/// A granted permission of this form gives the plugin's module the network (contracts 6.3).
const NETWORK_PERMISSION_PREFIX: &str = "network:";
/// The states a load may start from. A `disabled` plugin is loaded only when asked (`plugin.enable`, M8), never at
/// startup: a `disabled` row is the user's decision (the host's own outcomes are never saved).
const LOADABLE_STATES: &[PluginState] = &[PluginState::Installed, PluginState::Enabled, PluginState::Disabled];
const STARTUP_STATES: &[PluginState] = &[PluginState::Installed, PluginState::Enabled];

/// Where the host found a plugin (contracts 6.5, `PluginSummary.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    /// Shipped with the app; a `plugins` row with source `preinstalled`.
    Preinstalled,
    /// Installed by the user; a `plugins` row with source `installed`.
    Installed,
    /// A folder in `plugins.dev_paths`; no row, found again at every start.
    Dev,
}

/// One plugin the host knows, whatever its state.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPlugin {
    pub plugin_id: String,
    /// The manifest's version, or the row's when the manifest couldn't be read.
    pub version: String,
    pub source: PluginSource,
    pub folder: PathBuf,
    /// None when the manifest couldn't be read (`RRS-P0001`).
    pub manifest: Option<PluginManifest>,
    pub state: PluginState,
    pub granted_permissions: Vec<String>,
    /// The installed archive's hash; empty for a dev folder.
    pub content_hash: String,
    /// Why it isn't enabled, or what it couldn't register; empty when it loaded cleanly.
    pub diagnostics: Vec<Diagnostic>,
    /// What reading the manifest found (layers 1 to 3), kept so every load checks it again.
    pub manifest_problems: Vec<Diagnostic>,
    /// The manifest's event handlers, which the host runs from M5 (layout 12.5).
    pub event_handlers: Vec<EventHandlerContribution>,
}

/// The server's table of plugins (layout 8.2), behind `Server.plugins`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginHost {
    pub plugins: Vec<LoadedPlugin>,
    /// `plugin.state_changed` records made under this table's lock and not yet in the daemon log, newest last.
    /// `record_plugin_state_changes` takes them once the lock is released (lifecycle.rs).
    pub state_changes: Vec<PluginStateChanged>,
}

/// The word `PluginSummary.source` and the `plugins` row use.
pub(crate) fn plugin_source_name(source: PluginSource) -> &'static str {
    match source {
        PluginSource::Preinstalled => "preinstalled",
        PluginSource::Installed => "installed",
        PluginSource::Dev => "dev",
    }
}

/// Loads every plugin discovery found, first-party first, then each after the plugins it depends on. A plugin in a
/// dependency cycle, or depending on one, is disabled with `RRS-P3001` naming the dependency that can't be ordered.
pub async fn load_all_plugins(server: &Server) {
    let (order, blocked) = {
        let host = read_plugin_host(server);
        plugin_load_order(&host.plugins)
    };
    for (plugin_id, dependency_id) in &blocked {
        refuse_dependency_cycle(server, plugin_id, dependency_id).await;
    }
    for plugin_id in &order {
        let starts_now = {
            let host = read_plugin_host(server);
            find_plugin(&host, plugin_id).is_some_and(|plugin| STARTUP_STATES.contains(&plugin.state))
        };
        if starts_now {
            load_plugin(server, plugin_id).await;
        }
    }
}

/// Runs the decision chain for one plugin and returns the state it ends in. A plugin that may load has its
/// contributions registered; any other outcome removes them, so a command never outlives its plugin.
pub async fn load_plugin(server: &Server, plugin_id: &str) -> PluginState {
    let experimental_allowed = experimental_apis_allowed(server);
    let decision = {
        let host = read_plugin_host(server);
        match find_plugin(&host, plugin_id) {
            Some(plugin) if LOADABLE_STATES.contains(&plugin.state) => {
                load_decision(plugin, &host, experimental_allowed)
            }
            Some(plugin) => return plugin.state,
            None => return PluginState::Unspecified,
        }
    };
    let (target_state, mut diagnostics) = decision;
    if target_state == PluginState::Enabled {
        diagnostics.extend(register_contributions(server, plugin_id));
    } else {
        unload_plugin(server, plugin_id).await;
    }
    let row = apply_state(server, plugin_id, target_state, diagnostics);
    // Any other outcome is the host's, decided again at every start: saving it would keep the plugin off after its
    // fix is in place, since a start loads only installed and enabled rows.
    if target_state == PluginState::Enabled {
        save_plugin_row(server, row).await;
    }
    record_plugin_state_changes(server).await;
    current_state(server, plugin_id)
}

/// Removes everything a plugin registered: its commands leave every surface at once. Stopping its module joins
/// with the modules (M4).
pub async fn unload_plugin(server: &Server, plugin_id: &str) {
    let mut registry = write_registry(server);
    remove_plugin_commands(&mut registry, plugin_id);
}

/// Turns a plugin off: its contributions go, its state becomes `disabled` (the user's decision, so its row keeps
/// it), the tokens it made are revoked with what they made (contracts 8.2), and every other enabled plugin is checked
/// again, since it may have needed this one's capabilities (`RRS-P1002`) or this one as a dependency (`RRS-P3001`).
/// `plugin.disable` (M8) calls it; until then tests do.
pub async fn disable_plugin(server: &Server, plugin_id: &str) -> PluginState {
    {
        // From the state change until the sweep is in the gate's cache, so a token.create for this plugin either
        // committed before it (and is swept) or finds the plugin disabled (token.rs).
        let _plugin_host_guard = server.plugin_host_lock.lock().await;
        unload_plugin(server, plugin_id).await;
        let row = apply_state(server, plugin_id, PluginState::Disabled, Vec::new());
        save_disabled_plugin(server, plugin_id, row).await;
    }
    // Before the re-check, so the log has this plugin's move ahead of the moves it causes.
    record_plugin_state_changes(server).await;
    recheck_enabled_plugins(server).await;
    current_state(server, plugin_id)
}

/// Checks every enabled plugin again, in load order, until a pass turns nothing off. Load order checks a dependency
/// before its dependents, and the repeat catches what one pass can't: a plugin turned off late in a pass that an
/// earlier one needed. A check never turns a plugin on, so each pass either turns one off or is the last.
async fn recheck_enabled_plugins(server: &Server) {
    loop {
        let order = {
            let host = read_plugin_host(server);
            plugin_load_order(&host.plugins).0
        };
        let mut turned_off = false;
        for plugin_id in &order {
            if current_state(server, plugin_id) != PluginState::Enabled {
                continue;
            }
            if load_plugin(server, plugin_id).await != PluginState::Enabled {
                turned_off = true;
            }
        }
        if !turned_off {
            return;
        }
    }
}

/// The scopes a plugin's principal holds (flows/platform.md 3.2, 3.3): its granted `commands:[scopes]` and its own
/// `plugin:<plugin_id>`, while it is enabled; nothing otherwise, so a disabled plugin can run nothing.
pub(crate) fn plugin_principal_scopes(server: &Server, plugin_id: &str) -> Vec<Scope> {
    let host = read_plugin_host(server);
    match find_plugin(&host, plugin_id) {
        Some(plugin) if plugin.state == PluginState::Enabled => {
            plugin_scopes(&plugin.plugin_id, &plugin.granted_permissions)
        }
        _ => Vec::new(),
    }
}

/// The plugin's state and diagnostics, or None for a plugin the host doesn't know.
pub(crate) fn plugin_status(server: &Server, plugin_id: &str) -> Option<(PluginState, Vec<Diagnostic>)> {
    let host = read_plugin_host(server);
    find_plugin(&host, plugin_id).map(|plugin| (plugin.state, plugin.diagnostics.clone()))
}

/// Whether a plugin module may start where user namespaces are unavailable (contracts 8.8, flows/platform.md 7.3).
/// `Ok` carries the warnings to show (`RRS-P2005` for a trusted first-party module running without the network
/// namespace); `Err` is the refusal (`RRS-P2001` for a third-party module without network permission). A module with
/// network permission, or any module where namespaces work, starts as usual. Trusted means preinstalled, with a
/// first-party id and the content hash the release shipped.
// A refusal is the contract `Diagnostic`; refusals are rare, so it is returned as it is rather than boxed.
#[allow(clippy::result_large_err)]
pub fn check_module_isolation(
    plugin: &LoadedPlugin,
    namespaces_available: bool,
    release_content_hash: &str,
) -> Result<Vec<Diagnostic>, Diagnostic> {
    let network_granted = plugin
        .granted_permissions
        .iter()
        .any(|permission| permission.starts_with(NETWORK_PERMISSION_PREFIX));
    if namespaces_available || network_granted {
        return Ok(Vec::new());
    }
    let trusted_first_party = plugin.source == PluginSource::Preinstalled
        && is_first_party_plugin_id(&plugin.plugin_id)
        && !release_content_hash.is_empty()
        && plugin.content_hash == release_content_hash;
    let values = [("plugin_id", plugin.plugin_id.as_str())];
    if trusted_first_party {
        return Ok(vec![diagnostic(CODE_NO_NAMESPACES_FIRST_PARTY, &values)]);
    }
    Err(diagnostic(CODE_NO_NAMESPACES_THIRD_PARTY, &values))
}

/// The order plugins load in: first-party plugins first, then repeatedly every plugin whose dependencies among the
/// discovered plugins are already ordered. What is left over is in a dependency cycle or depends on one; it comes
/// back in the second list, each plugin with the first dependency that couldn't be ordered. A dependency nobody
/// discovered doesn't hold a plugin back: negotiation reports it (`RRS-P3001`).
pub(crate) fn plugin_load_order(plugins: &[LoadedPlugin]) -> (Vec<String>, Vec<(String, String)>) {
    let mut ordered: Vec<String> = Vec::new();
    let mut waiting: Vec<&LoadedPlugin> = Vec::new();
    for plugin in plugins {
        if is_first_party_plugin_id(&plugin.plugin_id) {
            ordered.push(plugin.plugin_id.clone());
        } else {
            waiting.push(plugin);
        }
    }
    loop {
        let (ready, still_waiting): (Vec<&LoadedPlugin>, Vec<&LoadedPlugin>) = waiting
            .into_iter()
            .partition(|plugin| dependencies_ordered(plugin, plugins, &ordered));
        if ready.is_empty() {
            let blocked = still_waiting
                .iter()
                .map(|plugin| {
                    (
                        plugin.plugin_id.clone(),
                        unordered_dependency(plugin, plugins, &ordered),
                    )
                })
                .collect();
            return (ordered, blocked);
        }
        ordered.extend(ready.iter().map(|plugin| plugin.plugin_id.clone()));
        waiting = still_waiting;
    }
}

fn dependencies_ordered(plugin: &LoadedPlugin, plugins: &[LoadedPlugin], ordered: &[String]) -> bool {
    unordered_dependencies(plugin, plugins, ordered).is_empty()
}

/// The plugin's dependencies that were discovered but aren't ordered yet, in the manifest's order.
fn unordered_dependencies(plugin: &LoadedPlugin, plugins: &[LoadedPlugin], ordered: &[String]) -> Vec<String> {
    let Some(manifest) = &plugin.manifest else {
        return Vec::new();
    };
    manifest
        .dependencies
        .keys()
        .filter(|dependency_id| {
            let discovered = plugins.iter().any(|other| &other.plugin_id == *dependency_id);
            discovered && !ordered.contains(dependency_id)
        })
        .cloned()
        .collect()
}

/// The first dependency that held the plugin back; a plugin is only left over when it has one.
fn unordered_dependency(plugin: &LoadedPlugin, plugins: &[LoadedPlugin], ordered: &[String]) -> String {
    let unordered = unordered_dependencies(plugin, plugins, ordered);
    unordered.into_iter().next().unwrap_or_default()
}

/// Disables a plugin that can't be ordered. The host's decision: its row isn't written, and the next start checks
/// the order again.
async fn refuse_dependency_cycle(server: &Server, plugin_id: &str, dependency_id: &str) {
    let refusal = diagnostic(
        CODE_DEPENDENCY_MISSING,
        &[("dependency", dependency_id), ("plugin_id", plugin_id)],
    );
    apply_state(server, plugin_id, PluginState::Disabled, vec![refusal]);
    record_plugin_state_changes(server).await;
}

/// The state a load leads to, with its reasons: `failed` without a readable manifest, `incompatible` with layer-3
/// problems, else what negotiation decides.
fn load_decision(
    plugin: &LoadedPlugin,
    host: &PluginHost,
    experimental_allowed: bool,
) -> (PluginState, Vec<Diagnostic>) {
    let Some(manifest) = &plugin.manifest else {
        return (PluginState::Failed, plugin.manifest_problems.clone());
    };
    if has_errors(&plugin.manifest_problems) {
        return (PluginState::Incompatible, plugin.manifest_problems.clone());
    }
    let others: Vec<&LoadedPlugin> = host
        .plugins
        .iter()
        .filter(|other| other.plugin_id != plugin.plugin_id)
        .collect();
    let (enabled_first_party, disabled_first_party) = first_party_ids(&others);
    let available = available_capabilities(&enabled_first_party, &disabled_first_party);
    let result = negotiate_capabilities(
        manifest,
        &available,
        &enabled_versions(&others),
        &plugin.granted_permissions,
    );
    let state = state_after_negotiation(&result, experimental_allowed);
    (state, negotiation_diagnostics(manifest, &result, experimental_allowed))
}

/// The first-party plugins the host knows: those enabled, and those in any other state.
fn first_party_ids(plugins: &[&LoadedPlugin]) -> (Vec<String>, Vec<String>) {
    let mut enabled = Vec::new();
    let mut disabled = Vec::new();
    for plugin in plugins {
        if !is_first_party_plugin_id(&plugin.plugin_id) {
            continue;
        }
        if plugin.state == PluginState::Enabled {
            enabled.push(plugin.plugin_id.clone());
        } else {
            disabled.push(plugin.plugin_id.clone());
        }
    }
    (enabled, disabled)
}

/// Each enabled plugin's id and version, for `dependencies`. A version that doesn't parse satisfies no range.
fn enabled_versions(plugins: &[&LoadedPlugin]) -> Vec<(String, Version)> {
    let mut versions = Vec::new();
    for plugin in plugins {
        if plugin.state != PluginState::Enabled {
            continue;
        }
        if let Ok(version) = parse_version(&plugin.version) {
            versions.push((plugin.plugin_id.clone(), version));
        }
    }
    versions
}

/// Registers the plugin's commands, each capped by its grant; what was refused comes back as the plugin's
/// diagnostics. The other contribution keys are what `plugin.list` returns, for the UI to register.
fn register_contributions(server: &Server, plugin_id: &str) -> Vec<Diagnostic> {
    let (manifest, scopes) = {
        let host = read_plugin_host(server);
        let Some(plugin) = find_plugin(&host, plugin_id) else {
            return Vec::new();
        };
        let Some(manifest) = plugin.manifest.clone() else {
            return Vec::new();
        };
        (manifest, plugin_scopes(&plugin.plugin_id, &plugin.granted_permissions))
    };
    let mut registry = write_registry(server);
    register_plugin_commands(&mut registry, &manifest, &scopes)
}

/// Moves the plugin to its new state and records the change; returns its row when it has one to save.
fn apply_state(
    server: &Server,
    plugin_id: &str,
    to_state: PluginState,
    diagnostics: Vec<Diagnostic>,
) -> Option<PluginRow> {
    let mut host = write_plugin_host(server);
    let PluginHost { plugins, state_changes } = &mut *host;
    let plugin = plugins.iter_mut().find(|plugin| plugin.plugin_id == plugin_id)?;
    if let Some(change) = change_plugin_state(plugin, to_state, diagnostics) {
        record_state_change(state_changes, change);
    }
    plugin_row(plugin)
}

fn current_state(server: &Server, plugin_id: &str) -> PluginState {
    let host = read_plugin_host(server);
    find_plugin(&host, plugin_id).map_or(PluginState::Unspecified, |plugin| plugin.state)
}

fn experimental_apis_allowed(server: &Server) -> bool {
    match server.settings.read() {
        Ok(settings) => setting_bool(&settings, EXPERIMENTAL_APIS_SETTING),
        Err(poisoned) => setting_bool(&poisoned.into_inner(), EXPERIMENTAL_APIS_SETTING),
    }
}

pub(crate) fn find_plugin<'a>(host: &'a PluginHost, plugin_id: &str) -> Option<&'a LoadedPlugin> {
    host.plugins.iter().find(|plugin| plugin.plugin_id == plugin_id)
}

// The tables are changed only by whole pushes, retains and field writes, so a panic elsewhere leaves them poisoned
// but whole: reads and writes carry on.
pub(crate) fn read_plugin_host(server: &Server) -> RwLockReadGuard<'_, PluginHost> {
    match server.plugins.read() {
        Ok(host) => host,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn write_plugin_host(server: &Server) -> RwLockWriteGuard<'_, PluginHost> {
    match server.plugins.write() {
        Ok(host) => host,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_registry(server: &Server) -> RwLockWriteGuard<'_, CommandRegistry> {
    match server.registry.write() {
        Ok(registry) => registry,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn plugin(plugin_id: &str, dependencies: &[&str]) -> LoadedPlugin {
        let mut manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
            "type": "rastai.plugin",
            "schema_version": 1,
            "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
            "plugin_id": plugin_id,
            "display_name": plugin_id,
            "version": "1.0.0",
            "engines": {"rastai": "*"}
        }))
        .unwrap();
        manifest.dependencies = dependencies
            .iter()
            .map(|dependency| (dependency.to_string(), "*".to_string()))
            .collect::<BTreeMap<String, String>>();
        LoadedPlugin {
            plugin_id: plugin_id.to_string(),
            version: "1.0.0".to_string(),
            source: PluginSource::Dev,
            folder: PathBuf::new(),
            manifest: Some(manifest),
            state: PluginState::Installed,
            granted_permissions: Vec::new(),
            content_hash: String::new(),
            diagnostics: Vec::new(),
            manifest_problems: Vec::new(),
            event_handlers: Vec::new(),
        }
    }

    #[test]
    fn first_party_loads_first_then_dependencies_before_dependents() {
        let plugins = [
            plugin("acme_lens", &["acme_base"]),
            plugin("acme_base", &[]),
            plugin("psychology", &[]),
            plugin("acme_extra", &["missing_one"]),
        ];
        let (order, blocked) = plugin_load_order(&plugins);
        assert_eq!(order, ["psychology", "acme_base", "acme_extra", "acme_lens"]);
        assert!(blocked.is_empty());
    }

    #[test]
    fn a_dependency_cycle_is_left_out_of_the_order() {
        let plugins = [
            plugin("acme_a", &["acme_b"]),
            plugin("acme_b", &["acme_a"]),
            plugin("acme_c", &[]),
            plugin("acme_d", &["acme_a"]),
        ];
        let (order, blocked) = plugin_load_order(&plugins);
        assert_eq!(order, ["acme_c"]);
        // Each names the dependency that held it back; acme_d is only behind the cycle.
        let expected = [("acme_a", "acme_b"), ("acme_b", "acme_a"), ("acme_d", "acme_a")];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(plugin_id, dependency_id)| (plugin_id.to_string(), dependency_id.to_string()))
            .collect();
        assert_eq!(blocked, expected);
    }

    #[test]
    fn module_isolation_follows_contracts_8_8() {
        let mut first_party = plugin("psychology", &[]);
        first_party.source = PluginSource::Preinstalled;
        first_party.content_hash = "blake3:aa".to_string();
        // Namespaces work: every module starts as usual.
        assert_eq!(check_module_isolation(&first_party, true, "blake3:aa"), Ok(Vec::new()));
        // Without them, the trusted first-party module starts with a warning...
        let warnings = check_module_isolation(&first_party, false, "blake3:aa").unwrap();
        assert_eq!(warnings[0].code, CODE_NO_NAMESPACES_FIRST_PARTY);
        // ...but not when its content isn't the release's, or it came from a dev folder.
        let changed = check_module_isolation(&first_party, false, "blake3:bb").unwrap_err();
        assert_eq!(changed.code, CODE_NO_NAMESPACES_THIRD_PARTY);
        first_party.source = PluginSource::Dev;
        let from_dev = check_module_isolation(&first_party, false, "blake3:aa").unwrap_err();
        assert_eq!(from_dev.code, CODE_NO_NAMESPACES_THIRD_PARTY);
        // A third-party module is refused, unless it was granted the network.
        let mut third_party = plugin("acme_tools", &[]);
        let refused = check_module_isolation(&third_party, false, "").unwrap_err();
        assert_eq!(refused.code, CODE_NO_NAMESPACES_THIRD_PARTY);
        third_party.granted_permissions = vec!["network:[api.example.com]".to_string()];
        assert_eq!(check_module_isolation(&third_party, false, ""), Ok(Vec::new()));
    }
}
