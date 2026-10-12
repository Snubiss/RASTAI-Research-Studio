//! `command.list` (contracts 6.4): the registry as a command, so the CLI and MCP can list commands with the same call
//! as everything else. `ListCommands` (http/command_endpoints.rs) reads the same registry.

use rastai_core::commands::output::CommandOutput;
use rastai_core::commands::registry::{CommandFilter, CommandRegistry, list_commands};
use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::commands::command::{CommandListArgs, CommandListResult};
use rastai_formats::commands::result_json;
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::Value;

use crate::commands::context::{CommandContext, sync_result};
// The descriptor's protobuf JSON is the codec's: one writer for the wire and for this result (contracts 4).
use crate::http::connect_json::command_descriptor_to_json;
use crate::server_state::Server;

/// The registered commands the filter lets through, core commands first, each as its protobuf JSON.
pub(crate) async fn list(
    context: &CommandContext<'_>,
    arguments: CommandListArgs,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let filter = CommandFilter {
        section: arguments.section,
        include_experimental: arguments.include_experimental,
        basic_only: arguments.basic_only,
    };
    let descriptors = filtered_commands(context.server, &filter);
    let result = CommandListResult {
        commands: descriptors.iter().map(descriptor_json_value).collect(),
    };
    Ok(sync_result(result_json(&result)))
}

/// Copies of the descriptors the filter lets through, so no lock is held after this returns.
pub(crate) fn filtered_commands(server: &Server, filter: &CommandFilter) -> Vec<CommandDescriptor> {
    let registry = read_registry(server);
    list_commands(&registry, filter).into_iter().cloned().collect()
}

fn read_registry(server: &Server) -> std::sync::RwLockReadGuard<'_, CommandRegistry> {
    match server.registry.read() {
        Ok(registry) => registry,
        // Poisoned by a panic mid-change, but whole: every change is one push or retain.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The codec writes valid JSON for every descriptor, so the parse back never fails; were it to, the entry would be
/// `null` rather than a guess.
fn descriptor_json_value(descriptor: &CommandDescriptor) -> Value {
    serde_json::from_str(&command_descriptor_to_json(descriptor)).unwrap_or(Value::Null)
}
