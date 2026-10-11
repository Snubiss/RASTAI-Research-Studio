//! A description as a JSON Schema value (layout 4.1): at run time for MCP tool lists and `ListCommands`, at build
//! time for the files the codegen writes into `schemas/` (M1-16).
//!
//! The schema says what layer 2 checks, so an editor and Rastai agree on a file: no unknown keys
//! (`additionalProperties: false`), required fields listed, integers as `integer`, a `Proto` field as an object
//! whose description names its protobuf message. Types the description refers to go into `$defs`, each once.

use serde_json::{Map, Value, json};

use crate::describe::registry::find_description;
use crate::describe::types::{FieldDescription, FieldKind, TypeDescription};

/// The JSON Schema dialect the files declare.
const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
const DEFINITIONS_POINTER: &str = "#/$defs/";
/// The positional CLI argument a field fills (contracts 15.1, beside `x-rastai-stability`).
const CLI_POSITION_KEYWORD: &str = "x-rastai-cli-position";

/// The JSON Schema of one described type, with every type it refers to (directly or not) under `$defs`.
pub fn json_schema_for(description: &TypeDescription, all_types: &[TypeDescription]) -> Value {
    let mut schema = object_schema(description);
    schema.insert("$schema".to_string(), json!(SCHEMA_DIALECT));
    let definitions = referenced_definitions(description, all_types);
    if !definitions.is_empty() {
        schema.insert("$defs".to_string(), Value::Object(definitions));
    }
    Value::Object(schema)
}

/// The schema of one object type, without `$schema` or `$defs`.
fn object_schema(description: &TypeDescription) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &description.fields {
        properties.insert(field.name.clone(), field_schema(field));
        if field.required {
            required.push(json!(field.name));
        }
    }
    let mut schema = Map::new();
    schema.insert("title".to_string(), json!(description.name));
    schema.insert("description".to_string(), json!(description.doc));
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), Value::Object(properties));
    schema.insert("required".to_string(), Value::Array(required));
    schema.insert("additionalProperties".to_string(), json!(false));
    schema
}

/// A field's kind schema plus its doc and CLI position. A `Proto` kind already carries a description naming its
/// message, so the field's doc goes in front of it.
fn field_schema(field: &FieldDescription) -> Value {
    let mut schema = kind_schema(&field.kind);
    let description = match schema.get("description").and_then(Value::as_str) {
        Some(kind_description) => format!("{} {kind_description}", field.doc),
        None => field.doc.clone(),
    };
    schema.insert("description".to_string(), json!(description));
    if let Some(position) = field.cli_position {
        schema.insert(CLI_POSITION_KEYWORD.to_string(), json!(position));
    }
    Value::Object(schema)
}

fn kind_schema(kind: &FieldKind) -> Map<String, Value> {
    let schema = match kind {
        FieldKind::String => json!({"type": "string"}),
        FieldKind::Integer => json!({"type": "integer"}),
        FieldKind::Number => json!({"type": "number"}),
        FieldKind::Boolean => json!({"type": "boolean"}),
        // Any value at all: the empty schema.
        FieldKind::Json => json!({}),
        FieldKind::Enum(values) => json!({"type": "string", "enum": values}),
        FieldKind::List(element_kind) => json!({"type": "array", "items": kind_schema(element_kind)}),
        FieldKind::Map(member_kind) => json!({"type": "object", "additionalProperties": kind_schema(member_kind)}),
        FieldKind::Type(name) => json!({"$ref": format!("{DEFINITIONS_POINTER}{name}")}),
        FieldKind::Proto(message) => json!({"type": "object", "description": format!("Protobuf JSON of {message}.")}),
    };
    match schema {
        Value::Object(members) => members,
        _ => Map::new(),
    }
}

/// Every type reachable from this one, as `$defs` entries. A worklist rather than recursion, so a type that refers
/// to itself (a layout tree's children) is visited once.
fn referenced_definitions(description: &TypeDescription, all_types: &[TypeDescription]) -> Map<String, Value> {
    let mut definitions = Map::new();
    let mut to_visit = type_references(description);
    while let Some(name) = to_visit.pop() {
        if definitions.contains_key(&name) {
            continue;
        }
        let Some(referenced) = find_description(all_types, &name) else {
            continue;
        };
        to_visit.extend(type_references(referenced));
        definitions.insert(name, Value::Object(object_schema(referenced)));
    }
    definitions
}

/// The type names this description's fields refer to, nested kinds included.
fn type_references(description: &TypeDescription) -> Vec<String> {
    let mut names = Vec::new();
    for field in &description.fields {
        collect_type_names(&field.kind, &mut names);
    }
    names
}

fn collect_type_names(kind: &FieldKind, names: &mut Vec<String>) {
    match kind {
        FieldKind::Type(name) => names.push(name.clone()),
        FieldKind::List(inner) | FieldKind::Map(inner) => collect_type_names(inner, names),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::describe::types::{OPTIONAL, REQUIRED, enum_of, field, list_of};
    use crate::header::describe_shared_header;

    fn node_type() -> TypeDescription {
        TypeDescription {
            name: "Node".to_string(),
            schema_id: "test.tree/1".to_string(),
            doc: "A tree node.".to_string(),
            fields: vec![
                field("kind", enum_of(&["split", "panel"]), REQUIRED, "What it is."),
                field(
                    "children",
                    list_of(FieldKind::Type("Node".to_string())),
                    OPTIONAL,
                    "Child nodes.",
                ),
            ],
        }
    }

    fn tree_type() -> TypeDescription {
        TypeDescription {
            name: "Tree".to_string(),
            schema_id: "test.tree/1".to_string(),
            doc: "A tree.".to_string(),
            fields: vec![
                FieldDescription {
                    cli_position: Some(0),
                    ..field("root", FieldKind::Type("Node".to_string()), REQUIRED, "The root.")
                },
                field(
                    "fingerprint",
                    FieldKind::Proto("rastai.common.v1.HardwareFingerprint".to_string()),
                    OPTIONAL,
                    "Where.",
                ),
                field("extra", FieldKind::Json, OPTIONAL, "Anything."),
            ],
        }
    }

    #[test]
    fn the_header_schema_matches_layer_two() {
        let all_types = vec![describe_shared_header()];
        let schema = json_schema_for(&all_types[0], &all_types);
        assert_eq!(schema["$schema"], SCHEMA_DIALECT);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["$schema", "type", "schema_version", "id"]));
        assert_eq!(schema["properties"]["schema_version"]["type"], "integer");
        assert_eq!(
            schema["properties"]["requires"]["additionalProperties"]["type"],
            "string"
        );
        assert_eq!(schema["properties"]["authors"]["items"]["type"], "string");
        assert!(schema.get("$defs").is_none());
    }

    #[test]
    fn referenced_types_go_into_defs_once_even_when_recursive() {
        let all_types = vec![tree_type(), node_type()];
        let schema = json_schema_for(&tree_type(), &all_types);
        assert_eq!(schema["properties"]["root"]["$ref"], "#/$defs/Node");
        assert_eq!(schema["properties"]["root"][CLI_POSITION_KEYWORD], 0);
        let node = &schema["$defs"]["Node"];
        assert_eq!(node["properties"]["children"]["items"]["$ref"], "#/$defs/Node");
        assert_eq!(node["properties"]["kind"]["enum"], json!(["split", "panel"]));
        assert_eq!(schema["$defs"].as_object().map(Map::len), Some(1));
    }

    #[test]
    fn proto_fields_are_objects_named_after_their_message() {
        let schema = json_schema_for(&tree_type(), &[tree_type(), node_type()]);
        let fingerprint = &schema["properties"]["fingerprint"];
        assert_eq!(fingerprint["type"], "object");
        assert_eq!(
            fingerprint["description"],
            "Where. Protobuf JSON of rastai.common.v1.HardwareFingerprint."
        );
        // A Json field accepts anything: no "type" keyword.
        assert!(schema["properties"]["extra"].get("type").is_none());
    }
}
