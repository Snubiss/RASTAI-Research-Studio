//! The three in-process twins against their proto messages (layout rust-control-plane.md 5.5): rastai-diag's
//! `Diagnostic` lists exactly the fields of `rastai.common.v1.Diagnostic`, `CommandRow` those of
//! `rastai.commands.v1.CommandDescriptor` minus the five registration fields, and `EventTypeRow` those of
//! `rastai.events.v1.EventTypeDescriptor` minus `plugin_id`. The field lists are read from the .proto files
//! themselves, so a field added there fails this test until the twin and wire.rs follow.
//!
//! The golden messages of fixtures/proto-golden/ then round-trip through the conversions. They are read from their
//! `.json` form: rastai-core doesn't depend on prost, and rastai-proto's own test already checks that the `.json`
//! and `.binpb` goldens are the same message.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_core::commands::descriptor::CommandRow;
use rastai_core::event_types::{EVENT_TYPES, EventTypeRow};
use rastai_core::scopes::Scope;
use rastai_core::wire::{descriptor_from_row, diagnostic_from_proto, diagnostic_to_proto};
use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind};
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, Severity as ProtoSeverity, Stability};
use serde_json::Value;

const DIAGNOSTIC_PROTO: &str = "proto/rastai/common/v1/diagnostic.proto";
const COMMANDS_PROTO: &str = "proto/rastai/commands/v1/command_service.proto";
const EVENTS_PROTO: &str = "proto/rastai/events/v1/event_service.proto";
const DIAGNOSTIC_GOLDEN: &str = "fixtures/proto-golden/common/Diagnostic.json";
const DESCRIPTOR_GOLDEN: &str = "fixtures/proto-golden/commands/CommandDescriptor.json";

/// What registration fills in, so a core row leaves them out (layout 5.2).
const REGISTRATION_FIELDS: &[&str] = &[
    "plugin_id",
    "arguments_type",
    "result_type",
    "arguments_schema_json",
    "result_schema_json",
];

/// What a plugin's manifest fills in for its own event types; every core event type leaves it empty.
const EVENT_REGISTRATION_FIELDS: &[&str] = &["plugin_id"];

/// The golden descriptor's values as a core row; registration fields aside, they are the golden's.
const GOLDEN_ROW: CommandRow = CommandRow {
    command_id: "psychology.replay.open",
    title: "Open replay",
    summary: "Recompute a conversation's full view from its token IDs.",
    section: "psychology",
    scope: Scope::Psychology,
    kind: CommandKind::Job,
    stability: Stability::Preview,
    undoable: true,
    basic: true,
    default_shortcut: "Mod+Shift+R",
    experimental: true,
    since_version: "0.1.0",
};

fn repository_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

fn read_json(relative: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(repository_file(relative)).unwrap()).unwrap()
}

/// The field names of `message <name> { ... }` in a .proto file, in order: the identifier before `=` on each
/// field line. Comments and nested blocks are skipped; these two messages have neither oneofs nor nesting.
fn proto_message_fields(relative: &str, message_name: &str) -> Vec<String> {
    let text = fs::read_to_string(repository_file(relative)).unwrap();
    let opening = format!("message {message_name} {{");
    let body_start = text
        .find(&opening)
        .unwrap_or_else(|| panic!("no {opening} in {relative}"))
        + opening.len();
    let body_end = body_start + text[body_start..].find('}').unwrap();
    let mut fields = Vec::new();
    for line in text[body_start..body_end].lines() {
        let code = line.split("//").next().unwrap().trim();
        if let Some((declaration, _number)) = code.split_once('=') {
            fields.push(declaration.split_whitespace().last().unwrap().to_string());
        }
    }
    fields
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// The twin's fields. The pattern has no `..`, so a field added to the struct fails to compile until it's listed.
fn diagnostic_twin_fields(diagnostic: &Diagnostic) -> Vec<&'static str> {
    let Diagnostic {
        code: _,
        severity: _,
        message: _,
        path: _,
        fix_text: _,
        fix_command_id: _,
        fix_arguments_json: _,
    } = diagnostic;
    vec![
        "code",
        "severity",
        "message",
        "path",
        "fix_text",
        "fix_command_id",
        "fix_arguments_json",
    ]
}

/// The row's fields, with the same full-pattern guard.
fn command_row_fields(row: &CommandRow) -> Vec<&'static str> {
    let CommandRow {
        command_id: _,
        title: _,
        summary: _,
        section: _,
        scope: _,
        kind: _,
        stability: _,
        undoable: _,
        basic: _,
        default_shortcut: _,
        experimental: _,
        since_version: _,
    } = row;
    vec![
        "command_id",
        "title",
        "summary",
        "section",
        "scope",
        "kind",
        "stability",
        "undoable",
        "basic",
        "default_shortcut",
        "experimental",
        "since_version",
    ]
}

/// The event row's fields, with the same full-pattern guard.
fn event_type_row_fields(row: &EventTypeRow) -> Vec<&'static str> {
    let EventTypeRow {
        event_type: _,
        event_class: _,
        payload_type: _,
        emitter: _,
        stability: _,
        since_version: _,
    } = row;
    vec![
        "event_type",
        "event_class",
        "payload_type",
        "emitter",
        "stability",
        "since_version",
    ]
}

fn text_field(golden: &Value, name: &str) -> String {
    golden[name]
        .as_str()
        .unwrap_or_else(|| panic!("{name} is not text"))
        .to_string()
}

fn flag_field(golden: &Value, name: &str) -> bool {
    golden[name].as_bool().unwrap_or_else(|| panic!("{name} is not a bool"))
}

/// The golden Diagnostic as the prost struct, from its protobuf JSON (enums by name).
fn golden_proto_diagnostic() -> ProtoDiagnostic {
    let golden = read_json(DIAGNOSTIC_GOLDEN);
    let severity = ProtoSeverity::from_str_name(&text_field(&golden, "severity")).unwrap();
    ProtoDiagnostic {
        code: text_field(&golden, "code"),
        severity: severity as i32,
        message: text_field(&golden, "message"),
        path: text_field(&golden, "path"),
        fix_text: text_field(&golden, "fix_text"),
        fix_command_id: text_field(&golden, "fix_command_id"),
        fix_arguments_json: text_field(&golden, "fix_arguments_json"),
    }
}

/// The golden CommandDescriptor as the prost struct, from its protobuf JSON.
fn golden_descriptor() -> CommandDescriptor {
    let golden = read_json(DESCRIPTOR_GOLDEN);
    CommandDescriptor {
        command_id: text_field(&golden, "command_id"),
        title: text_field(&golden, "title"),
        summary: text_field(&golden, "summary"),
        section: text_field(&golden, "section"),
        scope: text_field(&golden, "scope"),
        undoable: flag_field(&golden, "undoable"),
        kind: CommandKind::from_str_name(&text_field(&golden, "kind")).unwrap() as i32,
        stability: Stability::from_str_name(&text_field(&golden, "stability")).unwrap() as i32,
        arguments_type: text_field(&golden, "arguments_type"),
        result_type: text_field(&golden, "result_type"),
        arguments_schema_json: text_field(&golden, "arguments_schema_json"),
        result_schema_json: text_field(&golden, "result_schema_json"),
        default_shortcut: text_field(&golden, "default_shortcut"),
        basic: flag_field(&golden, "basic"),
        plugin_id: text_field(&golden, "plugin_id"),
        experimental: flag_field(&golden, "experimental"),
        since_version: text_field(&golden, "since_version"),
    }
}

/// The keys of a golden JSON object. Every golden sets every field, so these are the message's fields too.
fn golden_keys(relative: &str) -> Vec<String> {
    read_json(relative).as_object().unwrap().keys().cloned().collect()
}

#[test]
fn diagnostic_twin_lists_the_proto_fields() {
    let proto_fields = proto_message_fields(DIAGNOSTIC_PROTO, "Diagnostic");
    let twin = diagnostic_from_proto(&golden_proto_diagnostic());
    let twin_fields: Vec<String> = diagnostic_twin_fields(&twin).into_iter().map(String::from).collect();
    assert_eq!(twin_fields, proto_fields);
    // The golden reader above sets every field the proto has.
    assert_eq!(sorted(golden_keys(DIAGNOSTIC_GOLDEN)), sorted(proto_fields));
}

#[test]
fn command_row_lists_the_proto_fields_minus_registration() {
    let proto_fields = proto_message_fields(COMMANDS_PROTO, "CommandDescriptor");
    let mut row_fields: Vec<String> = command_row_fields(&GOLDEN_ROW).into_iter().map(String::from).collect();
    for registration_field in REGISTRATION_FIELDS {
        assert!(
            !row_fields.iter().any(|field| field == registration_field),
            "{registration_field}"
        );
        row_fields.push(registration_field.to_string());
    }
    assert_eq!(sorted(row_fields), sorted(proto_fields.clone()));
    assert_eq!(sorted(golden_keys(DESCRIPTOR_GOLDEN)), sorted(proto_fields));
}

#[test]
fn event_type_row_lists_the_proto_fields_minus_plugin_id() {
    let proto_fields = proto_message_fields(EVENTS_PROTO, "EventTypeDescriptor");
    let mut row_fields: Vec<String> = event_type_row_fields(&EVENT_TYPES[0])
        .into_iter()
        .map(String::from)
        .collect();
    for registration_field in EVENT_REGISTRATION_FIELDS {
        assert!(
            !row_fields.iter().any(|field| field == registration_field),
            "{registration_field}"
        );
        row_fields.push(registration_field.to_string());
    }
    assert_eq!(sorted(row_fields), sorted(proto_fields));
}

#[test]
fn golden_diagnostic_round_trips() {
    let golden = golden_proto_diagnostic();
    let twin = diagnostic_from_proto(&golden);
    assert_eq!(twin.severity, Severity::Error);
    assert_eq!(twin.code, golden.code);
    assert_eq!(twin.fix_arguments_json, golden.fix_arguments_json);
    assert_eq!(diagnostic_to_proto(&twin), golden);
}

#[test]
fn every_severity_round_trips() {
    let golden = golden_proto_diagnostic();
    for severity in [ProtoSeverity::Error, ProtoSeverity::Warning, ProtoSeverity::Info] {
        let message = ProtoDiagnostic {
            severity: severity as i32,
            ..golden.clone()
        };
        assert_eq!(
            diagnostic_to_proto(&diagnostic_from_proto(&message)),
            message,
            "{severity:?}"
        );
    }
}

#[test]
fn golden_row_becomes_the_golden_descriptor() {
    let expected = CommandDescriptor {
        plugin_id: String::new(),
        arguments_type: String::new(),
        result_type: String::new(),
        arguments_schema_json: String::new(),
        result_schema_json: String::new(),
        ..golden_descriptor()
    };
    assert_eq!(descriptor_from_row(&GOLDEN_ROW), expected);
}

#[test]
fn a_plugin_scope_is_written_as_its_name() {
    let row = CommandRow {
        scope: Scope::Plugin("acme".to_string()),
        ..GOLDEN_ROW
    };
    assert_eq!(descriptor_from_row(&row).scope, "plugin:acme");
}
