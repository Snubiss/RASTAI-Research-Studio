//! The argument and result types of the core commands (contracts 6.4, 6.5; layout rust-control-plane.md 4.1 and
//! 10.1). Each area of contracts 6.1 has one file here with a `<Name>Args` and a `<Name>Result` struct per command,
//! plus the smaller types they are built from, each with its hand-written `describe_<type>()`.
//!
//! `COMMAND_TYPES` ties each core command id to its two descriptions. It is the one place the codegen and the
//! daemon look them up (`describe_arguments`, `describe_result`), and `tests/commands_described.rs` checks that
//! every row of rastai-core's `COMMANDS` has one. A command's types share one schema id,
//! `rastai.command.<command_id>/1` (`command_schema_id`), so the generated JavaScript and Python put them in one
//! module per command.
//!
//! `parse_arguments` is how the daemon reads a command's `arguments_json`, which comes from whoever called it: the
//! strict reader (layer 1), then the arguments' description (layer 2, every problem with its JSON pointer), then
//! serde. `result_json` writes a handler's result back out.

pub mod command;
pub mod job;
pub mod node;
pub mod plugin;
pub mod settings;
pub mod system;
pub mod token;
pub mod train;

use rastai_diag::diagnostic::{Diagnostic, has_errors};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::describe::registry::describe_all_types;
use crate::describe::structure_check::StructureCheck;
use crate::describe::types::{FieldDescription, TypeDescription};
use crate::json::reader::{ParsedJson, read_json};
use crate::limits::ImportLimits;
use crate::validate::structure::check_structure;
use crate::validate::syntax::syntax_diagnostic;
use crate::validate::wrong_kind_diagnostic;

/// Every command's arguments and result share one schema id with this prefix (contracts 5.2, layout 2.7).
const COMMAND_SCHEMA_ID_PREFIX: &str = "rastai.command.";
/// The version of every command schema. A breaking change to a `stable` command makes a new command id instead
/// (layout 11.1), so the version stays 1.
const COMMAND_SCHEMA_VERSION: u32 = 1;
/// An `ExecuteRequest` that leaves `arguments_json` out (proto3's empty text) passes no arguments: `{}`.
const NO_ARGUMENTS: &str = "{}";
/// Arguments aren't a file, so a diagnostic about them points with a JSON pointer alone: `#/name`.
const ARGUMENTS_FILE_NAME: &str = "";
/// What the arguments of a command without a description must be (a plugin's, until its `types.json` is read, M4).
const ANY_ARGUMENTS_OBJECT: &str = "an object";

/// One core command's argument and result descriptions.
#[derive(Debug, Clone, Copy)]
pub struct CommandTypes {
    pub command_id: &'static str,
    pub describe_arguments: fn() -> TypeDescription,
    pub describe_result: fn() -> TypeDescription,
}

/// One row per core command, in the order of rastai-core's `COMMANDS`.
pub const COMMAND_TYPES: &[CommandTypes] = &[
    CommandTypes {
        command_id: "system.status",
        describe_arguments: system::describe_system_status_args,
        describe_result: system::describe_system_status_result,
    },
    CommandTypes {
        command_id: "system.shutdown",
        describe_arguments: system::describe_system_shutdown_args,
        describe_result: system::describe_system_shutdown_result,
    },
    CommandTypes {
        command_id: "command.list",
        describe_arguments: command::describe_command_list_args,
        describe_result: command::describe_command_list_result,
    },
    CommandTypes {
        command_id: "job.list",
        describe_arguments: job::describe_job_list_args,
        describe_result: job::describe_job_list_result,
    },
    CommandTypes {
        command_id: "job.get",
        describe_arguments: job::describe_job_get_args,
        describe_result: job::describe_job_get_result,
    },
    CommandTypes {
        command_id: "job.cancel",
        describe_arguments: job::describe_job_cancel_args,
        describe_result: job::describe_job_cancel_result,
    },
    CommandTypes {
        command_id: "train.start",
        describe_arguments: train::describe_train_start_args,
        describe_result: train::describe_train_start_result,
    },
    CommandTypes {
        command_id: "settings.get",
        describe_arguments: settings::describe_settings_get_args,
        describe_result: settings::describe_settings_get_result,
    },
    CommandTypes {
        command_id: "settings.set",
        describe_arguments: settings::describe_settings_set_args,
        describe_result: settings::describe_settings_set_result,
    },
    CommandTypes {
        command_id: "token.login_code.create",
        describe_arguments: token::describe_token_login_code_create_args,
        describe_result: token::describe_token_login_code_create_result,
    },
    CommandTypes {
        command_id: "token.create",
        describe_arguments: token::describe_token_create_args,
        describe_result: token::describe_token_create_result,
    },
    CommandTypes {
        command_id: "token.list",
        describe_arguments: token::describe_token_list_args,
        describe_result: token::describe_token_list_result,
    },
    CommandTypes {
        command_id: "token.revoke",
        describe_arguments: token::describe_token_revoke_args,
        describe_result: token::describe_token_revoke_result,
    },
    CommandTypes {
        command_id: "plugin.list",
        describe_arguments: plugin::describe_plugin_list_args,
        describe_result: plugin::describe_plugin_list_result,
    },
    CommandTypes {
        command_id: "node.list",
        describe_arguments: node::describe_node_list_args,
        describe_result: node::describe_node_list_result,
    },
    CommandTypes {
        command_id: "node.status",
        describe_arguments: node::describe_node_status_args,
        describe_result: node::describe_node_status_result,
    },
];

/// The types the arguments and results above are built from. Each belongs to the command that first needs it
/// (its schema id), and the others refer to it by name.
const COMMAND_PART_TYPES: &[fn() -> TypeDescription] = &[
    job::describe_job_summary,
    node::describe_node_summary,
    plugin::describe_plugin_summary,
    settings::describe_setting_key_source,
    token::describe_token_summary,
    train::describe_training_plan,
];

/// `rastai.command.model.load/1`: the schema id every type of one command shares.
pub fn command_schema_id(command_id: &str) -> String {
    format!("{COMMAND_SCHEMA_ID_PREFIX}{command_id}/{COMMAND_SCHEMA_VERSION}")
}

/// The description of a core command's arguments, or None for an id no row has (a plugin's command, whose types
/// come from its `types.json`).
pub fn describe_arguments(command_id: &str) -> Option<TypeDescription> {
    let row = find_command_types(command_id)?;
    Some((row.describe_arguments)())
}

/// The description of a core command's result, or None for an id no row has.
pub fn describe_result(command_id: &str) -> Option<TypeDescription> {
    let row = find_command_types(command_id)?;
    Some((row.describe_result)())
}

/// A command's arguments as its `<Name>Args` struct. Every problem layer 2 finds comes back at once, each with its
/// JSON pointer (`#/scopes/0`): `RRS-E0xxx` and the `RRS-E5xxx` limits for text that isn't JSON, `RRS-E1xxx` for a
/// missing field, a value of the wrong kind, a key the type doesn't have or a value its enum doesn't allow. A
/// command without a description (a plugin's) is only checked to be a JSON object. Empty text means `{}`.
pub fn parse_arguments<T: DeserializeOwned>(command_id: &str, arguments_json: &str) -> Result<T, Vec<Diagnostic>> {
    let value = check_arguments_json(command_id, arguments_json)?;
    // Layer 2 passed, so serde refuses only an integer outside its field's Rust type (a negative count).
    serde_json::from_value(value).map_err(|error| vec![wrong_kind_diagnostic(ARGUMENTS_FILE_NAME, &error.to_string())])
}

/// A handler's result as the text `ExecuteResponse.result_json` carries. Results are plain structs of text, numbers,
/// lists and JSON values, which serde always writes; were one ever refused, the result would be empty rather than
/// half written.
pub fn result_json<T: Serialize>(result: &T) -> String {
    serde_json::to_string(result).unwrap_or_default()
}

/// Layers 1 and 2 over the arguments' text: the JSON value, or every problem found.
fn check_arguments_json(command_id: &str, arguments_json: &str) -> Result<Value, Vec<Diagnostic>> {
    let text = if arguments_json.is_empty() {
        NO_ARGUMENTS
    } else {
        arguments_json
    };
    let parsed = match read_json(text.as_bytes(), &ImportLimits::default()) {
        Ok(parsed) => parsed,
        Err(error) => return Err(vec![syntax_diagnostic(&error, ARGUMENTS_FILE_NAME)]),
    };
    let diagnostics = structure_diagnostics(command_id, &parsed);
    if has_errors(&diagnostics) {
        return Err(diagnostics);
    }
    Ok(parsed.value)
}

fn structure_diagnostics(command_id: &str, parsed: &ParsedJson) -> Vec<Diagnostic> {
    let all_types = describe_all_types();
    let (description, report_unknown_fields) = match describe_arguments(command_id) {
        Some(description) => (description, true),
        // Its keys can't be judged without a description, so only the object itself is checked.
        None => (any_object_description(command_id), false),
    };
    let check = StructureCheck {
        all_types: &all_types,
        report_unknown_fields,
    };
    check_structure(
        &parsed.value,
        &description,
        &check,
        ARGUMENTS_FILE_NAME,
        &parsed.positions,
    )
}

fn any_object_description(command_id: &str) -> TypeDescription {
    command_type(ANY_ARGUMENTS_OBJECT, command_id, "", Vec::new())
}

/// Every command type: each command's arguments and result in table order, then the parts they are built from.
/// `describe_all_types()` lists these after the document types.
pub fn describe_command_types() -> Vec<TypeDescription> {
    let mut descriptions = Vec::new();
    for row in COMMAND_TYPES {
        descriptions.push((row.describe_arguments)());
        descriptions.push((row.describe_result)());
    }
    for describe_part in COMMAND_PART_TYPES {
        descriptions.push(describe_part());
    }
    descriptions
}

fn find_command_types(command_id: &str) -> Option<&'static CommandTypes> {
    COMMAND_TYPES.iter().find(|row| row.command_id == command_id)
}

/// A command type's description: the area files build every one through this, so each gets its command's schema
/// id the same way.
pub(crate) fn command_type(name: &str, command_id: &str, doc: &str, fields: Vec<FieldDescription>) -> TypeDescription {
    TypeDescription {
        name: name.to_string(),
        schema_id: command_schema_id(command_id),
        doc: doc.to_string(),
        fields,
    }
}

/// The same field as a positional CLI argument (`rastai job get <job_id>`, layout 9.4).
pub(crate) fn positional(position: u32, description: FieldDescription) -> FieldDescription {
    FieldDescription {
        cli_position: Some(position),
        ..description
    }
}

/// The same field, written as `"<redacted>"` in the command journal, in the arguments or the result (contracts 5.1):
/// a credential passed or shown once.
pub(crate) fn secret(description: FieldDescription) -> FieldDescription {
    FieldDescription {
        secret: true,
        ..description
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::commands::token::{TokenCreateArgs, TokenListArgs};
    use crate::describe::types::{FieldKind, REQUIRED, field};

    /// (code, path) of each diagnostic, for compact asserts.
    fn codes_and_paths(diagnostics: &[Diagnostic]) -> Vec<(String, String)> {
        diagnostics
            .iter()
            .map(|item| (item.code.clone(), item.path.clone()))
            .collect()
    }

    fn refusal(command_id: &str, arguments_json: &str) -> Vec<(String, String)> {
        let diagnostics = parse_arguments::<Value>(command_id, arguments_json).unwrap_err();
        codes_and_paths(&diagnostics)
    }

    #[test]
    fn valid_arguments_become_their_struct() {
        let parsed: TokenCreateArgs = parse_arguments(
            "token.create",
            r#"{"name": "ci", "preset": "chat-only", "scopes": ["read"]}"#,
        )
        .unwrap();
        assert_eq!(parsed.name, "ci");
        assert_eq!(parsed.preset, "chat-only");
        assert_eq!(parsed.scopes, ["read"]);
    }

    #[test]
    fn empty_text_means_no_arguments() {
        let parsed: TokenListArgs = parse_arguments("token.list", "").unwrap();
        assert!(!parsed.include_revoked);
    }

    #[test]
    fn every_structure_problem_comes_back_with_its_pointer() {
        let found = refusal(
            "token.create",
            r#"{"preset": "everything", "scopes": [1], "colour": "red"}"#,
        );
        assert_eq!(
            found,
            [
                ("RRS-E1001".to_string(), "#/name".to_string()),
                ("RRS-E1004".to_string(), "#/preset".to_string()),
                ("RRS-E1002".to_string(), "#/scopes/0".to_string()),
                ("RRS-E1003".to_string(), "#/colour".to_string()),
            ]
        );
    }

    #[test]
    fn text_that_is_not_json_is_a_syntax_problem() {
        let found = refusal("token.create", r#"{"name": "a", "name": "b"}"#);
        assert_eq!(found, [("RRS-E0003".to_string(), "#/name".to_string())]);
        let found = refusal("token.list", "{");
        assert_eq!(found.len(), 1);
        assert!(found[0].0.starts_with("RRS-E0"), "{found:?}");
    }

    #[test]
    fn arguments_must_be_an_object() {
        for text in ["[]", "null", "\"name\"", "7"] {
            let found = refusal("token.list", text);
            assert_eq!(found, [("RRS-E1002".to_string(), String::new())], "{text}");
        }
    }

    #[test]
    fn a_command_without_a_description_takes_any_object() {
        let parsed: Value = parse_arguments("acme.tool.run", r#"{"anything": [1, 2]}"#).unwrap();
        assert_eq!(parsed["anything"][1], 2);
        assert_eq!(
            refusal("acme.tool.run", "[1]"),
            [("RRS-E1002".to_string(), String::new())]
        );
    }

    #[test]
    fn an_integer_outside_its_rust_type_is_refused_by_serde() {
        #[derive(Debug, serde::Deserialize)]
        struct Counted {
            #[allow(dead_code)]
            count: u32,
        }
        let refused = parse_arguments::<Counted>("acme.tool.run", r#"{"count": -1}"#).unwrap_err();
        assert_eq!(refused[0].code, "RRS-E1002");
        assert!(refused[0].message.contains("-1"), "{}", refused[0].message);
    }

    #[test]
    fn a_result_is_written_as_compact_json() {
        let result = crate::commands::token::TokenRevokeResult {
            token_id: "tok_a".to_string(),
            revoked_unix_ms: 5,
        };
        assert_eq!(result_json(&result), r#"{"token_id":"tok_a","revoked_unix_ms":5}"#);
    }

    #[test]
    fn a_command_schema_id_names_the_command() {
        assert_eq!(command_schema_id("model.load"), "rastai.command.model.load/1");
        assert_eq!(
            command_schema_id("token.login_code.create"),
            "rastai.command.token.login_code.create/1"
        );
    }

    #[test]
    fn known_ids_have_both_descriptions_and_unknown_ids_have_none() {
        assert_eq!(describe_arguments("job.get").unwrap().name, "JobGetArgs");
        assert_eq!(describe_result("job.get").unwrap().name, "JobGetResult");
        assert!(describe_arguments("psychology.step").is_none());
        assert!(describe_result("no.such").is_none());
    }

    #[test]
    fn every_command_type_is_listed_once() {
        let descriptions = describe_command_types();
        assert_eq!(descriptions.len(), COMMAND_TYPES.len() * 2 + COMMAND_PART_TYPES.len());
        for description in &descriptions {
            let count = descriptions
                .iter()
                .filter(|other| other.name == description.name)
                .count();
            assert_eq!(count, 1, "{}", description.name);
        }
    }

    #[test]
    fn the_flag_helpers_set_only_their_flag() {
        let plain = field("job_id", FieldKind::String, REQUIRED, "The job.");
        let first = positional(0, plain.clone());
        assert_eq!(first.cli_position, Some(0));
        assert!(!first.secret);
        let hidden = secret(plain);
        assert!(hidden.secret);
        assert_eq!(hidden.cli_position, None);
    }
}
