//! `check_arguments`, layer 2 for every command before it runs (flows/platform.md 3.2): the arguments against the
//! command's description, for core and plugin commands alike, so every caller gets `RRS-C0002` with paths.
//!
//! rastai-formats does the reading (`parse_arguments`: the strict reader, then the description); this file puts
//! `RRS-C0002` in front of what it finds, so a caller sees "invalid arguments" first and each `RRS-E` problem, with
//! its JSON pointer, after it.

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_formats::commands::{describe_arguments, parse_arguments};
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::Value;

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
/// What a command without a description (a plugin's, until its `types.json` is read, M4) takes.
const ANY_ARGUMENTS: &str = "a JSON object";
/// How the summary names a problem about the arguments as a whole, which has no pointer.
const WHOLE_ARGUMENTS: &str = "the top level";

/// The arguments are valid for the command, or `RRS-C0002` followed by each problem found.
pub fn check_arguments(command: &CommandDescriptor, arguments_json: &str) -> Result<(), Vec<Diagnostic>> {
    match parse_arguments::<Value>(&command.command_id, arguments_json) {
        Ok(_) => Ok(()),
        Err(findings) => Err(invalid_arguments(&command.command_id, findings)),
    }
}

/// `RRS-C0002` in front of rastai-formats' findings. It points where the first finding points and says how many
/// there are, so the one line a CLI or a toast shows is already useful.
pub(crate) fn invalid_arguments(command_id: &str, findings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let first_path = findings.first().map(|finding| finding.path.clone()).unwrap_or_default();
    let location = if first_path.is_empty() {
        WHOLE_ARGUMENTS.to_string()
    } else {
        first_path.clone()
    };
    let found = match findings.len() {
        1 => format!("1 problem, at {location}"),
        count => format!("{count} problems, the first at {location}"),
    };
    let expected = expected_arguments(command_id);
    let summary = diagnostic_at(
        CODE_INVALID_ARGUMENTS,
        &[("found", &found), ("expected", &expected)],
        &first_path,
    );
    let mut diagnostics = vec![summary];
    diagnostics.extend(findings);
    diagnostics
}

fn expected_arguments(command_id: &str) -> String {
    match describe_arguments(command_id) {
        Some(description) => format!("arguments that match {}", description.name),
        None => ANY_ARGUMENTS.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn command(command_id: &str) -> CommandDescriptor {
        CommandDescriptor {
            command_id: command_id.to_string(),
            ..CommandDescriptor::default()
        }
    }

    #[test]
    fn valid_arguments_pass() {
        check_arguments(&command("token.create"), r#"{"name": "ci"}"#).unwrap();
        check_arguments(&command("system.status"), "").unwrap();
        check_arguments(&command("acme.tool.run"), r#"{"anything": 1}"#).unwrap();
    }

    #[test]
    fn invalid_arguments_lead_with_c0002_and_keep_every_path() {
        let refusal = check_arguments(&command("token.create"), r#"{"name": 5, "colour": "red"}"#).unwrap_err();
        let codes: Vec<(&str, &str)> = refusal
            .iter()
            .map(|item| (item.code.as_str(), item.path.as_str()))
            .collect();
        assert_eq!(
            codes,
            [
                ("RRS-C0002", "#/name"),
                ("RRS-E1002", "#/name"),
                ("RRS-E1003", "#/colour")
            ]
        );
        assert!(refusal[0].message.contains("2 problems"), "{}", refusal[0].message);
        assert!(refusal[0].message.contains("TokenCreateArgs"), "{}", refusal[0].message);
    }

    #[test]
    fn a_problem_with_the_whole_value_has_no_path() {
        let refusal = check_arguments(&command("acme.tool.run"), "[1]").unwrap_err();
        assert_eq!(refusal[0].code, "RRS-C0002");
        assert_eq!(refusal[0].path, "");
        assert!(refusal[0].message.contains("the top level"), "{}", refusal[0].message);
        assert!(refusal[0].message.contains("a JSON object"), "{}", refusal[0].message);
    }
}
