//! `execute_command`: where every caller ends (flows/platform.md 3.1 and 3.2). The UI, the CLI over daemon.sock,
//! MCP tools, plugin forwards and node calls all come here with a principal the gate or the node link derived, never
//! one the request names.
//!
//! The decisions, in order, each with its code:
//! 1. `acting_plugin_id` narrows the principal to that plugin's grant (the plugin host's); it can only remove rights.
//! 2. The command must be registered (`RRS-C0001`); a retired one names its replacement (`RRS-C0007`).
//! 3. The principal must hold its scope (`RRS-C1005`), checked before the arguments so a caller without the scope
//!    learns nothing about their shape.
//! 4. A plugin's command needs its plugin enabled (`RRS-C0003`).
//! 5. The arguments must match the command's description (`RRS-C0002` with the `RRS-E` problems and their paths).
//! 6. A dry run goes to the handler with `dry_run` set: no journal, no side effects.
//! 7. A key seen before replays the stored response, or is `RRS-C0004` for a different request (compared by a keyed
//!    hash of the unredacted arguments, so a different secret is a different request).
//! 8. Otherwise the journal's started row, the handler, then the finished row with the redacted result, then
//!    `command.finished` through the daemon log (`record_daemon_event`): logged, indexed, then published, so a
//!    rebuild keeps the journal and a UI hears about every journaled command.
//!
//! A command that fails is `ExecuteResponse{ok: false, diagnostics}`; only the gate answers with HTTP errors.

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
use rastai_proto::common::v1::PluginState;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{CommandFinished, EventPayload, EventRecord};

use crate::commands::arguments::check_arguments;
use crate::commands::context::CommandContext;
use crate::commands::dispatch::dispatch;
use crate::commands::journal::{
    arguments_hash, find_recent_journal_row, first_error_code, idempotency_key_lock, is_journaled, is_same_request,
    journal_finished, journal_started,
};
use crate::events::daemon_events::record_daemon_event;
use crate::plugin_host::load::{plugin_principal_scopes, plugin_status};
use crate::server_state::Server;

const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";
const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_PLUGIN_NOT_ENABLED: &str = "RRS-C0003";
const CODE_KEY_REUSED: &str = "RRS-C0004";
const CODE_RETIRED: &str = "RRS-C0007";
/// The fix of `RRS-C0003` (M8 builds it).
const PLUGIN_ENABLE_COMMAND_ID: &str = "plugin.enable";
const COMMAND_FINISHED_EVENT: &str = "command.finished";

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
    let plugin_grant = plugin_principal_scopes(server, &request.acting_plugin_id);
    let principal = narrow_to_acting_plugin(caller, &request.acting_plugin_id, plugin_grant)?;
    let command = find_descriptor(server, &request.command_id)?;
    check_scope(&principal, &command).map_err(|refusal| vec![refusal])?;
    check_plugin_enabled(&command, plugin_status(server, &command.plugin_id))?;
    check_arguments(&command, &request.arguments_json)?;
    Ok(CheckedCommand { principal, command })
}

/// A plugin UI's call runs as `plugin:<plugin_id>`, with only the scopes both the caller and the plugin's grant hold.
/// The grant comes from the plugin host: an enabled plugin's `commands:[scopes]` and its own scope; a plugin that
/// isn't enabled, or that the host doesn't know, holds nothing, so every scope check fails.
fn narrow_to_acting_plugin(
    caller: &Principal,
    acting_plugin_id: &str,
    plugin_grant: Vec<Scope>,
) -> Result<Principal, Vec<Diagnostic>> {
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

/// A plugin's command runs only while its plugin is enabled. A disabled plugin's commands leave the registry, so this
/// catches the moment between a state change and its unregistering, and a plugin the host doesn't know. The refusal
/// carries the plugin's own diagnostics after it, so the caller sees why.
fn check_plugin_enabled(
    command: &CommandDescriptor,
    plugin_status: Option<(PluginState, Vec<Diagnostic>)>,
) -> Result<(), Vec<Diagnostic>> {
    if command.plugin_id.is_empty() {
        return Ok(());
    }
    let plugin_diagnostics = match plugin_status {
        Some((PluginState::Enabled, _)) => return Ok(()),
        Some((_, diagnostics)) => diagnostics,
        None => Vec::new(),
    };
    let mut refusal = diagnostic(
        CODE_PLUGIN_NOT_ENABLED,
        &[("command_id", &command.command_id), ("plugin_id", &command.plugin_id)],
    );
    refusal.fix_command_id = PLUGIN_ENABLE_COMMAND_ID.to_string();
    let mut refusals = vec![refusal];
    refusals.extend(plugin_diagnostics);
    Err(refusals)
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
    let sent_arguments_hash = arguments_hash(&server.auth.owner_token_hash, &request.arguments_json);
    match find_recent_journal_row(server, &label, &request.idempotency_key) {
        Err(diagnostics) => unjournaled(Err(diagnostics)),
        Ok(None) => run_journaled(server, checked, request).await,
        Ok(Some(row)) if !is_same_request(&request.command_id, &sent_arguments_hash, &row) => {
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
    // After the finished row, whose redacted result the record doesn't carry and an earlier finish would keep out.
    let record = command_finished_record(&request.command_id, &label, &journal_id, &result);
    if let Err(error) = record_daemon_event(server, record).await {
        log_warn(
            "a command's command.finished record could not be logged and indexed",
            &[("journal_id", &journal_id), ("code", error.code())],
        );
    }
    CommandOutcome { journal_id, result }
}

/// The daemon-log record of a journaled command's end (contracts 4.3): its outcome and first error's code.
fn command_finished_record(
    command_id: &str,
    principal_label: &str,
    journal_id: &str,
    result: &Result<CommandOutput, Vec<Diagnostic>>,
) -> EventRecord {
    let (ok, code) = match result {
        Ok(_) => (true, String::new()),
        Err(diagnostics) => (false, first_error_code(diagnostics)),
    };
    let finished = CommandFinished {
        journal_id: journal_id.to_string(),
        command_id: command_id.to_string(),
        principal: principal_label.to_string(),
        ok,
        code,
    };
    EventRecord {
        event_type: COMMAND_FINISHED_EVENT.to_string(),
        payload: Some(EventPayload {
            payload: Some(Payload::CommandFinished(finished)),
        }),
        ..EventRecord::default()
    }
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
        let narrowed = narrow_to_acting_plugin(&owner_principal(), "acme", Vec::new()).unwrap();
        assert_eq!(narrowed.kind, PrincipalKind::Plugin);
        assert_eq!(narrowed.id, "acme");
        assert!(narrowed.scopes.is_empty());
        assert_eq!(principal_label(&narrowed), "plugin:acme");
        let unchanged = narrow_to_acting_plugin(&owner_principal(), "", Vec::new()).unwrap();
        assert_eq!(unchanged, owner_principal());
        // The owner narrowed to a plugin holds the plugin's grant; a token keeps only what it holds itself.
        let grant = vec![Scope::Read, Scope::Plugin("acme".to_string())];
        let as_owner = narrow_to_acting_plugin(&owner_principal(), "acme", grant.clone()).unwrap();
        assert_eq!(as_owner.scopes, grant);
        let token = Principal {
            kind: PrincipalKind::Token,
            id: "tok_01J".to_string(),
            scopes: vec![Scope::Read, Scope::SystemAdmin],
        };
        let as_token = narrow_to_acting_plugin(&token, "acme", grant).unwrap();
        assert_eq!(as_token.scopes, [Scope::Read]);
    }

    #[test]
    fn an_acting_plugin_id_that_is_not_one_is_refused() {
        for bad in ["Acme", "acme_", "a.b", "plugin:acme"] {
            let refusal = narrow_to_acting_plugin(&owner_principal(), bad, Vec::new()).unwrap_err();
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
        check_plugin_enabled(&core, None).unwrap();
        let plugin = CommandDescriptor {
            command_id: "acme.tool.run".to_string(),
            plugin_id: "acme".to_string(),
            ..CommandDescriptor::default()
        };
        let refusal = check_plugin_enabled(&plugin, None).unwrap_err();
        assert_eq!(refusal[0].code, CODE_PLUGIN_NOT_ENABLED);
        assert_eq!(refusal[0].fix_command_id, "plugin.enable");
        check_plugin_enabled(&plugin, Some((PluginState::Enabled, Vec::new()))).unwrap();
        let why = vec![diagnostic(
            "RRS-P3001",
            &[("dependency", "base"), ("plugin_id", "acme")],
        )];
        let refusal = check_plugin_enabled(&plugin, Some((PluginState::Disabled, why))).unwrap_err();
        let codes: Vec<&str> = refusal.iter().map(|item| item.code.as_str()).collect();
        assert_eq!(codes, [CODE_PLUGIN_NOT_ENABLED, "RRS-P3001"]);
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
