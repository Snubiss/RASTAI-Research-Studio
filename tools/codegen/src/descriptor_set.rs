//! The payload fields of each event type, read from the descriptor set `buf build` writes for proto/ (layout
//! rust-control-plane.md 4.9, question 17).
//!
//! `schemas/events/<event_type>.json` lists its payload's fields so `check-schema-diff` can see a field go away
//! without reading any .proto file. buf already parses proto/ for `just codegen` and `just proto-check`, so this
//! file asks buf for the result instead of parsing .proto text itself. buf writes the descriptor set in the
//! protobuf JSON mapping (`--as-file-descriptor-set`, format `json`): plain JSON that serde_json reads, so there is
//! no wire decoder here. The input is our own build output, never a user's file.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::error::{CodegenError, codegen_error};

/// The buf that `just codegen` pins; the justfile refuses any other version before this runs.
const BUF_PROGRAM: &str = "buf";
/// A plain FileDescriptorSet (not buf's image wrapper), without comments and source positions, as JSON on stdout.
const BUF_BUILD_ARGUMENTS: &[&str] = &[
    "build",
    "--as-file-descriptor-set",
    "--exclude-source-info",
    "--output",
    "-#format=json",
];
/// buf reads its module from here; an error running it is reported against this file.
const BUF_CONFIG_FILE: &str = "buf.yaml";
/// Where a payload message is named; a message the descriptor set lacks is fixed there or in proto/.
const EVENT_TYPES_FILE: &str = "crates/rastai-core/src/event_types.rs";

// Words of the protobuf JSON mapping of descriptor.proto.
const MESSAGE_TYPE_TYPE: &str = "TYPE_MESSAGE";
const ENUM_TYPE: &str = "TYPE_ENUM";
const SCALAR_TYPE_PREFIX: &str = "TYPE_";
const REPEATED_LABEL: &str = "LABEL_REPEATED";
// A map field is a repeated message whose nested entry type has these two fields (descriptor.proto MessageOptions).
const MAP_KEY_FIELD: &str = "key";
const MAP_VALUE_FIELD: &str = "value";

/// One field of a payload message, as the event file writes it (contracts 12.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadField {
    pub name: String,
    pub number: u64,
    /// A scalar as .proto writes it (`string`, `uint64`), a message or enum by its full name
    /// (`rastai.common.v1.Diagnostic`), or `map<string, double>`.
    pub type_name: String,
    pub repeated: bool,
    /// Declared `optional` (proto3 presence), so 0 and unset differ.
    pub optional: bool,
}

/// Runs `buf build` in `repo_root` and returns the descriptor set of every file in proto/.
pub fn build_descriptor_set(repo_root: &Path) -> Result<Value, CodegenError> {
    let config_path = repo_root.join(BUF_CONFIG_FILE);
    let output = Command::new(BUF_PROGRAM)
        .args(BUF_BUILD_ARGUMENTS)
        .current_dir(repo_root)
        .output()
        .map_err(|error| codegen_error(&config_path, format!("can't run buf build: {error}")))?;
    if !output.status.success() {
        let problem = format!("buf build failed: {}", String::from_utf8_lossy(&output.stderr).trim());
        return Err(codegen_error(&config_path, problem));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| codegen_error(&config_path, format!("buf build wrote no JSON: {error}")))
}

/// The fields of the message `message_name` (a full name such as `rastai.events.v1.JobStateChanged`), in the
/// order the .proto file declares them.
pub fn payload_fields(descriptor_set: &Value, message_name: &str) -> Result<Vec<PayloadField>, CodegenError> {
    let message = find_message(descriptor_set, message_name).ok_or_else(|| missing_message(message_name))?;
    let mut fields = Vec::new();
    for field in list(message, "field") {
        fields.push(payload_field(descriptor_set, field)?);
    }
    Ok(fields)
}

fn payload_field(descriptor_set: &Value, field: &Value) -> Result<PayloadField, CodegenError> {
    let name = text(field, "name").to_string();
    let is_repeated = text(field, "label") == REPEATED_LABEL;
    // A map is written as a repeated entry message on the wire, but read and written as a map everywhere else.
    if let Some(map_type) = map_type_name(descriptor_set, field)? {
        return Ok(PayloadField {
            name,
            number: number(field),
            type_name: map_type,
            repeated: false,
            optional: false,
        });
    }
    Ok(PayloadField {
        name,
        number: number(field),
        type_name: field_type_name(field),
        repeated: is_repeated,
        optional: field.get("proto3Optional").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// `map<K, V>` when the field is a map, else None.
fn map_type_name(descriptor_set: &Value, field: &Value) -> Result<Option<String>, CodegenError> {
    if text(field, "type") != MESSAGE_TYPE_TYPE || text(field, "label") != REPEATED_LABEL {
        return Ok(None);
    }
    let entry_name = full_type_name(field);
    let entry = find_message(descriptor_set, &entry_name).ok_or_else(|| missing_message(&entry_name))?;
    let is_map_entry = entry["options"]["mapEntry"].as_bool().unwrap_or(false);
    if !is_map_entry {
        return Ok(None);
    }
    let key = entry_field_type(entry, MAP_KEY_FIELD, &entry_name)?;
    let value = entry_field_type(entry, MAP_VALUE_FIELD, &entry_name)?;
    Ok(Some(format!("map<{key}, {value}>")))
}

fn entry_field_type(entry: &Value, field_name: &str, entry_name: &str) -> Result<String, CodegenError> {
    for field in list(entry, "field") {
        if text(field, "name") == field_name {
            return Ok(field_type_name(field));
        }
    }
    let problem = format!("the map entry `{entry_name}` has no `{field_name}` field");
    Err(codegen_error(Path::new(BUF_CONFIG_FILE), problem))
}

/// A message or enum by its full name; a scalar by its .proto word (`TYPE_UINT64` is `uint64`).
fn field_type_name(field: &Value) -> String {
    let field_type = text(field, "type");
    if field_type == MESSAGE_TYPE_TYPE || field_type == ENUM_TYPE {
        return full_type_name(field);
    }
    field_type.trim_start_matches(SCALAR_TYPE_PREFIX).to_lowercase()
}

/// The descriptor writes `.rastai.common.v1.Diagnostic`; the leading dot only says the name is absolute.
fn full_type_name(field: &Value) -> String {
    text(field, "typeName").trim_start_matches('.').to_string()
}

/// The message with this full name, looking through every file and every nested message.
fn find_message<'a>(descriptor_set: &'a Value, message_name: &str) -> Option<&'a Value> {
    for file in list(descriptor_set, "file") {
        let package = text(file, "package");
        for message in list(file, "messageType") {
            if let Some(found) = find_in_message(message, package, message_name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_in_message<'a>(message: &'a Value, parent_name: &str, message_name: &str) -> Option<&'a Value> {
    let full_name = format!("{parent_name}.{}", text(message, "name"));
    if full_name == message_name {
        return Some(message);
    }
    // Only a name that continues this message's name can be nested in it.
    if !message_name.starts_with(&format!("{full_name}.")) {
        return None;
    }
    for nested in list(message, "nestedType") {
        if let Some(found) = find_in_message(nested, &full_name, message_name) {
            return Some(found);
        }
    }
    None
}

fn missing_message(message_name: &str) -> CodegenError {
    let problem = format!("the payload message `{message_name}` isn't in the `buf build` descriptor set of proto/");
    codegen_error(Path::new(EVENT_TYPES_FILE), problem)
}

/// The array under `key`, or nothing: the JSON mapping leaves out an empty list.
fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The string under `key`, or "" (the JSON mapping leaves out a default value).
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number(field: &Value) -> u64 {
    field.get("number").and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A small descriptor set in the shape buf writes: a message with each kind of field, and a map entry.
    fn sample_descriptor_set() -> Value {
        json!({
            "file": [
                {
                    "name": "rastai/common/v1/diagnostic.proto",
                    "package": "rastai.common.v1",
                    "messageType": [{"name": "Diagnostic", "field": [
                        {"name": "code", "number": 1, "label": "LABEL_OPTIONAL", "type": "TYPE_STRING"}
                    ]}]
                },
                {
                    "name": "rastai/events/v1/payloads.proto",
                    "package": "rastai.events.v1",
                    "messageType": [
                        {"name": "SampleChanged", "field": [
                            {"name": "sample_id", "number": 1, "label": "LABEL_OPTIONAL", "type": "TYPE_STRING"},
                            {"name": "step", "number": 2, "label": "LABEL_OPTIONAL", "type": "TYPE_UINT64"},
                            {"name": "exit_code", "number": 3, "label": "LABEL_OPTIONAL", "type": "TYPE_INT32",
                             "oneofIndex": 0, "proto3Optional": true},
                            {"name": "tags", "number": 4, "label": "LABEL_REPEATED", "type": "TYPE_STRING"},
                            {"name": "diagnostic", "number": 5, "label": "LABEL_OPTIONAL", "type": "TYPE_MESSAGE",
                             "typeName": ".rastai.common.v1.Diagnostic"},
                            {"name": "state", "number": 6, "label": "LABEL_OPTIONAL", "type": "TYPE_ENUM",
                             "typeName": ".rastai.common.v1.JobState"},
                            {"name": "values", "number": 7, "label": "LABEL_REPEATED", "type": "TYPE_MESSAGE",
                             "typeName": ".rastai.events.v1.SampleChanged.ValuesEntry"}
                        ], "nestedType": [
                            {"name": "ValuesEntry", "options": {"mapEntry": true}, "field": [
                                {"name": "key", "number": 1, "label": "LABEL_OPTIONAL", "type": "TYPE_STRING"},
                                {"name": "value", "number": 2, "label": "LABEL_OPTIONAL", "type": "TYPE_DOUBLE"}
                            ]}
                        ]},
                        {"name": "Parent", "field": []},
                        {"name": "SampleCreated", "field": [
                            {"name": "parents", "number": 1, "label": "LABEL_REPEATED", "type": "TYPE_MESSAGE",
                             "typeName": ".rastai.events.v1.Parent"}
                        ]}
                    ]
                }
            ]
        })
    }

    fn field_named<'a>(fields: &'a [PayloadField], name: &str) -> &'a PayloadField {
        fields.iter().find(|field| field.name == name).unwrap()
    }

    #[test]
    fn fields_come_in_declaration_order_with_their_numbers() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.SampleChanged").unwrap();
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "sample_id",
                "step",
                "exit_code",
                "tags",
                "diagnostic",
                "state",
                "values"
            ]
        );
        let numbers: Vec<u64> = fields.iter().map(|field| field.number).collect();
        assert_eq!(numbers, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn scalars_messages_and_enums_read_as_their_proto_names() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.SampleChanged").unwrap();
        assert_eq!(field_named(&fields, "sample_id").type_name, "string");
        assert_eq!(field_named(&fields, "step").type_name, "uint64");
        assert_eq!(
            field_named(&fields, "diagnostic").type_name,
            "rastai.common.v1.Diagnostic"
        );
        assert_eq!(field_named(&fields, "state").type_name, "rastai.common.v1.JobState");
    }

    #[test]
    fn repeated_and_optional_are_flags_and_a_plain_field_has_neither() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.SampleChanged").unwrap();
        let exit_code = field_named(&fields, "exit_code");
        assert!(exit_code.optional && !exit_code.repeated);
        let tags = field_named(&fields, "tags");
        assert!(tags.repeated && !tags.optional);
        let step = field_named(&fields, "step");
        assert!(!step.repeated && !step.optional);
    }

    #[test]
    fn a_map_reads_as_a_map_not_as_its_repeated_entry_message() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.SampleChanged").unwrap();
        let values = field_named(&fields, "values");
        assert_eq!(values.type_name, "map<string, double>");
        assert!(!values.repeated);
    }

    #[test]
    fn a_repeated_message_that_is_not_a_map_entry_stays_repeated() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.SampleCreated").unwrap();
        assert_eq!(fields[0].type_name, "rastai.events.v1.Parent");
        assert!(fields[0].repeated);
    }

    #[test]
    fn a_message_without_fields_has_none() {
        let fields = payload_fields(&sample_descriptor_set(), "rastai.events.v1.Parent").unwrap();
        assert!(fields.is_empty());
    }

    #[test]
    fn a_missing_message_is_an_error_naming_the_event_table() {
        let error = payload_fields(&sample_descriptor_set(), "rastai.events.v1.Missing").unwrap_err();
        assert_eq!(error.path, Path::new(EVENT_TYPES_FILE));
        assert!(error.problem.contains("`rastai.events.v1.Missing`"), "{error}");
    }

    #[test]
    fn a_name_in_another_package_is_not_found() {
        assert!(find_message(&sample_descriptor_set(), "rastai.node.v1.SampleChanged").is_none());
    }
}
