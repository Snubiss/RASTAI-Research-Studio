//! Every core command is described (layout rust-control-plane.md 4.11, 10.1): each row of rastai-core's `COMMANDS`
//! has an argument and a result description named after its id, in its command's schema id, and a schema file in
//! `schemas/commands/` that holds both. `COMMAND_TYPES` has no row the table lacks, so no types outlive their command.
//!
//! The schema files are generated (`just codegen`); this test reads the checked-in ones, so a row added without
//! running the codegen fails here, before `check-generated` runs.
//!
//! A credential a command returns once (a login code, a new token's text) is `secret`, so the command journal writes
//! it as `"<redacted>"` in `result_json` and an idempotent replay can't hand it out again (contracts 5.1). The flag is
//! one `secret(...)` wrapper in an area file, easy to lose in an edit, so the tests below lock it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_core::commands::table::COMMANDS;
use rastai_formats::commands::token::{describe_token_create_result, describe_token_login_code_create_result};
use rastai_formats::commands::{
    COMMAND_TYPES, command_schema_id, describe_arguments, describe_command_types, describe_result,
};
use rastai_formats::describe::types::TypeDescription;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde_json::Value;

const SCHEMAS_FOLDER: &str = "schemas";
const COMMANDS_FOLDER: &str = "commands";
const ARGUMENTS_SUFFIX: &str = "Args";
const RESULT_SUFFIX: &str = "Result";
const STABILITY_KEYWORD: &str = "x-rastai-stability";
/// Every core command is `preview` until Phase 2 (owner decision N3).
const PREVIEW: &str = "preview";
/// A field whose name has one of these words holds a credential, so it must be `secret`.
const CREDENTIAL_WORDS: &[&str] = &["token", "code", "secret", "password"];
/// Fields whose names have a credential word but hold no credential: `<type>.<field>`.
const NOT_CREDENTIALS: &[&str] = &[
    // An id names a token without being one; `token.revoke` takes it.
    "TokenCreateResult.token_id",
    "TokenSummary.token_id",
    "TokenRevokeArgs.token_id",
    "TokenRevokeResult.token_id",
    // A catalog code such as RRS-T1001, which the user is shown.
    "JobSummary.error_code",
];

fn repository_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

/// `token.login_code.create` -> `TokenLoginCodeCreate`: the `<Name>` of `<Name>Args` (layout 4.1).
fn type_name_stem(command_id: &str) -> String {
    let mut stem = String::new();
    for word in command_id.split(['.', '_']) {
        let mut characters = word.chars();
        if let Some(first) = characters.next() {
            stem.push(first.to_ascii_uppercase());
            stem.push_str(characters.as_str());
        }
    }
    stem
}

/// The checked-in schema file of a command, read with the strict reader.
fn read_command_schema(command_id: &str) -> Value {
    let relative = format!(
        "{SCHEMAS_FOLDER}/{COMMANDS_FOLDER}/{}.json",
        command_schema_id(command_id)
    );
    let path = repository_file(&relative);
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{relative}: {error}; run `just codegen`"));
    read_json(&bytes, &ImportLimits::default()).unwrap().value
}

#[test]
fn the_type_name_stem_is_the_camel_case_id() {
    assert_eq!(type_name_stem("system.status"), "SystemStatus");
    assert_eq!(type_name_stem("token.login_code.create"), "TokenLoginCodeCreate");
}

#[test]
fn every_row_has_its_arguments_and_result() {
    for row in COMMANDS {
        let command_id = row.command_id;
        let stem = type_name_stem(command_id);
        let arguments = describe_arguments(command_id).unwrap_or_else(|| panic!("{command_id}: no arguments"));
        let result = describe_result(command_id).unwrap_or_else(|| panic!("{command_id}: no result"));
        assert_eq!(arguments.name, format!("{stem}{ARGUMENTS_SUFFIX}"), "{command_id}");
        assert_eq!(result.name, format!("{stem}{RESULT_SUFFIX}"), "{command_id}");
        assert_eq!(arguments.schema_id, command_schema_id(command_id), "{command_id}");
        assert_eq!(result.schema_id, command_schema_id(command_id), "{command_id}");
    }
}

#[test]
fn every_row_has_a_schema_file_with_both_types() {
    for row in COMMANDS {
        let command_id = row.command_id;
        let schema = read_command_schema(command_id);
        assert_eq!(schema["title"], command_id);
        assert_eq!(schema[STABILITY_KEYWORD], PREVIEW, "{command_id}");
        let arguments = describe_arguments(command_id).unwrap();
        let result = describe_result(command_id).unwrap();
        assert_eq!(
            schema["$defs"]["arguments"]["title"],
            arguments.name.as_str(),
            "{command_id}"
        );
        assert_eq!(schema["$defs"]["result"]["title"], result.name.as_str(), "{command_id}");
    }
}

#[test]
fn the_type_table_has_exactly_the_command_rows() {
    let table_ids: Vec<&str> = COMMANDS.iter().map(|row| row.command_id).collect();
    let typed_ids: Vec<&str> = COMMAND_TYPES.iter().map(|row| row.command_id).collect();
    assert_eq!(typed_ids, table_ids);
}

/// Whether a described type's field is `secret`; the test fails when the type has no such field.
fn field_is_secret(description: &TypeDescription, field_name: &str) -> bool {
    let found = description.fields.iter().find(|field| field.name == field_name);
    let field = found.unwrap_or_else(|| panic!("{}: no field {field_name}", description.name));
    field.secret
}

/// `token_id` has the words `token` and `id`.
fn has_credential_word(field_name: &str) -> bool {
    field_name.split('_').any(|word| CREDENTIAL_WORDS.contains(&word))
}

#[test]
fn shown_once_credentials_are_secret() {
    assert!(field_is_secret(&describe_token_create_result(), "token"));
    assert!(field_is_secret(&describe_token_login_code_create_result(), "code"));
}

#[test]
fn every_credential_named_field_is_secret() {
    for description in describe_command_types() {
        for field in &description.fields {
            let qualified_name = format!("{}.{}", description.name, field.name);
            if !has_credential_word(&field.name) || NOT_CREDENTIALS.contains(&qualified_name.as_str()) {
                continue;
            }
            assert!(
                field.secret,
                "{qualified_name} looks like a credential but isn't secret"
            );
        }
    }
}

#[test]
fn every_listed_non_credential_exists() {
    let mut qualified_names = Vec::new();
    for description in describe_command_types() {
        for field in &description.fields {
            qualified_names.push(format!("{}.{}", description.name, field.name));
        }
    }
    for listed in NOT_CREDENTIALS {
        assert!(
            qualified_names.iter().any(|name| name == listed),
            "{listed}: no such field"
        );
    }
}
