//! `dispatch`: one `match` arm per core command (contracts 6.2; layout rust-control-plane.md 8.3 and 10.1). No
//! reflection and no macros: adding a core command adds one arm here, and `tests/dispatch_complete.rs` fails until it
//! does.
//!
//! A command whose handler isn't built yet is an honest L0 stub (ADR-0017): its arm answers its area's "not available
//! yet" code (catalog/README.md) and never pretends to succeed. Anything without an arm falls through to the plugin
//! host: a plugin's command is forwarded to its module (M4; until then `RRS-P1004`), and a core-area command that
//! reaches the fall-through has no arm, which is a dispatch bug and answers `RRS-C0001`.

use rastai_core::commands::output::CommandOutput;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_formats::commands::parse_arguments;
use serde::de::DeserializeOwned;

use crate::commands::arguments::invalid_arguments;
use crate::commands::context::CommandContext;
use crate::commands::handlers::{command, job, node, plugin, settings, system, token, train};

const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";
const CODE_PLUGIN_COMMAND_NOT_AVAILABLE: &str = "RRS-P1004";
/// The area is a command id's first segment (contracts 6.1).
const AREA_SEPARATOR: char = '.';
/// The "not available yet" code of each core area (catalog/README.md). An area not listed is `system`'s.
const NOT_AVAILABLE_CODES: &[(&str, &str)] = &[
    ("plugin", "RRS-P1004"),
    ("storage", "RRS-S3006"),
    ("archive", "RRS-S3006"),
    ("secret", "RRS-S3006"),
    ("node", "RRS-N0005"),
    ("transfer", "RRS-N0005"),
    ("hf", "RRS-N0005"),
    ("chat", "RRS-G0011"),
    ("model", "RRS-G0011"),
    ("conversation", "RRS-G0011"),
    ("gateway", "RRS-G0011"),
    ("adapter", "RRS-G0011"),
    ("design", "RRS-M0004"),
    ("train", "RRS-T0006"),
    ("checkpoint", "RRS-T0006"),
    ("recipe", "RRS-T0006"),
    ("rlhf", "RRS-T0006"),
    ("sweep", "RRS-T0006"),
    ("teach", "RRS-T0006"),
    ("run", "RRS-T0006"),
    ("dataset", "RRS-D0002"),
    ("converter", "RRS-D0002"),
    ("gym", "RRS-R0007"),
    ("harness", "RRS-R0007"),
    ("sandbox", "RRS-R0007"),
];
/// The hardware and environment commands of `system` have a code of their own.
const HARDWARE_COMMAND_PREFIXES: &[&str] = &["system.doctor", "system.hardware", "system.env."];
const HARDWARE_NOT_AVAILABLE_CODE: &str = "RRS-H0003";
/// `system` (but the hardware commands), `settings`, `token`, `command`, `job`, `project`, `file`, `document`,
/// `workspace`, `artifact` and `tutorial`.
const DEFAULT_NOT_AVAILABLE_CODE: &str = "RRS-C0010";

/// Runs the command's handler. The arguments already passed `check_arguments`; each arm reads them into its own
/// `<Name>Args`.
pub async fn dispatch(context: &CommandContext<'_>, arguments_json: &str) -> Result<CommandOutput, Vec<Diagnostic>> {
    let command_id = context.command.command_id.as_str();
    match command_id {
        "system.status" => system::status(context, typed_arguments(command_id, arguments_json)?).await,
        "system.shutdown" => system::shutdown(context, typed_arguments(command_id, arguments_json)?).await,
        "command.list" => command::list(context, typed_arguments(command_id, arguments_json)?).await,
        "settings.get" => settings::get(context, typed_arguments(command_id, arguments_json)?).await,
        "settings.set" => settings::set(context, typed_arguments(command_id, arguments_json)?).await,
        "token.login_code.create" => {
            token::login_code_create(context, typed_arguments(command_id, arguments_json)?).await
        }
        "token.create" => token::create(context, typed_arguments(command_id, arguments_json)?).await,
        "token.list" => token::list(context, typed_arguments(command_id, arguments_json)?).await,
        "token.revoke" => token::revoke(context, typed_arguments(command_id, arguments_json)?).await,
        "plugin.list" => plugin::list(context, typed_arguments(command_id, arguments_json)?).await,
        "job.list" | "job.get" | "job.cancel" | "train.start" | "node.list" | "node.status" => {
            dispatch_jobs_and_nodes(context, arguments_json).await
        }
        _ => forward_to_plugin_host(context),
    }
}

/// The job, training and node commands: their own match, so each match stays short enough to read.
async fn dispatch_jobs_and_nodes(
    context: &CommandContext<'_>,
    arguments_json: &str,
) -> Result<CommandOutput, Vec<Diagnostic>> {
    let command_id = context.command.command_id.as_str();
    match command_id {
        "job.list" => job::list(context, typed_arguments(command_id, arguments_json)?).await,
        "job.get" => job::get(context, typed_arguments(command_id, arguments_json)?).await,
        "job.cancel" => job::cancel(context, typed_arguments(command_id, arguments_json)?).await,
        "train.start" => train::start(context, typed_arguments(command_id, arguments_json)?).await,
        "node.list" => node::list(context, typed_arguments(command_id, arguments_json)?).await,
        "node.status" => node::status(context, typed_arguments(command_id, arguments_json)?).await,
        _ => forward_to_plugin_host(context),
    }
}

/// The honest L0 stub: the area's "not available yet" code. Every M1 command has its handler now; the next
/// milestone's contract task registers its commands as stubs that answer through this again (plan 3.2).
#[allow(dead_code)]
pub(crate) fn not_available_yet(command_id: &str) -> Result<CommandOutput, Vec<Diagnostic>> {
    let code = not_available_code(command_id);
    Err(vec![diagnostic(code, &[("command_id", command_id)])])
}

/// The "not available yet" code of a command's area (catalog/README.md).
pub fn not_available_code(command_id: &str) -> &'static str {
    if HARDWARE_COMMAND_PREFIXES
        .iter()
        .any(|prefix| command_id.starts_with(prefix))
    {
        return HARDWARE_NOT_AVAILABLE_CODE;
    }
    let area = command_id.split(AREA_SEPARATOR).next().unwrap_or_default();
    match NOT_AVAILABLE_CODES.iter().find(|(known_area, _)| *known_area == area) {
        Some((_, code)) => code,
        None => DEFAULT_NOT_AVAILABLE_CODE,
    }
}

/// The arguments as the handler's type, with `RRS-C0002` in front of any problem, like `check_arguments`.
fn typed_arguments<Arguments: DeserializeOwned>(
    command_id: &str,
    arguments_json: &str,
) -> Result<Arguments, Vec<Diagnostic>> {
    parse_arguments(command_id, arguments_json).map_err(|findings| invalid_arguments(command_id, findings))
}

/// Where everything without an arm goes. Forwarding a plugin's command to its module arrives with the plugin modules
/// (M4); until then it is the plugin area's "not available yet". A core command here has no arm.
fn forward_to_plugin_host(context: &CommandContext<'_>) -> Result<CommandOutput, Vec<Diagnostic>> {
    let command_id = context.command.command_id.as_str();
    let code = if context.command.plugin_id.is_empty() {
        CODE_UNKNOWN_COMMAND
    } else {
        CODE_PLUGIN_COMMAND_NOT_AVAILABLE
    };
    Err(vec![diagnostic(code, &[("command_id", command_id)])])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_area_has_its_not_available_code() {
        assert_eq!(not_available_code("job.list"), "RRS-C0010");
        assert_eq!(not_available_code("settings.get"), "RRS-C0010");
        assert_eq!(not_available_code("train.start"), "RRS-T0006");
        assert_eq!(not_available_code("plugin.list"), "RRS-P1004");
        assert_eq!(not_available_code("node.status"), "RRS-N0005");
        assert_eq!(not_available_code("system.doctor"), "RRS-H0003");
        assert_eq!(not_available_code("system.env.install"), "RRS-H0003");
        assert_eq!(not_available_code("system.status"), "RRS-C0010");
        assert_eq!(not_available_code("model.load"), "RRS-G0011");
        assert_eq!(not_available_code("sandbox.list"), "RRS-R0007");
    }

    #[test]
    fn every_not_available_code_is_in_the_catalog() {
        let mut codes: Vec<&str> = NOT_AVAILABLE_CODES.iter().map(|(_, code)| *code).collect();
        codes.push(HARDWARE_NOT_AVAILABLE_CODE);
        codes.push(DEFAULT_NOT_AVAILABLE_CODE);
        codes.push(CODE_UNKNOWN_COMMAND);
        for code in codes {
            assert!(rastai_diag::catalog::find_entry(code).is_some(), "{code}");
        }
    }
}
