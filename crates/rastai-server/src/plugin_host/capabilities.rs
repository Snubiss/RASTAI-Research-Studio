//! The capability table of contracts 5.5 as data, and capability negotiation as a pure function over it (layout:
//! rust-control-plane.md 12.2 and 12.3; flows/platform.md 7.3).
//!
//! A manifest lists the capabilities it needs as `"<name>@<major>"` in `requires_capabilities`. Each contribution key
//! maps to one capability, and a plugin must require the capability of every key it uses. `negotiate_capabilities`
//! compares a manifest with what this build provides and returns plain data; `state_after_negotiation` and
//! `negotiation_diagnostics` turn that data into the plugin's next state and the reasons for it. Nothing here
//! touches disk or the server, so `tests/plugin_host.rs` runs the whole table without a daemon.

use rastai_core::versions::PLUGIN_API_VERSION;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::documents::plugin_manifest::{PluginContributions, PluginManifest};
use rastai_formats::semver::{Version, parse_version, version_matches};
use rastai_proto::common::v1::PluginState;
use serde_json::Value;

const CODE_ENGINES_EXCLUDE_API: &str = "RRS-P1001";
const CODE_CAPABILITY_NOT_PROVIDED: &str = "RRS-P1002";
const CODE_EXPERIMENTAL_WITHOUT_FLAG: &str = "RRS-P1003";
const CODE_UNDECLARED_CONTRIBUTION: &str = "RRS-P0006";
const CODE_DEPENDENCY_MISSING: &str = "RRS-P3001";

/// `"<name>@<major>"`: the capability's name, then its major version.
const MAJOR_SEPARATOR: char = '@';

/// What the host does with a capability in this build. A row moves from Unsupported to Supported in the PR that
/// lands its feature (themes, icon sets, macros and chat templates in M8), and only then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    Supported,
    /// Loads only with `plugins.experimental_apis` on (`RRS-P1003` otherwise).
    Experimental,
    /// Not provided: a plugin that requires it is refused with `RRS-P1002`.
    Unsupported,
}

/// A capability the host itself provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapability {
    pub name: &'static str,
    pub majors: &'static [u32],
    /// The manifest's `contributes` key this capability covers.
    pub contribution_key: &'static str,
    pub status: CapabilityStatus,
}

/// A capability a first-party plugin provides, only while that plugin is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstPartyCapability {
    pub plugin_id: &'static str,
    pub name: &'static str,
    pub majors: &'static [u32],
    pub contribution_key: &'static str,
}

/// The host's rows of contracts 5.5, in its order. Every contribution key has a row here or in
/// `FIRST_PARTY_CAPABILITIES`, so no key is silently accepted.
pub const HOST_CAPABILITIES: &[HostCapability] = &[
    host("commands", "commands", CapabilityStatus::Supported),
    host("ui.section", "sections", CapabilityStatus::Supported),
    host("ui.panel", "panels", CapabilityStatus::Supported),
    host("ui.settings_page", "settings_pages", CapabilityStatus::Supported),
    // M8: theme plugins end to end, with the accessibility check.
    host("ui.theme", "themes", CapabilityStatus::Unsupported),
    // M8: rastai.icon_set/1.
    host("ui.icon_set", "icon_sets", CapabilityStatus::Unsupported),
    // Phase 2: prototype strings are English.
    host("ui.locale", "locales", CapabilityStatus::Unsupported),
    // M8 (M8-E05).
    host("ui.macro", "macros", CapabilityStatus::Unsupported),
    host("ui.layout", "layouts", CapabilityStatus::Supported),
    host("ui.keybindings", "keybindings", CapabilityStatus::Supported),
    host("templates", "templates", CapabilityStatus::Supported),
    host("file.type", "file_types", CapabilityStatus::Supported),
    host("file.importer", "importers", CapabilityStatus::Supported),
    host("file.exporter", "exporters", CapabilityStatus::Supported),
    host("data.converter", "converters", CapabilityStatus::Supported),
    // Spec §33: streaming data is deferred.
    host("data.source", "data_sources", CapabilityStatus::Experimental),
    host("train.recipe", "recipes", CapabilityStatus::Supported),
    host("rl.reward_function", "reward_functions", CapabilityStatus::Supported),
    host("designer.node", "node_types", CapabilityStatus::Supported),
    // Evaluation auto-run and pins need it.
    host("events.handler", "event_handlers", CapabilityStatus::Supported),
    // M8, files only.
    host(
        "inference.chat_template",
        "chat_templates",
        CapabilityStatus::Unsupported,
    ),
    // Files only; code tokenizers wait for the WASM tier (Phase 2).
    host("inference.tokenizer", "tokenizers", CapabilityStatus::Experimental),
    host("inference.sampler", "samplers", CapabilityStatus::Supported),
    host("sandbox.image", "sandbox_images", CapabilityStatus::Supported),
    // Gateway adapters are a closed Rust enum today.
    host("inference.adapter", "adapters", CapabilityStatus::Experimental),
    // Phase 2, with the tutorials (plan §10 row 45).
    host("assistant.skill", "assistant_skills", CapabilityStatus::Unsupported),
    // Until providers arrive (Phase 3).
    host("compute.target", "compute_targets", CapabilityStatus::Experimental),
];

/// The first-party rows of contracts 5.5.
pub const FIRST_PARTY_CAPABILITIES: &[FirstPartyCapability] = &[
    first_party("psychology", "psychology.lens", "lenses"),
    first_party("psychology", "psychology.sae_loader", "sae_loaders"),
    first_party("eval", "eval.metric", "metrics"),
    first_party("eval", "eval.benchmark_loader", "benchmark_loaders"),
];

/// One capability a plugin may require right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableCapability {
    pub name: String,
    pub major: u32,
    pub experimental: bool,
}

/// What negotiation found. Every list names what it is about, so each entry becomes one diagnostic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NegotiationResult {
    /// `engines.rastai` covers `PLUGIN_API_VERSION`; `RRS-P1001` otherwise.
    pub engines_ok: bool,
    /// Required capabilities this build doesn't provide, as written in the manifest (`RRS-P1002`).
    pub missing_capabilities: Vec<String>,
    /// Required capabilities that are experimental (`RRS-P1003` unless `plugins.experimental_apis` is on).
    pub experimental_capabilities: Vec<String>,
    /// Contribution keys used without requiring their capability (`RRS-P0006`).
    pub undeclared_contribution_keys: Vec<String>,
    /// Plugin dependencies that aren't enabled at a version in range (`RRS-P3001`).
    pub missing_dependencies: Vec<String>,
    /// Permissions the manifest asks for that were never granted: the plugin goes back to `awaiting_trust`.
    pub new_permissions: Vec<String>,
}

const fn host(name: &'static str, contribution_key: &'static str, status: CapabilityStatus) -> HostCapability {
    HostCapability {
        name,
        majors: &[1],
        contribution_key,
        status,
    }
}

const fn first_party(
    plugin_id: &'static str,
    name: &'static str,
    contribution_key: &'static str,
) -> FirstPartyCapability {
    FirstPartyCapability {
        plugin_id,
        name,
        majors: &[1],
        contribution_key,
    }
}

/// What a plugin may require now: the host's supported and experimental rows, plus the rows of each first-party
/// plugin that is enabled. Disabling Psychology so takes `psychology.lens@1` away from the plugins that need it.
pub fn available_capabilities(enabled_first_party: &[String]) -> Vec<AvailableCapability> {
    let mut available = Vec::new();
    for row in HOST_CAPABILITIES {
        let experimental = match row.status {
            CapabilityStatus::Supported => false,
            CapabilityStatus::Experimental => true,
            CapabilityStatus::Unsupported => continue,
        };
        available.extend(each_major(row.name, row.majors, experimental));
    }
    for row in FIRST_PARTY_CAPABILITIES {
        if enabled_first_party.iter().any(|plugin_id| plugin_id == row.plugin_id) {
            available.extend(each_major(row.name, row.majors, false));
        }
    }
    available
}

fn each_major(name: &str, majors: &[u32], experimental: bool) -> Vec<AvailableCapability> {
    majors
        .iter()
        .map(|major| AvailableCapability {
            name: name.to_string(),
            major: *major,
            experimental,
        })
        .collect()
}

/// The ids of the first-party plugins, the ones that load first because others require their capabilities.
pub(crate) fn is_first_party_plugin_id(plugin_id: &str) -> bool {
    FIRST_PARTY_CAPABILITIES.iter().any(|row| row.plugin_id == plugin_id)
}

/// Compares a manifest with what is available. Pure: the same inputs always give the same result.
/// `enabled_plugins` holds each enabled plugin's id and version, for the manifest's `dependencies`.
pub fn negotiate_capabilities(
    manifest: &PluginManifest,
    available: &[AvailableCapability],
    enabled_plugins: &[(String, Version)],
    granted_permissions: &[String],
) -> NegotiationResult {
    let mut result = NegotiationResult {
        engines_ok: engines_cover_api(&manifest.engines.rastai),
        ..NegotiationResult::default()
    };
    for required in &manifest.requires_capabilities {
        match find_available(available, required) {
            None => result.missing_capabilities.push(required.clone()),
            Some(found) if found.experimental => result.experimental_capabilities.push(required.clone()),
            Some(_) => {}
        }
    }
    result.undeclared_contribution_keys = undeclared_contribution_keys(manifest);
    result.missing_dependencies = missing_dependencies(manifest, enabled_plugins);
    result.new_permissions = manifest
        .permissions
        .iter()
        .filter(|permission| !granted_permissions.contains(permission))
        .cloned()
        .collect();
    result
}

/// The state negotiation leads to, in the order of flows/platform.md 7.3. `Enabled` means the plugin may load: its
/// contributions are registered next.
pub fn state_after_negotiation(result: &NegotiationResult, experimental_allowed: bool) -> PluginState {
    let incompatible = !result.engines_ok
        || !result.missing_capabilities.is_empty()
        || !result.undeclared_contribution_keys.is_empty();
    if incompatible {
        return PluginState::Incompatible;
    }
    if !result.experimental_capabilities.is_empty() && !experimental_allowed {
        return PluginState::Disabled;
    }
    if !result.new_permissions.is_empty() {
        return PluginState::AwaitingTrust;
    }
    if !result.missing_dependencies.is_empty() {
        return PluginState::Disabled;
    }
    PluginState::Enabled
}

/// One diagnostic per problem negotiation found. Experimental capabilities are a problem only while the flag is off.
pub fn negotiation_diagnostics(
    manifest: &PluginManifest,
    result: &NegotiationResult,
    experimental_allowed: bool,
) -> Vec<Diagnostic> {
    let plugin_id = manifest.plugin_id.as_str();
    let mut diagnostics = Vec::new();
    if !result.engines_ok {
        let values = [
            ("plugin_id", plugin_id),
            ("range", manifest.engines.rastai.as_str()),
            ("api_version", PLUGIN_API_VERSION),
        ];
        diagnostics.push(diagnostic(CODE_ENGINES_EXCLUDE_API, &values));
    }
    for capability in &result.missing_capabilities {
        let values = [("plugin_id", plugin_id), ("capability", capability.as_str())];
        diagnostics.push(diagnostic(CODE_CAPABILITY_NOT_PROVIDED, &values));
    }
    for key in &result.undeclared_contribution_keys {
        diagnostics.push(diagnostic(CODE_UNDECLARED_CONTRIBUTION, &[("extension_point", key)]));
    }
    if !experimental_allowed {
        for capability in &result.experimental_capabilities {
            let values = [("plugin_id", plugin_id), ("api_name", capability.as_str())];
            diagnostics.push(diagnostic(CODE_EXPERIMENTAL_WITHOUT_FLAG, &values));
        }
    }
    for dependency in &result.missing_dependencies {
        let values = [("dependency", dependency.as_str()), ("plugin_id", plugin_id)];
        diagnostics.push(diagnostic(CODE_DEPENDENCY_MISSING, &values));
    }
    diagnostics
}

/// The capability every contribution key needs, from both tables.
pub(crate) fn capability_for_contribution_key(key: &str) -> Option<&'static str> {
    let host_row = HOST_CAPABILITIES.iter().find(|row| row.contribution_key == key);
    if let Some(row) = host_row {
        return Some(row.name);
    }
    let first_party_row = FIRST_PARTY_CAPABILITIES.iter().find(|row| row.contribution_key == key);
    first_party_row.map(|row| row.name)
}

/// A range that doesn't parse can't cover anything, so it is `RRS-P1001` too: the message shows the range as written.
fn engines_cover_api(range: &str) -> bool {
    let Ok(api_version) = parse_version(PLUGIN_API_VERSION) else {
        return false;
    };
    matches!(version_matches(range, &api_version), Ok(true))
}

/// `"ui.panel@1"` → the available row named `ui.panel` with major 1. A requirement without `@<major>`, or with a
/// major that isn't a number, matches nothing and is reported as not provided.
fn find_available<'a>(available: &'a [AvailableCapability], required: &str) -> Option<&'a AvailableCapability> {
    let (name, major_text) = required.rsplit_once(MAJOR_SEPARATOR)?;
    let major: u32 = major_text.parse().ok()?;
    available.iter().find(|row| row.name == name && row.major == major)
}

/// Each contribution key the manifest uses (a non-empty list) whose capability it doesn't require, at any major.
/// The keys are read from the serialized contributions, so a key added to the manifest type is checked at once; a
/// key with no row in either table is reported too.
fn undeclared_contribution_keys(manifest: &PluginManifest) -> Vec<String> {
    let mut undeclared = Vec::new();
    for key in used_contribution_keys(&manifest.contributes) {
        let declared = match capability_for_contribution_key(&key) {
            Some(capability) => requires_capability_name(manifest, capability),
            None => false,
        };
        if !declared {
            undeclared.push(key);
        }
    }
    undeclared
}

fn used_contribution_keys(contributes: &PluginContributions) -> Vec<String> {
    let Ok(Value::Object(lists)) = serde_json::to_value(contributes) else {
        return Vec::new();
    };
    lists
        .into_iter()
        .filter(|(_, entries)| entries.as_array().is_some_and(|items| !items.is_empty()))
        .map(|(key, _)| key)
        .collect()
}

fn requires_capability_name(manifest: &PluginManifest, capability: &str) -> bool {
    manifest.requires_capabilities.iter().any(|required| {
        let name = required
            .rsplit_once(MAJOR_SEPARATOR)
            .map_or(required.as_str(), |(name, _)| name);
        name == capability
    })
}

/// Each dependency that no enabled plugin satisfies: absent, or at a version outside the range (a range that
/// doesn't parse satisfies nothing).
fn missing_dependencies(manifest: &PluginManifest, enabled_plugins: &[(String, Version)]) -> Vec<String> {
    let mut missing = Vec::new();
    for (dependency_id, range) in &manifest.dependencies {
        let satisfied = enabled_plugins.iter().any(|(plugin_id, version)| {
            plugin_id == dependency_id && matches!(version_matches(range, version), Ok(true))
        });
        if !satisfied {
            missing.push(dependency_id.clone());
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_31_capabilities_with_one_key_each() {
        assert_eq!(HOST_CAPABILITIES.len() + FIRST_PARTY_CAPABILITIES.len(), 31);
        let mut keys: Vec<&str> = HOST_CAPABILITIES.iter().map(|row| row.contribution_key).collect();
        keys.extend(FIRST_PARTY_CAPABILITIES.iter().map(|row| row.contribution_key));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a contribution key has two rows");
    }

    #[test]
    fn first_party_capabilities_exist_only_while_their_plugin_is_enabled() {
        let without = available_capabilities(&[]);
        assert!(!without.iter().any(|row| row.name == "psychology.lens"));
        let with = available_capabilities(&["psychology".to_string()]);
        assert!(with.iter().any(|row| row.name == "psychology.lens" && row.major == 1));
        assert!(!with.iter().any(|row| row.name == "eval.metric"));
    }

    #[test]
    fn unsupported_rows_are_never_available() {
        let available = available_capabilities(&["psychology".to_string(), "eval".to_string()]);
        for name in [
            "ui.theme",
            "ui.icon_set",
            "ui.locale",
            "ui.macro",
            "inference.chat_template",
        ] {
            assert!(!available.iter().any(|row| row.name == name), "{name}");
        }
        let tokenizer = available.iter().find(|row| row.name == "inference.tokenizer");
        assert!(tokenizer.is_some_and(|row| row.experimental));
    }

    #[test]
    fn requirements_without_a_major_match_nothing() {
        let available = available_capabilities(&[]);
        assert!(find_available(&available, "commands@1").is_some());
        assert!(find_available(&available, "commands").is_none());
        assert!(find_available(&available, "commands@x").is_none());
        assert!(find_available(&available, "commands@2").is_none());
    }
}
