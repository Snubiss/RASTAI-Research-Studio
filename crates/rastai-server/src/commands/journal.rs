//! The command journal (flows/platform.md 3.1, contracts 5.1 and 8.2): the started and finished rows of every command
//! that isn't `read`-scope, secret redaction, and the idempotency lookup.
//!
//! - **Redaction.** A field marked `secret` or `content` in the description of a command's arguments or result is
//!   written as `"<redacted>"`, in `arguments_json` and `result_json` alike, at any depth (inside a listed type, a
//!   list or a map). Text the daemon can't judge (a command with no description, text that isn't JSON) is redacted
//!   whole: a credential must never reach `rastai.db` because its field wasn't recognised.
//! - **Idempotency.** The same key from the same principal within `IDEMPOTENCY_KEY_LIFETIME_HOURS` finds the earlier
//!   row; `execute.rs` decides between replaying it and `RRS-C0004`. Its stored result is the redacted one, so a
//!   replayed `token.create` answers `<redacted>` and makes no second token. A per-key lock makes a duplicate that
//!   arrives while the first is still running wait for it.
//! - **Comparing requests.** The redacted arguments can't tell two requests apart that differ only in a secret, so a
//!   row with a key also stores `arguments_hash`: BLAKE3 of the canonical, unredacted arguments, keyed with the owner
//!   token's hash. The key never reaches disk, so a reader of `rastai.db` can't test guesses of a secret against the
//!   hash, and the same owner token gives the same hash after a restart.

use std::sync::{Arc, Mutex, MutexGuard};

use rastai_core::commands::output::CommandOutput;
use rastai_core::ids::{IdPrefix, new_id};
use rastai_core::time::now_unix_ms;
use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_formats::commands::{describe_arguments, describe_result};
use rastai_formats::describe::registry::{describe_all_types, find_description};
use rastai_formats::describe::types::{FieldKind, TypeDescription};
use rastai_formats::hashing::hash_bytes;
use rastai_formats::json::canonical::canonical_json;
use rastai_proto::commands::v1::{CommandDescriptor, ExecuteRequest};
use rastai_store::index::journal::{
    JournalFinishedRow, JournalRow, JournalStartedRow, find_journal_by_idempotency_key,
};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use serde_json::Value;

use crate::commands::context::{core_failure, store_failure};
use crate::server_state::Server;

/// What a redacted field holds in the journal (contracts 5.1).
pub(crate) const REDACTED: &str = "<redacted>";
/// How long a key is honoured (contracts 15.10); after that the same key starts a new command.
pub(crate) const IDEMPOTENCY_KEY_LIFETIME_HOURS: i64 = 24;
const MILLISECONDS_PER_HOUR: i64 = 3_600_000;
/// Commands with this scope only read, so they get no journal rows (flows/platform.md 3.1).
const READ_SCOPE: &str = "read";
/// The journal's form of "no arguments" (an `ExecuteRequest` that left `arguments_json` out).
const NO_ARGUMENTS: &str = "{}";
/// Separates the principal from the key in a lock's name; neither a principal label nor a key reaches the lock
/// with a newline in it unchanged, and two different pairs never join to the same name.
const LOCK_NAME_SEPARATOR: char = '\n';
/// Goes first in what `arguments_hash` hashes, so the owner token's hash, used as the key, gives values no other use
/// of it gives.
const ARGUMENTS_HASH_PURPOSE: &str = "rastai command_journal.arguments_hash/1";

/// The lock of one (principal, idempotency key) pair while a command with that key runs.
pub struct IdempotencyKeyLock {
    pub lock_name: String,
    pub lock: Arc<tokio::sync::Mutex<()>>,
}

/// True for every command that changes something: all but `read`-scope ones (flows/platform.md 3.1).
pub(crate) fn is_journaled(command: &CommandDescriptor) -> bool {
    command.scope != READ_SCOPE
}

/// The arguments as the journal stores them: `secret` and `content` fields redacted.
pub(crate) fn redact_arguments(command_id: &str, arguments_json: &str) -> String {
    let text = if arguments_json.is_empty() {
        NO_ARGUMENTS
    } else {
        arguments_json
    };
    redact_json(text, describe_arguments(command_id))
}

/// The result as the journal stores it and an idempotent replay returns it: `secret` and `content` fields
/// redacted. A command with no result (a job's) stays empty.
pub(crate) fn redact_result(command_id: &str, result_json: &str) -> String {
    if result_json.is_empty() {
        return String::new();
    }
    redact_json(result_json, describe_result(command_id))
}

/// Redacts the fields `description` marks, at any depth. With no description, or text that isn't JSON, the whole
/// value is redacted.
pub(crate) fn redact_json(text: &str, description: Option<TypeDescription>) -> String {
    let Some(description) = description else {
        return whole_value_redacted();
    };
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return whole_value_redacted();
    };
    let all_types = describe_all_types();
    redact_object(&mut value, &description, &all_types);
    value.to_string()
}

/// The lock for a principal's key. Locks nobody holds or waits for any more are dropped on the way, so the list
/// stays as short as the commands running with a key.
pub(crate) fn idempotency_key_lock(server: &Server, principal_label: &str, key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let lock_name = format!("{principal_label}{LOCK_NAME_SEPARATOR}{key}");
    let mut locks = lock_list(&server.idempotency_locks);
    // The list itself holds one reference; any other means a command holds the lock or waits for it.
    locks.retain(|entry| Arc::strong_count(&entry.lock) > 1);
    if let Some(entry) = locks.iter().find(|entry| entry.lock_name == lock_name) {
        return entry.lock.clone();
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.push(IdempotencyKeyLock {
        lock_name,
        lock: lock.clone(),
    });
    lock
}

/// The newest row of this principal's key, if it started within the key's lifetime.
pub(crate) fn find_recent_journal_row(
    server: &Server,
    principal_label: &str,
    key: &str,
) -> Result<Option<JournalRow>, Vec<Diagnostic>> {
    // A short indexed read on a pooled connection; it never waits on the writer (WAL).
    let reader = open_index_reader(&server.store.index_readers).map_err(|error| store_failure(&error))?;
    let row = find_journal_by_idempotency_key(&reader, principal_label, key).map_err(|error| store_failure(&error))?;
    let oldest_kept_unix_ms = now_unix_ms() - IDEMPOTENCY_KEY_LIFETIME_HOURS * MILLISECONDS_PER_HOUR;
    Ok(row.filter(|row| row.started_unix_ms > oldest_kept_unix_ms))
}

/// What the idempotency check compares: BLAKE3 of the canonical, unredacted arguments, keyed with `hash_key` (the
/// owner token's hash). Canonical, so key order and spacing don't make two requests different; unredacted, so two
/// secrets do. Only the hash is stored, never the arguments it was made from.
pub fn arguments_hash(hash_key: &str, arguments_json: &str) -> String {
    let text = if arguments_json.is_empty() {
        NO_ARGUMENTS
    } else {
        arguments_json
    };
    let canonical = match serde_json::from_str::<Value>(text) {
        Ok(value) => canonical_json(&value),
        // Layer 2 already read the arguments as JSON, so this is only a fallback: the text as it was sent.
        Err(_) => text.to_string(),
    };
    let keyed = format!("{ARGUMENTS_HASH_PURPOSE}\n{hash_key}\n{canonical}");
    hash_bytes(keyed.as_bytes())
}

/// True when a request carries the same command and arguments as an earlier row: the same command id and the same
/// `arguments_hash`. A row without a hash (one written before the column existed) is never the same request, so its
/// key is refused with `RRS-C0004` rather than replayed for arguments nobody can compare.
pub(crate) fn is_same_request(command_id: &str, sent_arguments_hash: &str, row: &JournalRow) -> bool {
    row.command_id == command_id && !row.arguments_hash.is_empty() && row.arguments_hash == sent_arguments_hash
}

/// Writes the started row and returns its journal id.
pub(crate) async fn journal_started(
    server: &Server,
    principal_label: &str,
    request: &ExecuteRequest,
) -> Result<String, Vec<Diagnostic>> {
    let journal_id = new_id(IdPrefix::Journal).map_err(|error| core_failure(&error))?;
    let row = JournalStartedRow {
        journal_id: journal_id.clone(),
        command_id: request.command_id.clone(),
        principal: principal_label.to_string(),
        arguments_json: redact_arguments(&request.command_id, &request.arguments_json),
        idempotency_key: request.idempotency_key.clone(),
        started_unix_ms: now_unix_ms(),
        arguments_hash: started_arguments_hash(server, request),
    };
    write_index(&server.store.index, IndexWrite::JournalStarted(row))
        .await
        .map_err(|error| store_failure(&error))?;
    Ok(journal_id)
}

/// Only a row with a key is ever compared, so only such a row keeps a hash.
fn started_arguments_hash(server: &Server, request: &ExecuteRequest) -> String {
    if request.idempotency_key.is_empty() {
        return String::new();
    }
    arguments_hash(&server.auth.owner_token_hash, &request.arguments_json)
}

/// Writes the finished row: ok, the first error's code, and the redacted result.
pub(crate) async fn journal_finished(
    server: &Server,
    command_id: &str,
    journal_id: &str,
    result: &Result<CommandOutput, Vec<Diagnostic>>,
) -> Result<(), Vec<Diagnostic>> {
    let row = match result {
        Ok(output) => JournalFinishedRow {
            journal_id: journal_id.to_string(),
            ok: true,
            code: String::new(),
            result_json: redact_result(command_id, &output.result_json),
            job_id: output.job_id.clone(),
            finished_unix_ms: now_unix_ms(),
        },
        Err(diagnostics) => JournalFinishedRow {
            journal_id: journal_id.to_string(),
            ok: false,
            code: first_error_code(diagnostics),
            result_json: String::new(),
            job_id: String::new(),
            finished_unix_ms: now_unix_ms(),
        },
    };
    write_index(&server.store.index, IndexWrite::JournalFinished(row))
        .await
        .map_err(|error| store_failure(&error))
}

/// The code `command_journal.code` and `command.finished` record: the first error's, else the first diagnostic's.
pub(crate) fn first_error_code(diagnostics: &[Diagnostic]) -> String {
    let first_error = diagnostics.iter().find(|item| item.severity == Severity::Error);
    match first_error.or(diagnostics.first()) {
        Some(item) => item.code.clone(),
        None => String::new(),
    }
}

fn whole_value_redacted() -> String {
    Value::String(REDACTED.to_string()).to_string()
}

fn redact_object(value: &mut Value, description: &TypeDescription, all_types: &[TypeDescription]) {
    let Value::Object(members) = value else {
        return;
    };
    for field in &description.fields {
        let Some(member) = members.get_mut(&field.name) else {
            continue;
        };
        if field.secret || field.content {
            *member = Value::String(REDACTED.to_string());
        } else {
            redact_kind(member, &field.kind, all_types);
        }
    }
}

/// Walks into the kinds that can hold marked fields: a described type, and lists and maps of one.
fn redact_kind(value: &mut Value, kind: &FieldKind, all_types: &[TypeDescription]) {
    match kind {
        FieldKind::Type(name) => {
            if let Some(description) = find_description(all_types, name) {
                redact_object(value, description, all_types);
            }
        }
        FieldKind::List(item_kind) => {
            if let Value::Array(items) = value {
                for item in items {
                    redact_kind(item, item_kind, all_types);
                }
            }
        }
        FieldKind::Map(item_kind) => {
            if let Value::Object(members) = value {
                for member in members.values_mut() {
                    redact_kind(member, item_kind, all_types);
                }
            }
        }
        _ => {}
    }
}

/// A command that panicked while holding the list leaves it poisoned; the list is still whole (each change is one
/// push or retain), so commands carry on.
fn lock_list<Item>(list: &Mutex<Vec<Item>>) -> MutexGuard<'_, Vec<Item>> {
    match list.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::diagnostic;
    use rastai_formats::describe::types::{FieldDescription, OPTIONAL, field, list_of, type_description};
    use serde_json::json;

    fn redacted(value: &Value) -> bool {
        value == &json!(REDACTED)
    }

    /// A described type with a secret, a content field, a plain field, and a list and a map of registered types that
    /// hold secrets of their own.
    fn sample_description() -> TypeDescription {
        let fields = vec![
            FieldDescription {
                secret: true,
                ..field("password", FieldKind::String, OPTIONAL, "A credential.")
            },
            FieldDescription {
                content: true,
                ..field("text", FieldKind::String, OPTIONAL, "What the user wrote.")
            },
            field("name", FieldKind::String, OPTIONAL, "Plain."),
            field(
                "created",
                list_of(FieldKind::Type("TokenCreateResult".to_string())),
                OPTIONAL,
                "Nested in a list.",
            ),
            field(
                "codes",
                FieldKind::Map(Box::new(FieldKind::Type("TokenLoginCodeCreateResult".to_string()))),
                OPTIONAL,
                "Nested in a map.",
            ),
        ];
        type_description("SampleArgs", "rastai.command.sample/1", "A sample.", fields)
    }

    #[test]
    fn secret_and_content_fields_are_redacted_and_the_rest_kept() {
        let text = json!({"password": "hunter2", "text": "my diary", "name": "visible"}).to_string();
        let value: Value = serde_json::from_str(&redact_json(&text, Some(sample_description()))).unwrap();
        assert!(redacted(&value["password"]));
        assert!(redacted(&value["text"]));
        assert_eq!(value["name"], "visible");
    }

    #[test]
    fn marked_fields_inside_lists_and_maps_of_described_types_are_redacted() {
        let text = json!({
            "created": [{"token_id": "tok_a", "name": "n", "scopes": [], "token": "first-secret"}],
            "codes": {"browser": {"code": "second-secret", "expires_unix_ms": 1, "http_port": 47820}},
        });
        let stored = redact_json(&text.to_string(), Some(sample_description()));
        assert!(
            !stored.contains("first-secret") && !stored.contains("second-secret"),
            "{stored}"
        );
        let value: Value = serde_json::from_str(&stored).unwrap();
        assert!(redacted(&value["created"][0]["token"]));
        assert_eq!(value["created"][0]["token_id"], "tok_a");
        assert!(redacted(&value["codes"]["browser"]["code"]));
        assert_eq!(value["codes"]["browser"]["http_port"], 47820);
    }

    #[test]
    fn the_token_create_result_never_keeps_the_token() {
        let result = json!({"token_id": "tok_a", "name": "ci", "scopes": ["read"], "token": "abc123"}).to_string();
        let stored = redact_result("token.create", &result);
        assert!(!stored.contains("abc123"), "{stored}");
        let value: Value = serde_json::from_str(&stored).unwrap();
        assert!(redacted(&value["token"]));
        assert_eq!(value["token_id"], "tok_a");
        let code = json!({"code": "c0de", "expires_unix_ms": 1, "http_port": 47820}).to_string();
        assert!(!redact_result("token.login_code.create", &code).contains("c0de"));
    }

    #[test]
    fn what_cannot_be_judged_is_redacted_whole() {
        assert_eq!(redact_json(r#"{"x": 1}"#, None), "\"<redacted>\"");
        assert_eq!(redact_json("not json", Some(sample_description())), "\"<redacted>\"");
        assert_eq!(
            redact_arguments("acme.tool.run", r#"{"key": "s3cret"}"#),
            "\"<redacted>\""
        );
    }

    #[test]
    fn empty_arguments_are_an_empty_object_and_empty_results_stay_empty() {
        assert_eq!(redact_arguments("system.status", ""), "{}");
        assert_eq!(redact_result("train.start", ""), "");
    }

    const HASH_KEY: &str = "blake3:0a1b";

    fn keyed_row(command_id: &str, arguments_json: &str) -> JournalRow {
        JournalRow {
            journal_id: "jrn_a".to_string(),
            command_id: command_id.to_string(),
            principal: "owner".to_string(),
            arguments_json: redact_json(arguments_json, Some(sample_description())),
            ok: Some(true),
            code: String::new(),
            idempotency_key: "k".to_string(),
            result_json: String::new(),
            job_id: String::new(),
            started_unix_ms: 1,
            finished_unix_ms: Some(2),
            arguments_hash: arguments_hash(HASH_KEY, arguments_json),
        }
    }

    #[test]
    fn requests_compare_by_the_hash_of_their_canonical_arguments() {
        let row = keyed_row("token.create", r#"{"name":"ci","scopes":["read"]}"#);
        let reordered = arguments_hash(HASH_KEY, r#"{ "scopes": ["read"], "name": "ci" }"#);
        assert!(is_same_request("token.create", &reordered, &row));
        let other = arguments_hash(HASH_KEY, r#"{"name":"other"}"#);
        assert!(!is_same_request("token.create", &other, &row));
        assert!(!is_same_request("token.revoke", &reordered, &row));
        // No arguments and an empty object are the same request.
        assert_eq!(arguments_hash(HASH_KEY, ""), arguments_hash(HASH_KEY, "{}"));
    }

    #[test]
    fn two_secrets_that_redact_alike_are_different_requests() {
        let first = r#"{"password":"hunter2","name":"n"}"#;
        let second = r#"{"password":"hunter3","name":"n"}"#;
        let row = keyed_row("archive.purge", first);
        // The journal can't tell them apart: both redact to the same text.
        assert_eq!(row.arguments_json, redact_json(second, Some(sample_description())));
        assert!(!is_same_request(
            "archive.purge",
            &arguments_hash(HASH_KEY, second),
            &row
        ));
        assert!(is_same_request("archive.purge", &arguments_hash(HASH_KEY, first), &row));
        assert!(!row.arguments_hash.contains("hunter2"));
    }

    #[test]
    fn the_hash_depends_on_its_key_and_a_row_without_one_never_matches() {
        let arguments = r#"{"name":"ci"}"#;
        assert_ne!(
            arguments_hash(HASH_KEY, arguments),
            arguments_hash("blake3:ffff", arguments)
        );
        let unhashed = JournalRow {
            arguments_hash: String::new(),
            ..keyed_row("token.create", arguments)
        };
        assert!(!is_same_request("token.create", "", &unhashed));
        assert!(!is_same_request(
            "token.create",
            &arguments_hash(HASH_KEY, arguments),
            &unhashed
        ));
    }

    #[test]
    fn only_read_commands_go_unjournaled() {
        let read = CommandDescriptor {
            scope: "read".to_string(),
            ..CommandDescriptor::default()
        };
        let admin = CommandDescriptor {
            scope: "system.admin".to_string(),
            ..CommandDescriptor::default()
        };
        assert!(!is_journaled(&read));
        assert!(is_journaled(&admin));
    }

    #[test]
    fn the_first_error_code_is_recorded() {
        let warning = diagnostic("RRS-C0006", &[]);
        let error = diagnostic("RRS-C0002", &[]);
        assert_eq!(first_error_code(&[warning.clone(), error]), "RRS-C0002");
        assert_eq!(first_error_code(&[warning]), "RRS-C0006");
        assert_eq!(first_error_code(&[]), "");
    }
}
