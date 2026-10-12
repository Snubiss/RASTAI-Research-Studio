//! The L0 floor of ADR-0017: every registered command, run with `dry_run`, answers ok or with diagnostics that all
//! carry a catalog code; never a crash, never an un-coded error. Each command runs three ways: with no arguments,
//! with the smallest arguments its description allows (so the handler or stub itself is reached), and as a principal
//! that holds no scope (so the refusal is coded too).

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use rastai_core::principal::owner_principal;
use rastai_diag::catalog::find_entry;
use rastai_formats::commands::describe_arguments;
use rastai_formats::describe::types::FieldKind;
use rastai_proto::commands::v1::{ExecuteRequest, ExecuteResponse};
use serde_json::{Map, Value, json};

use common::{
    execute_request, remove_test_folder, run_command, start_test_daemon, stop_test_daemon, test_paths, token_principal,
};

/// The required fields of a command's arguments, each with the plainest value of its kind.
fn smallest_arguments(command_id: &str) -> String {
    let Some(description) = describe_arguments(command_id) else {
        return "{}".to_string();
    };
    let mut members = Map::new();
    for field in description.fields.iter().filter(|field| field.required) {
        members.insert(field.name.clone(), plainest_value(&field.kind));
    }
    Value::Object(members).to_string()
}

fn plainest_value(kind: &FieldKind) -> Value {
    match kind {
        FieldKind::String => json!("x"),
        FieldKind::Integer | FieldKind::Number => json!(0),
        FieldKind::Boolean => json!(false),
        FieldKind::Json => Value::Null,
        FieldKind::Enum(values) => json!(values.first().cloned().unwrap_or_default()),
        FieldKind::List(_) => json!([]),
        FieldKind::Map(_) | FieldKind::Type(_) | FieldKind::Proto(_) => json!({}),
    }
}

fn dry_run(command_id: &str, arguments_json: &str) -> ExecuteRequest {
    ExecuteRequest {
        dry_run: true,
        ..execute_request(command_id, arguments_json)
    }
}

/// Ok, or every diagnostic has a code the catalog knows.
fn assert_ok_or_coded(response: &ExecuteResponse, what: &str) {
    if response.ok {
        return;
    }
    assert!(!response.diagnostics.is_empty(), "{what}: failed with no diagnostic");
    for item in &response.diagnostics {
        assert!(find_entry(&item.code).is_some(), "{what}: un-coded diagnostic {item:?}");
        assert!(!item.message.is_empty(), "{what}: {item:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_registered_command_answers_ok_or_a_catalog_code() {
    let paths = test_paths("l0floor");
    let running = start_test_daemon(&paths).await;
    let command_ids: Vec<String> = running
        .server
        .registry
        .read()
        .unwrap()
        .commands
        .iter()
        .map(|command| command.command_id.clone())
        .collect();
    assert!(!command_ids.is_empty());
    let no_scope = token_principal(Vec::new());
    for command_id in &command_ids {
        let empty = run_command(&running.server, &owner_principal(), dry_run(command_id, "{}")).await;
        assert_ok_or_coded(&empty, &format!("{command_id} with no arguments"));
        let smallest = smallest_arguments(command_id);
        let filled = run_command(&running.server, &owner_principal(), dry_run(command_id, &smallest)).await;
        assert_ok_or_coded(&filled, &format!("{command_id} with {smallest}"));
        let refused = run_command(&running.server, &no_scope, dry_run(command_id, &smallest)).await;
        assert!(!refused.ok, "{command_id} ran for a principal with no scope");
        assert_ok_or_coded(&refused, &format!("{command_id} with no scope"));
        // A dry run is never journaled.
        assert!(filled.journal_id.is_empty(), "{command_id}");
    }
    // The dry runs left the daemon running.
    assert!(!*running.server.shutdown.borrow());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}
