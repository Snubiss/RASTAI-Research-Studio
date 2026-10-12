//! `execute_command`: where every caller ends (flows/platform.md 3.1 and 3.2). The UI, the CLI over daemon.sock,
//! MCP tools, plugin forwards and node calls all come here with a principal the gate or the node link derived, never
//! one the request names.
//!
//! The decisions, in order, each with its code:
//! 1. `acting_plugin_id` narrows the principal to that plugin's grant; it can only remove rights.
//! 2. The command must be registered (`RRS-C0001`); a retired one names its replacement (`RRS-C0007`).
//! 3. The principal must hold its scope (`RRS-C1005`), checked before the arguments so a caller without the scope
//!    learns nothing about their shape.
//! 4. A plugin's command needs its plugin enabled (`RRS-C0003`).
//! 5. The arguments must match the command's description (`RRS-C0002` with the `RRS-E` problems and their paths).
//! 6. A dry run goes to the handler with `dry_run` set: no journal, no side effects.
//! 7. A key seen before replays the stored response, or is `RRS-C0004` for a different request.
//! 8. Otherwise the journal's started row, the handler, then the finished row with the redacted result.
//!
//! A command that fails is `ExecuteResponse{ok: false, diagnostics}`; only the gate answers with HTTP errors.
//! Publishing `command.finished` on the bus and in the daemon log joins with the bus (M1-28).

use rastai_core::commands::output::CommandOutput;
use rastai_core::commands::registry::{check_scope, find_command};
use rastai_core::commands::table::{RETIRED_COMMANDS, RetiredCommand};
use rastai_core::principal::{Principal, PrincipalKind, principal_label};
use rastai_core::scopes::{Scope, scopes_allow};
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_formats::validate::meaning::plugin_manifest::{EXPECTED_PLUGIN_ID_FORM, is_plugin_id};
use rastai_proto::commands::v1::{CommandDescriptor, ExecuteRequest, ExecuteResponse};

use crate::commands::arguments::check_arguments;
use crate::commands::context::CommandContext;
use crate::commands::dispatch::dispatch;
use crate::commands::journal::{
    find_recent_journal_row, idempotency_key_lock, is_journaled, is_same_request, journal_finished, journal_started,
};
use crate::server_state::Server;

const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";
const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_PLUGIN_NOT_ENABLED: &str = "RRS-C0003";
const CODE_KEY_REUSED: &str = "RRS-C0004";
const CODE_RETIRED: &str = "RRS-C0007";

/// The outcome of one request, and the journal row it has (empty when it has none).
struct CommandOutcome {
    journal_id: String,
    result: Result<CommandOutput, Vec<Diagnostic>>,
}

/// A request that passed every check up to its arguments: who runs it, and the registry's descriptor.
struct CheckedCommand {
    principal: Principal,
    command: CommandDescriptor,
}

/// Runs one command for `caller`. Never fails: every failure is in the response.
pub async fn execute_command(server: &Server, caller: &Principal, request: ExecuteRequest) -> ExecuteResponse {
    let outcome = run_command(server, caller, &request).await;
    response_for(outcome)
}

async fn run_command(server: &Server, caller: &Principal, request: &ExecuteRequest) -> CommandOutcome {
    match check_command(server, caller, request) {
        Ok(checked) => run_checked_command(server, &checked, request).await,
        Err(diagnostics) => unjournaled(Err(diagnostics)),
    }
}

/// Steps 1 to 5 of the header. Nothing is written until they all pass.
fn check_command(
    server: &Server,
    caller: &Principal,
    request: &ExecuteRequest,
) -> Result<CheckedCommand, Vec<Diagnostic>> {
    let principal = narrow_to_acting_plugin(caller, &request.acting_plugin_id)?;
    let command = find_descriptor(server, &request.command_id)?;
    check_scope(&principal, &command).map_err(|refusal| vec![refusal])?;
    check_plugin_enabled(&command)?;
    check_arguments(&command, &request.arguments_json)?;
    Ok(CheckedCommand { principal, command })
}

/// A plugin UI's call runs as `plugin:<plugin_id>`, with only the scopes both the caller and the plugin's grant hold.
/// The grant comes from the plugin host (M1-32); until it loads plugins none is enabled, so the grant is empty and
/// every scope check fails, as for a disabled plugin.
fn narrow_to_acting_plugin(caller: &Principal, acting_plugin_id: &str) -> Result<Principal, Vec<Diagnostic>> {
    if acting_plugin_id.is_empty() {
        return Ok(caller.clone());
    }
    if !is_plugin_id(acting_plugin_id) {
        let refusal = diagnostic(
            CODE_INVALID_ARGUMENTS,
            &[("found", acting_plugin_id), ("expected", EXPECTED_PLUGIN_ID_FORM)],
        );
        return Err(vec![refusal]);
    }
    let plugin_grant: Vec<Scope> = Vec::new();
    let scopes = plugin_grant
        .into_iter()
        .filter(|scope| caller.kind == PrincipalKind::Owner || scopes_allow(&caller.scopes, scope))
        .collect();
    Ok(Principal {
        kind: PrincipalKind::Plugin,
        id: acting_plugin_id.to_string(),
        scopes,
    })
}

/// A copy of the registry's descriptor, so no lock is held while the command runs.
fn find_descriptor(server: &Server, command_id: &str) -> Result<CommandDescriptor, Vec<Diagnostic>> {
    if let Some(retired) = RETIRED_COMMANDS.iter().find(|retired| retired.id == command_id) {
        return Err(vec![retired_command_refusal(retired)]);
    }
    let registry = match server.registry.read() {
        Ok(registry) => registry,
        // A panic while the registry was being changed leaves it poisoned but whole (each change is a push or a
        // retain), so lookups carry on.
        Err(poisoned) => poisoned.into_inner(),
    };
    match find_command(&registry, command_id) {
        Some(command) => Ok(command.clone()),
        None => Err(vec![diagnostic(CODE_UNKNOWN_COMMAND, &[("command_id", command_id)])]),
    }
}

/// `RRS-C0007`, whose fix button runs the replacement (layout rust-control-plane.md 11.2).
fn retired_command_refusal(retired: &RetiredCommand) -> Diagnostic {
    let mut refusal = diagnostic(
        CODE_RETIRED,
        &[
            ("command_id", retired.id),
            ("replacement_command_id", retired.replaced_by),
        ],
    );
    refusal.fix_command_id = retired.replaced_by.to_string();
    refusal
}

/// A plugin's command runs only while its plugin is enabled. The plugin host (M1-32) keeps the plugins' states;
/// until it lands no plugin is enabled, and the registry holds no plugin command to reach this.
fn check_plugin_enabled(command: &CommandDescriptor) -> Result<(), Vec<Diagnostic>> {
    if command.plugin_id.is_empty() {
        return Ok(());
    }
    let mut refusal = diagnostic(
        CODE_PLUGIN_NOT_ENABLED,
        &[("command_id", &command.command_id), ("plugin_id", &command.plugin_id)],
    );
    refusal.fix_command_id = "plugin.enable".to_string();
    Err(vec![refusal])
}

/// Steps 6 to 8 of the header.
async fn run_checked_command(server: &Server, checked: &CheckedCommand, request: &ExecuteRequest) -> CommandOutcome {
    if request.dry_run || !is_journaled(&checked.command) {
        let context = command_context(server, checked, request, String::new());
        return unjournaled(dispatch(&context, &request.arguments_json).await);
    }
    if request.idempotency_key.is_empty() {
        return run_journaled(server, checked, request).await;
    }
    let label = principal_label(&checked.principal);
    let key_lock = idempotency_key_lock(server, &label, &request.idempotency_key);
    // Held until the finished row is written, so a duplicate sent meanwhile waits and then finds it.
    let _key_guard = key_lock.lock().await;
    match find_recent_journal_row(server, &label, &request.idempotency_key) {
        Err(diagnostics) => unjournaled(Err(diagnostics)),
        Ok(None) => run_journaled(server, checked, request).await,
        Ok(Some(row)) if !is_same_request(&request.command_id, &request.arguments_json, &row) => {
            let refusal = diagnostic(CODE_KEY_REUSED, &[("idempotency_key", &request.idempotency_key)]);
            unjournaled(Err(vec![refusal]))
        }
        // The stored response, as the journal holds it: its result redacted.
        Ok(Some(row)) if row.ok == Some(true) => CommandOutcome {
            journal_id: row.journal_id,
            result: Ok(CommandOutput {
                result_json: row.result_json,
                job_id: row.job_id,
                warnings: Vec::new(),
            }),
        },
        // An earlier attempt that failed or was cut off by a restart changed nothing to keep: a retry runs again.
        Ok(Some(_failed)) => run_journaled(server, checked, request).await,
    }
}

async fn run_journaled(server: &Server, checked: &CheckedCommand, request: &ExecuteRequest) -> CommandOutcome {
    let label = principal_label(&checked.principal);
    let journal_id = match journal_started(server, &label, request).await {
        Ok(journal_id) => journal_id,
        Err(diagnostics) => return unjournaled(Err(diagnostics)),
    };
    let context = command_context(server, checked, request, journal_id.clone());
    let result = dispatch(&context, &request.arguments_json).await;
    // The command has run, so its answer goes back even when its row can't be finished; the next start marks that
    // row interrupted (RRS-C0005).
    if let Err(diagnostics) = journal_finished(server, &request.command_id, &journal_id, &result).await {
        let code = diagnostics.first().map(|item| item.code.clone()).unwrap_or_default();
        log_warn(
            "a command's journal row could not be finished",
            &[("journal_id", &journal_id), ("code", &code)],
        );
    }
    CommandOutcome { journal_id, result }
}

fn command_context<'a>(
    server: &'a Server,
    checked: &'a CheckedCommand,
    request: &ExecuteRequest,
    journal_id: String,
) -> CommandContext<'a> {
    CommandContext {
        server,
        principal: &checked.principal,
        command: &checked.command,
        journal_id,
        project_id: request.project_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        dry_run: request.dry_run,
    }
}

fn unjournaled(result: Result<CommandOutput, Vec<Diagnostic>>) -> CommandOutcome {
    CommandOutcome {
        journal_id: String::new(),
        result,
    }
}

/// The wire answer. A success carries the handler's warnings; a failure every diagnostic.
fn response_for(outcome: CommandOutcome) -> ExecuteResponse {
    match outcome.result {
        Ok(output) => ExecuteResponse {
            ok: true,
            result_json: output.result_json,
            job_id: output.job_id,
            diagnostics: output.warnings.iter().map(diagnostic_to_proto).collect(),
            journal_id: outcome.journal_id,
        },
        Err(diagnostics) => ExecuteResponse {
            ok: false,
            result_json: String::new(),
            job_id: String::new(),
            diagnostics: diagnostics.iter().map(diagnostic_to_proto).collect(),
            journal_id: outcome.journal_id,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::principal::owner_principal;

    #[test]
    fn an_acting_plugin_only_removes_rights() {
        let narrowed = narrow_to_acting_plugin(&owner_principal(), "acme").unwrap();
        assert_eq!(narrowed.kind, PrincipalKind::Plugin);
        assert_eq!(narrowed.id, "acme");
        assert!(narrowed.scopes.is_empty());
        assert_eq!(principal_label(&narrowed), "plugin:acme");
        let unchanged = narrow_to_acting_plugin(&owner_principal(), "").unwrap();
        assert_eq!(unchanged, owner_principal());
    }

    #[test]
    fn an_acting_plugin_id_that_is_not_one_is_refused() {
        for bad in ["Acme", "acme_", "a.b", "plugin:acme"] {
            let refusal = narrow_to_acting_plugin(&owner_principal(), bad).unwrap_err();
            assert_eq!(refusal[0].code, CODE_INVALID_ARGUMENTS, "{bad}");
        }
    }

    #[test]
    fn a_retired_command_names_its_replacement() {
        let retired = RetiredCommand {
            id: "model.fetch",
            replaced_by: "model.download",
            retired_in: "0.4.0",
        };
        let refusal = retired_command_refusal(&retired);
        assert_eq!(refusal.code, CODE_RETIRED);
        assert_eq!(refusal.fix_command_id, "model.download");
        assert!(refusal.message.contains("model.fetch"), "{}", refusal.message);
    }

    #[test]
    fn a_plugin_command_is_refused_until_its_plugin_is_enabled() {
        let core = CommandDescriptor::default();
        check_plugin_enabled(&core).unwrap();
        let plugin = CommandDescriptor {
            command_id: "acme.tool.run".to_string(),
            plugin_id: "acme".to_string(),
            ..CommandDescriptor::default()
        };
        let refusal = check_plugin_enabled(&plugin).unwrap_err();
        assert_eq!(refusal[0].code, CODE_PLUGIN_NOT_ENABLED);
        assert_eq!(refusal[0].fix_command_id, "plugin.enable");
    }

    #[test]
    fn responses_carry_warnings_or_diagnostics_and_the_journal_id() {
        let success = response_for(CommandOutcome {
            journal_id: "jrn_a".to_string(),
            result: Ok(CommandOutput {
                result_json: "{}".to_string(),
                job_id: String::new(),
                warnings: vec![diagnostic("RRS-C0006", &[])],
            }),
        });
        assert!(success.ok);
        assert_eq!(success.journal_id, "jrn_a");
        assert_eq!(success.diagnostics[0].code, "RRS-C0006");
        let failure = response_for(unjournaled(Err(vec![diagnostic("RRS-C0001", &[])])));
        assert!(!failure.ok);
        assert!(failure.journal_id.is_empty());
        assert_eq!(failure.diagnostics[0].code, "RRS-C0001");
    }
}
