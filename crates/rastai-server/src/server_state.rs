//! `Server`, the plain struct every handler reads, shared as `Arc<Server>` (layout section 8.2). Tables sit behind
//! std locks that are never held across an `.await`.
//!
//! M1-26 brought the fields startup and sign-in need; M1-27 the command registry, the settings layers that sit above
//! the files, the idempotency locks and the port; M1-32 the plugin host. The bus (M1-28), the node links and identity
//! (M1-42, M1-47), the gateway and the rest join with the tasks that build them.

use std::sync::{Mutex, OnceLock, RwLock};

use rastai_core::commands::registry::{CommandRegistry, load_core_commands};
use rastai_core::paths::DataPaths;
use rastai_core::settings::{ResolvedSettings, SettingsLayer};
use rastai_formats::commands::{describe_arguments, describe_result};
use rastai_formats::describe::json_schema::json_schema_for;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::describe::types::TypeDescription;
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_store::store::Store;
use tokio::sync::watch;

use crate::commands::journal::IdempotencyKeyLock;
use crate::http::sessions::AuthState;
use crate::plugin_host::load::PluginHost;

pub struct Server {
    pub paths: DataPaths,
    /// The settings in effect outside any project: the defaults, the user's `settings.json`, then the overrides.
    pub settings: RwLock<ResolvedSettings>,
    /// The environment and `--set` layers, read once at startup. They sit above the files, so `settings.set`
    /// resolves with them again after it writes one (contracts 11.1).
    pub settings_overrides: Vec<SettingsLayer>,
    pub store: Store,
    /// Every command, core and plugin, with its argument and result schemas (layout 10.2). Rebuilt at every start.
    pub registry: RwLock<CommandRegistry>,
    /// Every plugin discovery found, with its state and diagnostics (plugin_host/). Rebuilt at every start.
    pub plugins: RwLock<PluginHost>,
    /// Login codes, sessions, the owner token's hash and the scoped tokens' hashes.
    pub auth: AuthState,
    /// One lock per (principal, idempotency key) while a command with that key runs (commands/journal.rs).
    pub idempotency_locks: Mutex<Vec<IdempotencyKeyLock>>,
    /// The loopback port, set once the listener is bound; `token.login_code.create` reports it.
    pub http_port: OnceLock<u16>,
    /// Set to true to stop the daemon (`request_shutdown`): the signal handler and `system.shutdown` do. The
    /// listeners watch it.
    pub shutdown: watch::Sender<bool>,
}

pub fn build_server(
    paths: DataPaths,
    settings: ResolvedSettings,
    settings_overrides: Vec<SettingsLayer>,
    store: Store,
    auth: AuthState,
) -> Server {
    let (shutdown, _first_receiver) = watch::channel(false);
    Server {
        paths,
        settings: RwLock::new(settings),
        settings_overrides,
        store,
        registry: RwLock::new(core_command_registry()),
        plugins: RwLock::new(PluginHost::default()),
        auth,
        idempotency_locks: Mutex::new(Vec::new()),
        http_port: OnceLock::new(),
        shutdown,
    }
}

/// The core table, with each command's argument and result types and their JSON Schemas filled in from
/// rastai-formats' descriptions (layout 10.2): what `ListCommands`, MCP's `inputSchema` and the palette's forms read.
fn core_command_registry() -> CommandRegistry {
    let all_types = describe_all_types();
    let mut registry = load_core_commands();
    for command in &mut registry.commands {
        fill_command_types(command, &all_types);
    }
    registry
}

fn fill_command_types(command: &mut CommandDescriptor, all_types: &[TypeDescription]) {
    if let Some(arguments) = describe_arguments(&command.command_id) {
        command.arguments_schema_json = json_schema_for(&arguments, all_types).to_string();
        command.arguments_type = arguments.name;
    }
    if let Some(result) = describe_result(&command.command_id) {
        command.result_schema_json = json_schema_for(&result, all_types).to_string();
        command.result_type = result.name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_command_carries_its_types_and_schemas() {
        let registry = core_command_registry();
        assert!(!registry.commands.is_empty());
        for command in &registry.commands {
            assert!(command.arguments_type.ends_with("Args"), "{}", command.command_id);
            assert!(command.result_type.ends_with("Result"), "{}", command.command_id);
            let arguments: serde_json::Value = serde_json::from_str(&command.arguments_schema_json).unwrap_or_default();
            assert_eq!(arguments["type"], "object", "{}", command.command_id);
            let result: serde_json::Value = serde_json::from_str(&command.result_schema_json).unwrap_or_default();
            assert_eq!(result["title"], command.result_type.as_str(), "{}", command.command_id);
        }
    }
}
