//! What registration does on the daemon side (layout: rust-control-plane.md 12.4): a plugin's manifest commands
//! become registry descriptors, so they are palette entries, CLI verbs and MCP tools at once, and the plugin's grant
//! becomes the scopes of its principal.
//!
//! Two rules keep a plugin inside its grant:
//! - **Namespaced.** Every command id starts with the plugin id and sits outside the core areas (`RRS-P0004`,
//!   rastai-core's `add_plugin_commands`); the manifest's layer 3 checks the same before a load gets here.
//! - **Capped by the grant.** A command's scope, the one a caller needs, must be one the plugin itself holds: its
//!   granted `commands:[scopes]` or its own `plugin:<plugin_id>` (flows/platform.md 7.3). A command asking for more
//!   is refused with `RRS-C1005` and the rest load. A command that names no scope gets the plugin's own.
//!
//! The other contribution keys need nothing here yet: the UI reads them from `plugin.list`. File types, importers and
//! exporters join the routing table with `file.open` (M6); argument and result schemas come from the plugin's
//! `types.json` (M4).

use rastai_core::commands::registry::{CommandRegistry, add_plugin_commands};
use rastai_core::principal::{Principal, PrincipalKind, principal_label};
use rastai_core::scopes::{Scope, parse_scope, scope_name, scopes_allow};
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::documents::plugin_manifest::{CommandContribution, PluginCommandKind, PluginManifest};
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind};

const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
/// `commands:[read, chat]`: the scopes a plugin's commands may run with (contracts 6.3).
const COMMANDS_PERMISSION_START: &str = "commands:[";
const COMMANDS_PERMISSION_END: &str = "]";
const SCOPE_LIST_SEPARATOR: char = ',';

/// The scopes in a plugin's granted `commands:[...]` permissions. A scope that doesn't parse, and another plugin's
/// own scope, grant nothing: a plugin holds only its own `plugin:<plugin_id>`.
pub(crate) fn granted_command_scopes(plugin_id: &str, granted_permissions: &[String]) -> Vec<Scope> {
    let mut scopes = Vec::new();
    for permission in granted_permissions {
        let Some(inside) = permission
            .strip_prefix(COMMANDS_PERMISSION_START)
            .and_then(|rest| rest.strip_suffix(COMMANDS_PERMISSION_END))
        else {
            continue;
        };
        for name in inside.split(SCOPE_LIST_SEPARATOR) {
            let Ok(scope) = parse_scope(name.trim()) else {
                continue;
            };
            let foreign = matches!(&scope, Scope::Plugin(owner) if owner != plugin_id);
            if !foreign && !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

/// The scopes the plugin's principal holds: its grant, and its own scope.
pub(crate) fn plugin_scopes(plugin_id: &str, granted_permissions: &[String]) -> Vec<Scope> {
    let mut scopes = granted_command_scopes(plugin_id, granted_permissions);
    let own = Scope::Plugin(plugin_id.to_string());
    if !scopes.contains(&own) {
        scopes.push(own);
    }
    scopes
}

/// Registers the manifest's commands within the plugin's scopes, replacing any it registered before. What was
/// refused comes back, one diagnostic per command, and everything else is registered.
pub(crate) fn register_plugin_commands(
    registry: &mut CommandRegistry,
    manifest: &PluginManifest,
    plugin_scopes: &[Scope],
) -> Vec<Diagnostic> {
    let plugin_id = manifest.plugin_id.as_str();
    let mut allowed = Vec::new();
    let mut refusals = Vec::new();
    for contribution in &manifest.contributes.commands {
        let descriptor = plugin_command_descriptor(manifest, contribution);
        match check_scope_within_grant(plugin_id, &descriptor, plugin_scopes) {
            Ok(()) => allowed.push(descriptor),
            Err(refusal) => refusals.push(refusal),
        }
    }
    if let Err(skipped) = add_plugin_commands(registry, plugin_id, allowed) {
        refusals.extend(skipped);
    }
    refusals
}

/// The registry descriptor of one manifest command. Its schemas stay empty until `types.json` is read (M4), so its
/// arguments are checked as a JSON object. `add_plugin_commands` sets `plugin_id` and `preview`.
pub(crate) fn plugin_command_descriptor(
    manifest: &PluginManifest,
    contribution: &CommandContribution,
) -> CommandDescriptor {
    let scope = if contribution.scope.is_empty() {
        scope_name(&Scope::Plugin(manifest.plugin_id.clone()))
    } else {
        contribution.scope.clone()
    };
    CommandDescriptor {
        command_id: contribution.command_id.clone(),
        title: contribution.title.clone(),
        summary: contribution.summary.clone(),
        section: manifest.plugin_id.clone(),
        scope,
        undoable: contribution.undoable,
        kind: command_kind(contribution.kind) as i32,
        arguments_type: contribution.arguments_type.clone(),
        result_type: contribution.result_type.clone(),
        default_shortcut: contribution.default_shortcut.clone(),
        basic: contribution.basic,
        since_version: manifest.version.clone(),
        ..CommandDescriptor::default()
    }
}

fn command_kind(kind: PluginCommandKind) -> CommandKind {
    match kind {
        PluginCommandKind::Sync => CommandKind::Sync,
        PluginCommandKind::Job => CommandKind::Job,
        PluginCommandKind::Stream => CommandKind::Stream,
    }
}

/// `RRS-C1005` for a command whose scope the plugin doesn't hold, named as the plugin principal would be. A scope
/// that doesn't parse is left to `add_plugin_commands`, which refuses it with its own code.
// A refusal is the contract `Diagnostic`; refusals are rare, so it is returned as it is rather than boxed.
#[allow(clippy::result_large_err)]
fn check_scope_within_grant(
    plugin_id: &str,
    descriptor: &CommandDescriptor,
    plugin_scopes: &[Scope],
) -> Result<(), Diagnostic> {
    let Ok(needed) = parse_scope(&descriptor.scope) else {
        return Ok(());
    };
    if scopes_allow(plugin_scopes, &needed) {
        return Ok(());
    }
    let principal = Principal {
        kind: PrincipalKind::Plugin,
        id: plugin_id.to_string(),
        scopes: plugin_scopes.to_vec(),
    };
    let label = principal_label(&principal);
    Err(diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[
            ("scope", &descriptor.scope),
            ("principal", &label),
            ("command_id", &descriptor.command_id),
        ],
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn owned(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn the_grant_is_read_from_commands_permissions_only() {
        let granted = owned(&[
            "commands:[read, chat]",
            "network:[api.example.com]",
            "commands:[plugin:acme, plugin:other, everything]",
            "commands:read",
        ]);
        let scopes = granted_command_scopes("acme", &granted);
        assert_eq!(scopes, [Scope::Read, Scope::Chat, Scope::Plugin("acme".to_string())]);
    }

    #[test]
    fn a_plugin_always_holds_its_own_scope_once() {
        let scopes = plugin_scopes("acme", &owned(&["commands:[plugin:acme]"]));
        assert_eq!(scopes, [Scope::Plugin("acme".to_string())]);
        assert_eq!(plugin_scopes("acme", &[]), [Scope::Plugin("acme".to_string())]);
    }

    #[test]
    fn a_command_without_a_scope_gets_the_plugins_own() {
        let manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "$schema": "https://schemas.rastai.dev/rastai.plugin/1.json",
            "type": "rastai.plugin",
            "schema_version": 1,
            "id": "doc_01JA2B3C4D5E6F7G8H9J0K1M2N",
            "plugin_id": "acme",
            "display_name": "Acme",
            "version": "1.2.0",
            "engines": {"rastai": "*"},
            "contributes": {"commands": [
                {"command_id": "acme.run", "title": "Run", "kind": "job", "arguments_type": "A", "result_type": "R"}
            ]}
        }))
        .unwrap();
        let descriptor = plugin_command_descriptor(&manifest, &manifest.contributes.commands[0]);
        assert_eq!(descriptor.scope, "plugin:acme");
        assert_eq!(descriptor.kind, CommandKind::Job as i32);
        assert_eq!(descriptor.section, "acme");
        assert_eq!(descriptor.since_version, "1.2.0");
    }
}
