//! Layer 2's walk: a raw JSON value against a description, collecting every mismatch with its JSON pointer in one
//! pass (layout 4.1, 4.4). It runs before `serde_json::from_value`, so serde never sees bad input; a `from_value`
//! failure after a clean walk is a description bug, which tests/describe_matches_struct.rs catches.
//!
//! This file only finds mismatches. validate/structure.rs turns them into coded diagnostics with a file, line and
//! column. The walk's recursion follows the value, whose depth the reader already capped (`max_json_depth`).

use serde_json::Value;

use crate::describe::registry::find_description;
use crate::describe::types::{FieldKind, TypeDescription, describe_kind};
use crate::json::pointer::{ROOT_POINTER, join_pointer};

/// What kind of mismatch was found. Each maps to one `RRS-E1xxx` code (validate/structure.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchKind {
    /// A required field isn't there.
    MissingField,
    /// The value is the wrong JSON kind: a string where a number belongs, `null` anywhere but a `Json` field.
    WrongKind,
    /// The object has a key its description doesn't list. Never ignored: a typo must not silently drop data.
    UnknownField,
    /// A string that isn't one of an `Enum`'s values.
    ValueNotAllowed,
}

/// One mismatch and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureMismatch {
    pub kind: MismatchKind,
    /// The value's pointer; for a missing field, the pointer the field would have.
    pub pointer: String,
    /// What the description wants there ("an integer", "one of: a, b"); empty for an unknown field.
    pub expected: String,
    /// What the JSON has ("a string", or the text of a value that isn't allowed); empty for a missing field.
    pub found: String,
}

/// The settings of one walk.
pub struct StructureCheck<'a> {
    /// Where `FieldKind::Type` names are looked up.
    pub all_types: &'a [TypeDescription],
    /// False only where a key can't be judged: a document whose type is unknown (only its header is checked), or a
    /// file from a newer schema version, whose new fields this version can't know (it opens read-only).
    pub report_unknown_fields: bool,
}

/// Every mismatch between a document (or any object) and its description, in a stable order: each object's fields
/// in description order, then its unknown keys in key order.
pub fn find_mismatches(value: &Value, description: &TypeDescription, check: &StructureCheck) -> Vec<StructureMismatch> {
    let mut mismatches = Vec::new();
    check_object(value, description, ROOT_POINTER, check, &mut mismatches);
    mismatches
}

fn check_object(
    value: &Value,
    description: &TypeDescription,
    pointer: &str,
    check: &StructureCheck,
    mismatches: &mut Vec<StructureMismatch>,
) {
    let Value::Object(members) = value else {
        mismatches.push(wrong_kind(pointer, &description.name, value));
        return;
    };
    for field in &description.fields {
        let field_pointer = join_pointer(pointer, &field.name);
        match members.get(&field.name) {
            Some(member) => check_kind(member, &field.kind, &field_pointer, check, mismatches),
            None if field.required => mismatches.push(StructureMismatch {
                kind: MismatchKind::MissingField,
                pointer: field_pointer,
                expected: describe_kind(&field.kind),
                found: String::new(),
            }),
            None => {}
        }
    }
    if check.report_unknown_fields {
        for key in members.keys() {
            let known = description.fields.iter().any(|field| field.name == *key);
            if !known {
                mismatches.push(StructureMismatch {
                    kind: MismatchKind::UnknownField,
                    pointer: join_pointer(pointer, key),
                    expected: String::new(),
                    found: String::new(),
                });
            }
        }
    }
}

fn check_kind(
    value: &Value,
    kind: &FieldKind,
    pointer: &str,
    check: &StructureCheck,
    mismatches: &mut Vec<StructureMismatch>,
) {
    let kind_matches = match kind {
        FieldKind::String => value.is_string(),
        FieldKind::Integer => value.is_i64() || value.is_u64(),
        FieldKind::Number => value.is_number(),
        FieldKind::Boolean => value.is_boolean(),
        FieldKind::Json => true,
        // Only the outer kind is checked here; the containers below check their own contents.
        FieldKind::Enum(_) => value.is_string(),
        FieldKind::List(_) => value.is_array(),
        FieldKind::Map(_) | FieldKind::Type(_) | FieldKind::Proto(_) => value.is_object(),
    };
    if !kind_matches {
        mismatches.push(wrong_kind(pointer, &describe_kind(kind), value));
        return;
    }
    match kind {
        FieldKind::Enum(allowed) => check_enum_value(value, allowed, kind, pointer, mismatches),
        FieldKind::List(element_kind) => check_list(value, element_kind, pointer, check, mismatches),
        FieldKind::Map(member_kind) => check_map(value, member_kind, pointer, check, mismatches),
        FieldKind::Type(type_name) => check_named_type(value, type_name, pointer, check, mismatches),
        _ => {}
    }
}

fn check_enum_value(
    value: &Value,
    allowed: &[String],
    kind: &FieldKind,
    pointer: &str,
    mismatches: &mut Vec<StructureMismatch>,
) {
    let Value::String(text) = value else {
        return;
    };
    if !allowed.contains(text) {
        mismatches.push(StructureMismatch {
            kind: MismatchKind::ValueNotAllowed,
            pointer: pointer.to_string(),
            expected: describe_kind(kind),
            found: value.to_string(),
        });
    }
}

fn check_list(
    value: &Value,
    element_kind: &FieldKind,
    pointer: &str,
    check: &StructureCheck,
    mismatches: &mut Vec<StructureMismatch>,
) {
    let Value::Array(elements) = value else {
        return;
    };
    for (index, element) in elements.iter().enumerate() {
        let element_pointer = join_pointer(pointer, &index.to_string());
        check_kind(element, element_kind, &element_pointer, check, mismatches);
    }
}

fn check_map(
    value: &Value,
    member_kind: &FieldKind,
    pointer: &str,
    check: &StructureCheck,
    mismatches: &mut Vec<StructureMismatch>,
) {
    let Value::Object(members) = value else {
        return;
    };
    for (key, member) in members {
        check_kind(member, member_kind, &join_pointer(pointer, key), check, mismatches);
    }
}

/// A reference to an unregistered type checks nothing: that is a description bug, and the registry's own test
/// fails on it (describe/registry.rs), so it never reaches a user's file.
fn check_named_type(
    value: &Value,
    type_name: &str,
    pointer: &str,
    check: &StructureCheck,
    mismatches: &mut Vec<StructureMismatch>,
) {
    if let Some(description) = find_description(check.all_types, type_name) {
        check_object(value, description, pointer, check, mismatches);
    }
}

fn wrong_kind(pointer: &str, expected: &str, value: &Value) -> StructureMismatch {
    StructureMismatch {
        kind: MismatchKind::WrongKind,
        pointer: pointer.to_string(),
        expected: expected.to_string(),
        found: json_kind_name(value).to_string(),
    }
}

/// How a JSON value's kind is named in diagnostics, in the same words as `describe_kind`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::describe::types::{OPTIONAL, REQUIRED, enum_of, field, list_of};
    use serde_json::json;

    fn point_type() -> TypeDescription {
        TypeDescription {
            name: "Point".to_string(),
            schema_id: "test.sample/1".to_string(),
            doc: "A point.".to_string(),
            fields: vec![
                field("x", FieldKind::Integer, REQUIRED, "X."),
                field("y", FieldKind::Integer, REQUIRED, "Y."),
            ],
        }
    }

    fn sample_type() -> TypeDescription {
        TypeDescription {
            name: "Sample".to_string(),
            schema_id: "test.sample/1".to_string(),
            doc: "A sample with every kind.".to_string(),
            fields: vec![
                field("name", FieldKind::String, REQUIRED, "Name."),
                field("ratio", FieldKind::Number, OPTIONAL, "Ratio."),
                field("enabled", FieldKind::Boolean, OPTIONAL, "On or off."),
                field("attributes", FieldKind::Json, OPTIONAL, "Anything."),
                field("mode", enum_of(&["fast", "slow"]), OPTIONAL, "Mode."),
                field(
                    "points",
                    list_of(FieldKind::Type("Point".to_string())),
                    OPTIONAL,
                    "Points.",
                ),
                field(
                    "limits",
                    FieldKind::Map(Box::new(FieldKind::Integer)),
                    OPTIONAL,
                    "Limits.",
                ),
                field(
                    "job_spec",
                    FieldKind::Proto("rastai.node.v1.JobSpec".to_string()),
                    OPTIONAL,
                    "Job.",
                ),
            ],
        }
    }

    fn mismatches_of(value: Value, report_unknown_fields: bool) -> Vec<(MismatchKind, String)> {
        let all_types = vec![sample_type(), point_type()];
        let check = StructureCheck {
            all_types: &all_types,
            report_unknown_fields,
        };
        find_mismatches(&value, &sample_type(), &check)
            .into_iter()
            .map(|mismatch| (mismatch.kind, mismatch.pointer))
            .collect()
    }

    fn found(kind: MismatchKind, pointer: &str) -> (MismatchKind, String) {
        (kind, pointer.to_string())
    }

    #[test]
    fn a_matching_value_has_no_mismatches() {
        let value = json!({
            "name": "a", "ratio": 0.5, "enabled": true, "attributes": [null, {"any": 1}], "mode": "fast",
            "points": [{"x": 1, "y": -2}], "limits": {"steps": 10}, "job_spec": {"job_id": "j"}
        });
        assert_eq!(mismatches_of(value, true), vec![]);
    }

    #[test]
    fn every_mismatch_is_found_in_one_pass_with_its_pointer() {
        let value = json!({
            "ratio": "high", "enabled": null, "mode": "medium", "points": [{"x": 1.5}, 3],
            "limits": {"a/b": "x"}, "job_spec": [], "extra": 1
        });
        let expected = vec![
            found(MismatchKind::MissingField, "/name"),
            found(MismatchKind::WrongKind, "/ratio"),
            found(MismatchKind::WrongKind, "/enabled"),
            found(MismatchKind::ValueNotAllowed, "/mode"),
            found(MismatchKind::WrongKind, "/points/0/x"),
            found(MismatchKind::MissingField, "/points/0/y"),
            found(MismatchKind::WrongKind, "/points/1"),
            found(MismatchKind::WrongKind, "/limits/a~1b"),
            found(MismatchKind::WrongKind, "/job_spec"),
            found(MismatchKind::UnknownField, "/extra"),
        ];
        assert_eq!(mismatches_of(value, true), expected);
    }

    #[test]
    fn unknown_fields_are_reported_at_every_depth_unless_turned_off() {
        let value = json!({"name": "a", "points": [{"x": 1, "y": 2, "z": 3}], "nmae": "typo"});
        let expected = vec![
            found(MismatchKind::UnknownField, "/points/0/z"),
            found(MismatchKind::UnknownField, "/nmae"),
        ];
        assert_eq!(mismatches_of(value.clone(), true), expected);
        assert_eq!(mismatches_of(value, false), vec![]);
    }

    #[test]
    fn a_root_that_is_not_an_object_is_one_mismatch() {
        assert_eq!(
            mismatches_of(json!([1]), true),
            vec![found(MismatchKind::WrongKind, "")]
        );
    }

    #[test]
    fn mismatches_say_what_was_expected_and_found() {
        let all_types = vec![sample_type(), point_type()];
        let check = StructureCheck {
            all_types: &all_types,
            report_unknown_fields: true,
        };
        let value = json!({"name": 7, "mode": "medium", "points": [{"x": 1.0, "y": 1}]});
        let mismatches = find_mismatches(&value, &sample_type(), &check);
        assert_eq!(mismatches[0].expected, "a string");
        assert_eq!(mismatches[0].found, "an integer");
        assert_eq!(mismatches[1].expected, "one of: fast, slow");
        assert_eq!(mismatches[1].found, "\"medium\"");
        // 1.0 is held as a float, so it isn't an integer (contracts 5.1: integers have no fraction in the text).
        assert_eq!(mismatches[2].found, "a number with a fraction or exponent");
    }

    #[test]
    fn integers_accept_the_full_signed_and_unsigned_range() {
        let value = json!({"name": "a", "limits": {"low": i64::MIN, "high": u64::MAX}});
        assert_eq!(mismatches_of(value, true), vec![]);
    }
}
