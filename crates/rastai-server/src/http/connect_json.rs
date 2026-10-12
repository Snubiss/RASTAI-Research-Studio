//! The hand-written Connect JSON of the six `CommandService` messages (layout: rust-control-plane.md 11.3):
//! `ExecuteRequest`, `ExecuteResponse`, `Diagnostic`, `ListCommandsRequest`, `ListCommandsResponse` and
//! `CommandDescriptor`, one encode and one decode function each. prost has no proto3 JSON mapping and spec §27 lists
//! no crate for one, so the mapping is written here, and `tests/connect_protocol.rs` holds it to protobuf-es, the
//! reference: what it writes byte for byte, what it accepts and what it refuses
//! (`fixtures/proto-golden/commands/connect_json/cases.json`).
//!
//! Writing is protobuf-es's `toJsonString` with `useProtoFieldName`:
//! - proto field names (`command_id`), the spelling of the goldens and of wire types inside documents (contracts 4);
//! - fields in field-number order, compact, with no spaces;
//! - a field at its default (empty text, false, enum 0, empty list) left out, as proto3 says;
//! - enums by name; a number this build has no name for stays a number, so nothing a newer peer sent is lost;
//! - strings escaped as `JSON.stringify` escapes them; serde_json escapes exactly the same characters.
//!
//! Reading is protobuf-es's `fromJsonString`. The body is untrusted, so it refuses rather than guesses:
//! - the JSON goes through rastai-formats' strict reader, so a key sent twice, a lone surrogate or NaN is refused
//!   with that reader's own code (`RRS-E0001`–`E0004`, and the `RRS-E5xxx` limits);
//! - a field may be named by its proto name or by its lowerCamelCase JSON name, which connect-es sends; both names
//!   for one field is `RRS-E0003`, so one body can't carry two values for a field;
//! - a key no field has is `RRS-E1003` (a typo must not silently become an empty field), a value of the wrong JSON
//!   kind `RRS-E1002`, an enum name the message doesn't have or a number outside 32 bits `RRS-E1004`;
//! - `null` is the field's default; an enum is its name or any 32-bit integer (proto3 enums are open).
//!
//! Every refusal's path is the JSON pointer of the value refused, such as `#/diagnostics/0/severity`; an unknown
//! key longer than `MAX_ECHOED_CHARACTERS` is cut in it, as in the message.

// A refusal is a `Diagnostic`, the contract type every caller renders; it is larger than clippy's default for an
// `Err`, and refusals are rare, so it is returned as it is rather than boxed.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_diag::path::{DiagnosticPath, format_path};
use rastai_formats::json::pointer::join_pointer;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::syntax::syntax_diagnostic;
use rastai_proto::commands::v1::{
    CommandDescriptor, CommandKind, ExecuteRequest, ExecuteResponse, ListCommandsRequest, ListCommandsResponse,
};
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, Severity, Stability};
use serde_json::{Number, Value};

const CODE_FIELD_GIVEN_TWICE: &str = "RRS-E0003";
const CODE_WRONG_KIND: &str = "RRS-E1002";
const CODE_UNKNOWN_FIELD: &str = "RRS-E1003";
const CODE_NOT_ALLOWED: &str = "RRS-E1004";

/// The JSON pointer of the whole body (RFC 6901: the empty string).
const WHOLE_BODY_POINTER: &str = "";
/// Untrusted text a refusal repeats back (an unknown key, an enum name) is cut to this many characters, in the
/// message and in the path alike, so a megabyte-long key doesn't come back in the diagnostic and every log line
/// it reaches.
const MAX_ECHOED_CHARACTERS: usize = 80;
const ECHO_CUT_MARK: &str = "…";

// What a refusal says was expected, in the words rastai-formats uses for the same kinds.
const EXPECTED_OBJECT: &str = "an object";
const EXPECTED_STRING: &str = "a string";
const EXPECTED_BOOLEAN: &str = "true or false";
const EXPECTED_LIST: &str = "a list";
const EXPECTED_ENUM_INTEGER: &str = "an integer";

// The proto names of each message's fields, in field-number order. A key that is none of these, in either spelling,
// is unknown.
const EXECUTE_REQUEST_FIELDS: &[&str] = &[
    "command_id",
    "arguments_json",
    "project_id",
    "idempotency_key",
    "dry_run",
    "acting_plugin_id",
];
const EXECUTE_RESPONSE_FIELDS: &[&str] = &["ok", "result_json", "job_id", "diagnostics", "journal_id"];
const DIAGNOSTIC_FIELDS: &[&str] = &[
    "code",
    "severity",
    "message",
    "path",
    "fix_text",
    "fix_command_id",
    "fix_arguments_json",
];
const LIST_COMMANDS_REQUEST_FIELDS: &[&str] = &["section"];
const LIST_COMMANDS_RESPONSE_FIELDS: &[&str] = &["commands"];
const COMMAND_DESCRIPTOR_FIELDS: &[&str] = &[
    "command_id",
    "title",
    "summary",
    "section",
    "scope",
    "undoable",
    "kind",
    "stability",
    "arguments_type",
    "result_type",
    "arguments_schema_json",
    "result_schema_json",
    "default_shortcut",
    "basic",
    "plugin_id",
    "experimental",
    "since_version",
];

// The full names of the messages and enums, as refusals name them.
const EXECUTE_REQUEST_NAME: &str = "rastai.commands.v1.ExecuteRequest";
const EXECUTE_RESPONSE_NAME: &str = "rastai.commands.v1.ExecuteResponse";
const DIAGNOSTIC_NAME: &str = "rastai.common.v1.Diagnostic";
const LIST_COMMANDS_REQUEST_NAME: &str = "rastai.commands.v1.ListCommandsRequest";
const LIST_COMMANDS_RESPONSE_NAME: &str = "rastai.commands.v1.ListCommandsResponse";
const COMMAND_DESCRIPTOR_NAME: &str = "rastai.commands.v1.CommandDescriptor";
const SEVERITY_NAME: &str = "rastai.common.v1.Severity";
const COMMAND_KIND_NAME: &str = "rastai.commands.v1.CommandKind";
const STABILITY_NAME: &str = "rastai.common.v1.Stability";

/// One member of a JSON object being written: the field's proto name and its value, already written as JSON text.
type JsonMember = (&'static str, String);

/// The members of one JSON object being read, each under the proto name of its field, and where the object is.
struct MessageFields<'a> {
    values: BTreeMap<&'static str, &'a Value>,
    pointer: String,
}

// ---- ExecuteRequest ----

pub fn execute_request_to_json(request: &ExecuteRequest) -> String {
    // Taken apart in full, so a field added to the .proto fails to compile here until it is written.
    let ExecuteRequest {
        command_id,
        arguments_json,
        project_id,
        idempotency_key,
        dry_run,
        acting_plugin_id,
    } = request;
    let mut members = Vec::new();
    push_string(&mut members, "command_id", command_id);
    push_string(&mut members, "arguments_json", arguments_json);
    push_string(&mut members, "project_id", project_id);
    push_string(&mut members, "idempotency_key", idempotency_key);
    push_bool(&mut members, "dry_run", *dry_run);
    push_string(&mut members, "acting_plugin_id", acting_plugin_id);
    object_text(&members)
}

pub fn execute_request_from_json(body: &[u8]) -> Result<ExecuteRequest, Diagnostic> {
    let value = read_strict_json(body)?;
    execute_request_from_value(&value, WHOLE_BODY_POINTER)
}

fn execute_request_from_value(value: &Value, pointer: &str) -> Result<ExecuteRequest, Diagnostic> {
    let fields = read_message_fields(value, EXECUTE_REQUEST_NAME, EXECUTE_REQUEST_FIELDS, pointer)?;
    Ok(ExecuteRequest {
        command_id: read_string(&fields, "command_id")?,
        arguments_json: read_string(&fields, "arguments_json")?,
        project_id: read_string(&fields, "project_id")?,
        idempotency_key: read_string(&fields, "idempotency_key")?,
        dry_run: read_bool(&fields, "dry_run")?,
        acting_plugin_id: read_string(&fields, "acting_plugin_id")?,
    })
}

// ---- ExecuteResponse ----

pub fn execute_response_to_json(response: &ExecuteResponse) -> String {
    let ExecuteResponse {
        ok,
        result_json,
        job_id,
        diagnostics,
        journal_id,
    } = response;
    let mut members = Vec::new();
    push_bool(&mut members, "ok", *ok);
    push_string(&mut members, "result_json", result_json);
    push_string(&mut members, "job_id", job_id);
    push_list(
        &mut members,
        "diagnostics",
        diagnostics.iter().map(diagnostic_to_json).collect(),
    );
    push_string(&mut members, "journal_id", journal_id);
    object_text(&members)
}

pub fn execute_response_from_json(body: &[u8]) -> Result<ExecuteResponse, Diagnostic> {
    let value = read_strict_json(body)?;
    let fields = read_message_fields(
        &value,
        EXECUTE_RESPONSE_NAME,
        EXECUTE_RESPONSE_FIELDS,
        WHOLE_BODY_POINTER,
    )?;
    Ok(ExecuteResponse {
        ok: read_bool(&fields, "ok")?,
        result_json: read_string(&fields, "result_json")?,
        job_id: read_string(&fields, "job_id")?,
        diagnostics: read_message_list(&fields, "diagnostics", diagnostic_from_value)?,
        journal_id: read_string(&fields, "journal_id")?,
    })
}

// ---- Diagnostic ----

pub fn diagnostic_to_json(diagnostic: &ProtoDiagnostic) -> String {
    let ProtoDiagnostic {
        code,
        severity,
        message,
        path,
        fix_text,
        fix_command_id,
        fix_arguments_json,
    } = diagnostic;
    let mut members = Vec::new();
    push_string(&mut members, "code", code);
    push_enum(&mut members, "severity", *severity, severity_name(*severity));
    push_string(&mut members, "message", message);
    push_string(&mut members, "path", path);
    push_string(&mut members, "fix_text", fix_text);
    push_string(&mut members, "fix_command_id", fix_command_id);
    push_string(&mut members, "fix_arguments_json", fix_arguments_json);
    object_text(&members)
}

pub fn diagnostic_from_json(body: &[u8]) -> Result<ProtoDiagnostic, Diagnostic> {
    let value = read_strict_json(body)?;
    diagnostic_from_value(&value, WHOLE_BODY_POINTER)
}

fn diagnostic_from_value(value: &Value, pointer: &str) -> Result<ProtoDiagnostic, Diagnostic> {
    let fields = read_message_fields(value, DIAGNOSTIC_NAME, DIAGNOSTIC_FIELDS, pointer)?;
    Ok(ProtoDiagnostic {
        code: read_string(&fields, "code")?,
        severity: read_enum(&fields, "severity", SEVERITY_NAME, severity_number)?,
        message: read_string(&fields, "message")?,
        path: read_string(&fields, "path")?,
        fix_text: read_string(&fields, "fix_text")?,
        fix_command_id: read_string(&fields, "fix_command_id")?,
        fix_arguments_json: read_string(&fields, "fix_arguments_json")?,
    })
}

// ---- ListCommandsRequest ----

pub fn list_commands_request_to_json(request: &ListCommandsRequest) -> String {
    let ListCommandsRequest { section } = request;
    let mut members = Vec::new();
    push_string(&mut members, "section", section);
    object_text(&members)
}

pub fn list_commands_request_from_json(body: &[u8]) -> Result<ListCommandsRequest, Diagnostic> {
    let value = read_strict_json(body)?;
    let fields = read_message_fields(
        &value,
        LIST_COMMANDS_REQUEST_NAME,
        LIST_COMMANDS_REQUEST_FIELDS,
        WHOLE_BODY_POINTER,
    )?;
    Ok(ListCommandsRequest {
        section: read_string(&fields, "section")?,
    })
}

// ---- ListCommandsResponse ----

pub fn list_commands_response_to_json(response: &ListCommandsResponse) -> String {
    let ListCommandsResponse { commands } = response;
    let mut members = Vec::new();
    push_list(
        &mut members,
        "commands",
        commands.iter().map(command_descriptor_to_json).collect(),
    );
    object_text(&members)
}

pub fn list_commands_response_from_json(body: &[u8]) -> Result<ListCommandsResponse, Diagnostic> {
    let value = read_strict_json(body)?;
    let fields = read_message_fields(
        &value,
        LIST_COMMANDS_RESPONSE_NAME,
        LIST_COMMANDS_RESPONSE_FIELDS,
        WHOLE_BODY_POINTER,
    )?;
    Ok(ListCommandsResponse {
        commands: read_message_list(&fields, "commands", command_descriptor_from_value)?,
    })
}

// ---- CommandDescriptor ----

pub fn command_descriptor_to_json(descriptor: &CommandDescriptor) -> String {
    let CommandDescriptor {
        command_id,
        title,
        summary,
        section,
        scope,
        undoable,
        kind,
        stability,
        arguments_type,
        result_type,
        arguments_schema_json,
        result_schema_json,
        default_shortcut,
        basic,
        plugin_id,
        experimental,
        since_version,
    } = descriptor;
    let mut members = Vec::new();
    push_string(&mut members, "command_id", command_id);
    push_string(&mut members, "title", title);
    push_string(&mut members, "summary", summary);
    push_string(&mut members, "section", section);
    push_string(&mut members, "scope", scope);
    push_bool(&mut members, "undoable", *undoable);
    push_enum(&mut members, "kind", *kind, command_kind_name(*kind));
    push_enum(&mut members, "stability", *stability, stability_name(*stability));
    push_string(&mut members, "arguments_type", arguments_type);
    push_string(&mut members, "result_type", result_type);
    push_string(&mut members, "arguments_schema_json", arguments_schema_json);
    push_string(&mut members, "result_schema_json", result_schema_json);
    push_string(&mut members, "default_shortcut", default_shortcut);
    push_bool(&mut members, "basic", *basic);
    push_string(&mut members, "plugin_id", plugin_id);
    push_bool(&mut members, "experimental", *experimental);
    push_string(&mut members, "since_version", since_version);
    object_text(&members)
}

pub fn command_descriptor_from_json(body: &[u8]) -> Result<CommandDescriptor, Diagnostic> {
    let value = read_strict_json(body)?;
    command_descriptor_from_value(&value, WHOLE_BODY_POINTER)
}

fn command_descriptor_from_value(value: &Value, pointer: &str) -> Result<CommandDescriptor, Diagnostic> {
    let fields = read_message_fields(value, COMMAND_DESCRIPTOR_NAME, COMMAND_DESCRIPTOR_FIELDS, pointer)?;
    Ok(CommandDescriptor {
        command_id: read_string(&fields, "command_id")?,
        title: read_string(&fields, "title")?,
        summary: read_string(&fields, "summary")?,
        section: read_string(&fields, "section")?,
        scope: read_string(&fields, "scope")?,
        undoable: read_bool(&fields, "undoable")?,
        kind: read_enum(&fields, "kind", COMMAND_KIND_NAME, command_kind_number)?,
        stability: read_enum(&fields, "stability", STABILITY_NAME, stability_number)?,
        arguments_type: read_string(&fields, "arguments_type")?,
        result_type: read_string(&fields, "result_type")?,
        arguments_schema_json: read_string(&fields, "arguments_schema_json")?,
        result_schema_json: read_string(&fields, "result_schema_json")?,
        default_shortcut: read_string(&fields, "default_shortcut")?,
        basic: read_bool(&fields, "basic")?,
        plugin_id: read_string(&fields, "plugin_id")?,
        experimental: read_bool(&fields, "experimental")?,
        since_version: read_string(&fields, "since_version")?,
    })
}

// ---- Enum names, from the generated code ----

fn severity_name(number: i32) -> Option<&'static str> {
    Severity::try_from(number).ok().map(|severity| severity.as_str_name())
}

fn severity_number(name: &str) -> Option<i32> {
    Severity::from_str_name(name).map(|severity| severity as i32)
}

fn command_kind_name(number: i32) -> Option<&'static str> {
    CommandKind::try_from(number).ok().map(|kind| kind.as_str_name())
}

fn command_kind_number(name: &str) -> Option<i32> {
    CommandKind::from_str_name(name).map(|kind| kind as i32)
}

fn stability_name(number: i32) -> Option<&'static str> {
    Stability::try_from(number)
        .ok()
        .map(|stability| stability.as_str_name())
}

fn stability_number(name: &str) -> Option<i32> {
    Stability::from_str_name(name).map(|stability| stability as i32)
}

// ---- Writing ----

fn push_string(members: &mut Vec<JsonMember>, name: &'static str, value: &str) {
    if !value.is_empty() {
        members.push((name, string_text(value)));
    }
}

fn push_bool(members: &mut Vec<JsonMember>, name: &'static str, value: bool) {
    if value {
        members.push((name, "true".to_string()));
    }
}

/// An enum by its name, or by its number when this build has no name for it (a newer peer's value).
fn push_enum(members: &mut Vec<JsonMember>, name: &'static str, number: i32, known_name: Option<&str>) {
    if number == 0 {
        return;
    }
    match known_name {
        Some(enum_name) => members.push((name, string_text(enum_name))),
        None => members.push((name, number.to_string())),
    }
}

/// A list of messages, each already written as a JSON object.
fn push_list(members: &mut Vec<JsonMember>, name: &'static str, items: Vec<String>) {
    if !items.is_empty() {
        members.push((name, format!("[{}]", items.join(","))));
    }
}

fn object_text(members: &[JsonMember]) -> String {
    let written: Vec<String> = members
        .iter()
        .map(|(name, value)| format!("{}:{value}", string_text(name)))
        .collect();
    format!("{{{}}}", written.join(","))
}

/// A string as JSON text, quoted and escaped.
fn string_text(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

// ---- Reading ----

/// The body as JSON, through the strict reader. A request body is no bigger than a document, so the document caps
/// apply; the HTTP endpoint limits the body's size before it gets here.
fn read_strict_json(body: &[u8]) -> Result<Value, Diagnostic> {
    match read_json(body, &ImportLimits::default()) {
        Ok(parsed) => Ok(parsed.value),
        Err(error) => Err(syntax_diagnostic(&error, "")),
    }
}

/// The members of a JSON object, each under the proto name of the field it sets. A key that names no field, and a
/// field named twice (once by each spelling), are refused here, before any value is read.
fn read_message_fields<'a>(
    value: &'a Value,
    message_name: &str,
    field_names: &[&'static str],
    pointer: &str,
) -> Result<MessageFields<'a>, Diagnostic> {
    let Value::Object(object) = value else {
        return Err(wrong_kind(message_name, EXPECTED_OBJECT, value, pointer));
    };
    let mut values = BTreeMap::new();
    for (key, member) in object {
        let Some(field_name) = field_for_key(key, field_names) else {
            // The key goes into the path too, so the path repeats it cut, like the message does.
            return Err(unknown_field(key, &join_pointer(pointer, &echoed(key))));
        };
        if values.insert(field_name, member).is_some() {
            return Err(field_given_twice(&join_pointer(pointer, field_name)));
        }
    }
    Ok(MessageFields {
        values,
        pointer: pointer.to_string(),
    })
}

/// The field a key names, by its proto name or by its JSON name.
fn field_for_key(key: &str, field_names: &[&'static str]) -> Option<&'static str> {
    field_names
        .iter()
        .copied()
        .find(|field_name| *field_name == key || json_name(field_name) == key)
}

/// The JSON name protoc gives a field: its proto name in lowerCamelCase (`fix_command_id` is `fixCommandId`).
fn json_name(proto_name: &str) -> String {
    let mut name = String::with_capacity(proto_name.len());
    let mut capitalize_next = false;
    for character in proto_name.chars() {
        if character == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            name.push(character.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            name.push(character);
        }
    }
    name
}

/// A field's value, or None when it is absent or `null`: both mean the field's default.
fn field_value<'a>(fields: &MessageFields<'a>, name: &str) -> Option<&'a Value> {
    fields.values.get(name).copied().filter(|value| !value.is_null())
}

fn read_string(fields: &MessageFields, name: &str) -> Result<String, Diagnostic> {
    match field_value(fields, name) {
        None => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Err(wrong_kind(
            name,
            EXPECTED_STRING,
            other,
            &join_pointer(&fields.pointer, name),
        )),
    }
}

fn read_bool(fields: &MessageFields, name: &str) -> Result<bool, Diagnostic> {
    match field_value(fields, name) {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(wrong_kind(
            name,
            EXPECTED_BOOLEAN,
            other,
            &join_pointer(&fields.pointer, name),
        )),
    }
}

/// An enum by its name, or by any 32-bit integer. A number without a name is kept: proto3 enums are open, and a
/// newer peer may send a value this build doesn't know.
fn read_enum(
    fields: &MessageFields,
    name: &str,
    enum_name: &str,
    number_of_name: fn(&str) -> Option<i32>,
) -> Result<i32, Diagnostic> {
    let pointer = join_pointer(&fields.pointer, name);
    let expected = format!("a {enum_name} name or a 32-bit integer");
    match field_value(fields, name) {
        None => Ok(0),
        Some(Value::String(text)) => match number_of_name(text) {
            Some(number) => Ok(number),
            None => Err(not_allowed(&format!("\"{}\"", echoed(text)), &expected, &pointer)),
        },
        Some(Value::Number(number)) => enum_number(number, name, &expected, &pointer),
        Some(other) => Err(wrong_kind(name, &expected, other, &pointer)),
    }
}

/// A number as an enum value. JSON doesn't tell `1` from `1.0`, so protobuf-es takes any number without a
/// fraction; one outside 32 bits names no enum value.
fn enum_number(number: &Number, name: &str, expected: &str, pointer: &str) -> Result<i32, Diagnostic> {
    if let Some(integer) = number.as_i64() {
        return i32::try_from(integer).map_err(|_| not_allowed(&integer.to_string(), expected, pointer));
    }
    if let Some(big_integer) = number.as_u64() {
        return Err(not_allowed(&big_integer.to_string(), expected, pointer));
    }
    let float = number.as_f64().unwrap_or(f64::NAN);
    if float.fract() != 0.0 || !float.is_finite() {
        return Err(diagnostic_at(
            CODE_WRONG_KIND,
            &[
                ("field", name),
                ("expected", EXPECTED_ENUM_INTEGER),
                ("found", "a number with a fraction"),
            ],
            &pointer_path(pointer),
        ));
    }
    if float < f64::from(i32::MIN) || float > f64::from(i32::MAX) {
        return Err(not_allowed(&float.to_string(), expected, pointer));
    }
    // In range and without a fraction, so the conversion is exact.
    Ok(float as i32)
}

/// A list of messages; `read_item` reads each one at its own pointer (`/diagnostics/0`).
fn read_message_list<T>(
    fields: &MessageFields,
    name: &str,
    read_item: fn(&Value, &str) -> Result<T, Diagnostic>,
) -> Result<Vec<T>, Diagnostic> {
    let pointer = join_pointer(&fields.pointer, name);
    let items = match field_value(fields, name) {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(other) => return Err(wrong_kind(name, EXPECTED_LIST, other, &pointer)),
    };
    let mut messages = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        messages.push(read_item(item, &join_pointer(&pointer, &index.to_string()))?);
    }
    Ok(messages)
}

// ---- Refusals ----

/// `RRS-E1002`: "<field> should be <expected>, but it is <found>."
fn wrong_kind(field: &str, expected: &str, value: &Value, pointer: &str) -> Diagnostic {
    diagnostic_at(
        CODE_WRONG_KIND,
        &[
            ("field", field),
            ("expected", expected),
            ("found", json_kind_name(value)),
        ],
        &pointer_path(pointer),
    )
}

/// `RRS-E1003`: "Unknown field <key>."
fn unknown_field(key: &str, pointer: &str) -> Diagnostic {
    diagnostic_at(CODE_UNKNOWN_FIELD, &[("field", &echoed(key))], &pointer_path(pointer))
}

/// `RRS-E0003`: the same field by its proto name and by its JSON name.
fn field_given_twice(pointer: &str) -> Diagnostic {
    diagnostic_at(CODE_FIELD_GIVEN_TWICE, &[], &pointer_path(pointer))
}

/// `RRS-E1004`: "<found> is not one of the allowed values; expected <expected>."
fn not_allowed(found: &str, expected: &str, pointer: &str) -> Diagnostic {
    diagnostic_at(
        CODE_NOT_ALLOWED,
        &[("found", found), ("expected", expected)],
        &pointer_path(pointer),
    )
}

/// The diagnostic path of a value in the body: its JSON pointer alone, since a body has no file name.
fn pointer_path(pointer: &str) -> String {
    format_path(&DiagnosticPath {
        json_pointer: pointer.to_string(),
        ..DiagnosticPath::default()
    })
}

/// How a JSON value's kind is named in a refusal, in rastai-formats' words.
fn json_kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "true or false",
        Value::Number(number) if number.is_f64() => "a number with a fraction or exponent",
        Value::Number(_) => "an integer",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Untrusted text as a refusal repeats it: cut after `MAX_ECHOED_CHARACTERS` characters.
fn echoed(text: &str) -> String {
    if text.chars().count() <= MAX_ECHOED_CHARACTERS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_ECHOED_CHARACTERS).collect();
    format!("{kept}{ECHO_CUT_MARK}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn json_names_are_lower_camel_case() {
        assert_eq!(json_name("command_id"), "commandId");
        assert_eq!(json_name("fix_arguments_json"), "fixArgumentsJson");
        assert_eq!(json_name("ok"), "ok");
    }

    #[test]
    fn a_refusal_cuts_long_untrusted_text() {
        let long_key = "k".repeat(10_000);
        let body = format!("{{\"{long_key}\":1}}");
        let refusal = execute_request_from_json(body.as_bytes()).unwrap_err();
        assert_eq!(refusal.code, "RRS-E1003");
        assert!(refusal.message.len() < 200, "{}", refusal.message);
        assert!(refusal.message.contains(ECHO_CUT_MARK));
        assert!(refusal.path.len() < 200, "{}", refusal.path);
        assert!(refusal.path.contains(ECHO_CUT_MARK));
    }

    #[test]
    fn a_short_unknown_key_comes_back_whole_in_the_path() {
        let refusal = execute_request_from_json(br#"{"spare":1}"#).unwrap_err();
        assert_eq!(refusal.code, "RRS-E1003");
        assert_eq!(refusal.path, "#/spare");
    }

    #[test]
    fn bytes_that_are_not_utf8_are_refused() {
        let refusal = list_commands_request_from_json(b"{\"section\":\"\xff\"}").unwrap_err();
        assert_eq!(refusal.code, "RRS-E0002");
    }

    #[test]
    fn a_refusal_names_the_field_and_the_kinds() {
        let refusal = execute_request_from_json(br#"{"dry_run":"yes"}"#).unwrap_err();
        assert_eq!(refusal.code, "RRS-E1002");
        assert_eq!(refusal.path, "#/dry_run");
        assert!(refusal.message.contains("dry_run"), "{}", refusal.message);
        assert!(refusal.message.contains("true or false"), "{}", refusal.message);
    }
}
