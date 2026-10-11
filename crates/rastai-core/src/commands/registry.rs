//! The command registry (layout rust-control-plane.md 5.2, 10.2): the core table turned into the generated
//! `CommandDescriptor`s, plus the commands of each loaded plugin. Every surface (`ListCommands`, MCP, the iframe
//! bridge, the CLI) reads this one list; none keeps its own. It is rebuilt at every start and never persisted.

use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_proto::common::v1::Stability;

use crate::commands::naming::check_command_id;
use crate::commands::table::COMMANDS;
use crate::principal::{Principal, PrincipalKind, principal_label};
use crate::scopes::{parse_scope, scopes_allow};
use crate::wire::descriptor_from_row;

const CODE_SCOPE_NOT_GRANTED: &str = "RRS-C1005";
const CODE_DUPLICATE_ID: &str = "RRS-E2005";

/// What the registry holds: the generated proto descriptor, with `arguments_schema_json` and `result_schema_json`
/// (contracts 4.2). Core commands come first, in table order, then each plugin's in the order they loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandRegistry {
    pub commands: Vec<CommandDescriptor>,
}

/// Which commands `list_commands` returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandFilter {
    /// Only this section's commands; empty for every section.
    pub section: String,
    /// Also experimental commands (the `plugins.experimental_apis` setting).
    pub include_experimental: bool,
    /// Only the commands shown in Basic mode.
    pub basic_only: bool,
}

/// The registry with every core command. The schema strings stay empty: rastai-server fills them from the
/// argument and result descriptions at startup (layout 10.2).
pub fn load_core_commands() -> CommandRegistry {
    CommandRegistry {
        commands: COMMANDS.iter().map(descriptor_from_row).collect(),
    }
}

/// Adds a plugin's commands, built by the plugin host from its manifest. Loading the same plugin again replaces
/// its earlier commands (hot reload in `plugin.dev`). Each command gets the plugin's id and is `preview` until
/// plugin API 1.0, whatever the manifest says.
///
/// A refused command is skipped and the others are still added, because one bad command must not keep the rest
/// of the plugin from loading (layout 10.2). The refusals come back in `Err`, one diagnostic per skipped command:
/// an id outside the plugin's namespace or in a core area (`RRS-P0004`), a malformed id or an unknown scope
/// (`RRS-E1004`), or an id the plugin lists twice (`RRS-E2005`).
pub fn add_plugin_commands(
    registry: &mut CommandRegistry,
    plugin_id: &str,
    commands: Vec<CommandDescriptor>,
) -> Result<(), Vec<Diagnostic>> {
    remove_plugin_commands(registry, plugin_id);
    let mut refusals = Vec::new();
    for mut command in commands {
        match check_plugin_command(registry, plugin_id, &command) {
            Ok(()) => {
                command.plugin_id = plugin_id.to_string();
                command.stability = Stability::Preview as i32;
                registry.commands.push(command);
            }
            Err(refusal) => refusals.push(refusal),
        }
    }
    if refusals.is_empty() { Ok(()) } else { Err(refusals) }
}

/// Removes every command of a plugin, when it is disabled, uninstalled or reloaded.
pub fn remove_plugin_commands(registry: &mut CommandRegistry, plugin_id: &str) {
    registry.commands.retain(|command| command.plugin_id != plugin_id);
}

/// The registered command with this id.
pub fn find_command<'a>(registry: &'a CommandRegistry, command_id: &str) -> Option<&'a CommandDescriptor> {
    registry
        .commands
        .iter()
        .find(|command| command.command_id == command_id)
}

/// The commands the filter lets through, in registry order.
pub fn list_commands<'a>(registry: &'a CommandRegistry, filter: &CommandFilter) -> Vec<&'a CommandDescriptor> {
    registry
        .commands
        .iter()
        .filter(|command| filter_allows(filter, command))
        .collect()
}

/// Refuses a caller that doesn't hold the command's scope (`RRS-C1005`). The owner holds every scope
/// (contracts 6.3), including a third-party plugin's own `plugin:<plugin_id>`, which `owner_principal()` can't
/// list ahead of time, so the owner always passes. A scope that doesn't parse is refused too: the check fails
/// closed.
#[allow(clippy::result_large_err)]
pub fn check_scope(principal: &Principal, command: &CommandDescriptor) -> Result<(), Diagnostic> {
    if principal.kind == PrincipalKind::Owner {
        return Ok(());
    }
    let granted = match parse_scope(&command.scope) {
        Ok(needed) => scopes_allow(&principal.scopes, &needed),
        Err(_) => false,
    };
    if granted {
        return Ok(());
    }
    let label = principal_label(principal);
    Err(diagnostic(
        CODE_SCOPE_NOT_GRANTED,
        &[
            ("scope", &command.scope),
            ("principal", &label),
            ("command_id", &command.command_id),
        ],
    ))
}

#[allow(clippy::result_large_err)]
fn check_plugin_command(
    registry: &CommandRegistry,
    plugin_id: &str,
    command: &CommandDescriptor,
) -> Result<(), Diagnostic> {
    check_command_id(&command.command_id, plugin_id)?;
    parse_scope(&command.scope)?;
    if find_command(registry, &command.command_id).is_some() {
        return Err(diagnostic(CODE_DUPLICATE_ID, &[("id", &command.command_id)]));
    }
    Ok(())
}

fn filter_allows(filter: &CommandFilter, command: &CommandDescriptor) -> bool {
    let section_matches = filter.section.is_empty() || command.section == filter.section;
    let experimental_allowed = filter.include_experimental || !command.experimental;
    let basic_allowed = !filter.basic_only || command.basic;
    section_matches && experimental_allowed && basic_allowed
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::principal::owner_principal;
    use crate::scopes::Scope;

    fn plugin_command(command_id: &str, scope: &str) -> CommandDescriptor {
        CommandDescriptor {
            command_id: command_id.to_string(),
            scope: scope.to_string(),
            section: "psychology".to_string(),
            stability: Stability::Stable as i32,
            ..CommandDescriptor::default()
        }
    }

    fn token_principal(scopes: Vec<Scope>) -> Principal {
        Principal {
            kind: PrincipalKind::Token,
            id: "tok_01J".to_string(),
            scopes,
        }
    }

    #[test]
    fn core_commands_are_the_table() {
        let registry = load_core_commands();
        assert_eq!(registry.commands.len(), COMMANDS.len());
        for (descriptor, row) in registry.commands.iter().zip(COMMANDS) {
            assert_eq!(descriptor.command_id, row.command_id);
            assert!(descriptor.plugin_id.is_empty());
        }
    }

    #[test]
    fn plugin_commands_are_added_found_and_removed() {
        let mut registry = load_core_commands();
        let commands = vec![
            plugin_command("psychology.replay.open", "psychology"),
            plugin_command("psychology.step", "psychology"),
        ];
        add_plugin_commands(&mut registry, "psychology", commands).unwrap();
        let found = find_command(&registry, "psychology.step").unwrap();
        assert_eq!(found.plugin_id, "psychology");
        assert_eq!(found.stability, Stability::Preview as i32);
        remove_plugin_commands(&mut registry, "psychology");
        assert!(find_command(&registry, "psychology.step").is_none());
        assert_eq!(registry.commands.len(), COMMANDS.len());
    }

    #[test]
    fn a_bad_command_is_skipped_and_the_rest_load() {
        let mut registry = CommandRegistry::default();
        let commands = vec![
            plugin_command("eval.test", "psychology"),
            plugin_command("model.load", "psychology"),
            plugin_command("psychology.step", "psychology"),
            plugin_command("psychology.step", "psychology"),
            plugin_command("psychology.live.start", "everything"),
        ];
        let refusals = add_plugin_commands(&mut registry, "psychology", commands).unwrap_err();
        let codes: Vec<&str> = refusals.iter().map(|refusal| refusal.code.as_str()).collect();
        assert_eq!(codes, ["RRS-P0004", "RRS-P0004", "RRS-E2005", "RRS-E1004"]);
        let ids: Vec<&str> = registry
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect();
        assert_eq!(ids, ["psychology.step"]);
    }

    #[test]
    fn loading_a_plugin_again_replaces_its_commands() {
        let mut registry = CommandRegistry::default();
        add_plugin_commands(&mut registry, "eval", vec![plugin_command("eval.test", "eval")]).unwrap();
        add_plugin_commands(&mut registry, "eval", vec![plugin_command("eval.vote", "eval")]).unwrap();
        let ids: Vec<&str> = registry
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect();
        assert_eq!(ids, ["eval.vote"]);
    }

    #[test]
    fn filters_pick_section_basic_and_experimental() {
        let mut shown = plugin_command("psychology.step", "psychology");
        shown.basic = true;
        let mut hidden = plugin_command("psychology.sae.load", "psychology");
        hidden.experimental = true;
        let mut other = plugin_command("eval.test", "eval");
        other.section = "eval".to_string();
        let registry = CommandRegistry {
            commands: vec![shown, hidden, other],
        };
        let ids = |filter: &CommandFilter| -> Vec<String> {
            list_commands(&registry, filter)
                .iter()
                .map(|command| command.command_id.clone())
                .collect()
        };
        assert_eq!(ids(&CommandFilter::default()), ["psychology.step", "eval.test"]);
        let everything = CommandFilter {
            include_experimental: true,
            ..CommandFilter::default()
        };
        assert_eq!(ids(&everything).len(), 3);
        let basic = CommandFilter {
            basic_only: true,
            ..CommandFilter::default()
        };
        assert_eq!(ids(&basic), ["psychology.step"]);
        let section = CommandFilter {
            section: "eval".to_string(),
            ..CommandFilter::default()
        };
        assert_eq!(ids(&section), ["eval.test"]);
    }

    #[test]
    fn a_token_needs_the_command_scope() {
        let command = plugin_command("psychology.step", "psychology");
        check_scope(&token_principal(vec![Scope::Psychology]), &command).unwrap();
        let refusal = check_scope(&token_principal(vec![Scope::Read]), &command).unwrap_err();
        assert_eq!(refusal.code, CODE_SCOPE_NOT_GRANTED);
    }

    #[test]
    fn the_owner_passes_a_plugin_own_scope() {
        let command = plugin_command("acme.tool.run", "plugin:acme");
        let owner = owner_principal();
        // scopes_allow alone says no: the owner's list can't name every plugin ahead of time.
        assert!(!scopes_allow(&owner.scopes, &Scope::Plugin("acme".to_string())));
        check_scope(&owner, &command).unwrap();
        let plugin = Principal {
            kind: PrincipalKind::Plugin,
            id: "acme".to_string(),
            scopes: vec![Scope::Plugin("acme".to_string())],
        };
        check_scope(&plugin, &command).unwrap();
        let other = check_scope(&token_principal(vec![Scope::Read]), &command).unwrap_err();
        assert_eq!(other.code, CODE_SCOPE_NOT_GRANTED);
    }

    #[test]
    fn an_unreadable_scope_fails_closed() {
        let command = plugin_command("psychology.step", "not a scope");
        let refusal = check_scope(&token_principal(vec![Scope::Read]), &command).unwrap_err();
        assert_eq!(refusal.code, CODE_SCOPE_NOT_GRANTED);
    }
}
