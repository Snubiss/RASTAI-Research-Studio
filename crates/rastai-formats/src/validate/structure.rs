//! Layer 2: which type a document is, then its header and body against that type's description (`RRS-E1xxx`,
//! each with a JSON pointer and the line and column it points at).
//!
//! The type comes from the header's `type` field, or from the file's extension when the field is missing. A type
//! no row knows is `RRS-E1004`, and then only the shared header is checked, because the other keys can't be
//! judged. A field and an extension that name two different types is `RRS-E1005`.

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use serde_json::Value;

use crate::describe::registry::find_description;
use crate::describe::structure_check::{MismatchKind, StructureCheck, StructureMismatch, find_mismatches};
use crate::describe::types::TypeDescription;
use crate::documents::{DocumentType, document_type_for_file, find_document_type};
use crate::header::{TYPE_KEY, describe_shared_header};
use crate::json::positions::PositionMap;
use crate::validate::diagnostic_path;

const CODE_MISSING_FIELD: &str = "RRS-E1001";
const CODE_WRONG_KIND: &str = "RRS-E1002";
const CODE_UNKNOWN_FIELD: &str = "RRS-E1003";
const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";
const CODE_TYPE_NOT_EXTENSION: &str = "RRS-E1005";

/// The document's type row, or None when it has none. Problems with the `type` value are added here; a missing or
/// non-string `type` is left to the header check, and the extension still names the type if it can.
pub fn identify_document_type<'a>(
    value: &Value,
    file_name: &str,
    positions: &PositionMap,
    document_types: &'a [DocumentType],
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'a DocumentType> {
    let from_extension = document_type_for_file(document_types, file_name);
    let Some(Value::String(type_name)) = value.get(TYPE_KEY) else {
        return from_extension;
    };
    let type_path = diagnostic_path(file_name, positions, &format!("/{TYPE_KEY}"));
    let Some(from_field) = find_document_type(document_types, type_name) else {
        let known_names: Vec<&str> = document_types.iter().map(|row| row.type_name).collect();
        let expected = format!("one of: {}", known_names.join(", "));
        let values = [("found", type_name.as_str()), ("expected", expected.as_str())];
        diagnostics.push(diagnostic_at(CODE_VALUE_NOT_ALLOWED, &values, &type_path));
        return None;
    };
    if let Some(extension_type) = from_extension
        && extension_type.type_name != from_field.type_name
    {
        let values = [
            ("type", from_field.type_name),
            ("extension_type", extension_type.type_name),
        ];
        diagnostics.push(diagnostic_at(CODE_TYPE_NOT_EXTENSION, &values, &type_path));
    }
    Some(from_field)
}

/// The description layer 2 checks a document against, and whether its unknown keys are errors. A known type is
/// checked whole; a file from a newer version (read-only) keeps the keys this version doesn't know. An unknown
/// type gets only the shared header.
pub fn structure_description(
    document_type: Option<&DocumentType>,
    all_types: &[TypeDescription],
    read_only: bool,
) -> (TypeDescription, bool) {
    let described = document_type.and_then(|row| find_description(all_types, row.description_name));
    match described {
        Some(description) => (description.clone(), !read_only),
        None => (describe_shared_header(), false),
    }
}

/// Every structure mismatch as a diagnostic, in the walk's order (describe/structure_check.rs).
pub fn check_structure(
    value: &Value,
    description: &TypeDescription,
    check: &StructureCheck,
    file_name: &str,
    positions: &PositionMap,
) -> Vec<Diagnostic> {
    find_mismatches(value, description, check)
        .iter()
        .map(|mismatch| mismatch_diagnostic(mismatch, file_name, positions))
        .collect()
}

fn mismatch_diagnostic(mismatch: &StructureMismatch, file_name: &str, positions: &PositionMap) -> Diagnostic {
    let values = [
        ("field", mismatch.pointer.as_str()),
        ("expected", mismatch.expected.as_str()),
        ("found", mismatch.found.as_str()),
    ];
    let path = diagnostic_path(file_name, positions, &mismatch.pointer);
    diagnostic_at(mismatch_code(mismatch.kind), &values, &path)
}

fn mismatch_code(kind: MismatchKind) -> &'static str {
    match kind {
        MismatchKind::MissingField => CODE_MISSING_FIELD,
        MismatchKind::WrongKind => CODE_WRONG_KIND,
        MismatchKind::UnknownField => CODE_UNKNOWN_FIELD,
        MismatchKind::ValueNotAllowed => CODE_VALUE_NOT_ALLOWED,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::json::reader::read_json;
    use crate::limits::ImportLimits;

    const SAMPLE_TYPES: &[DocumentType] = &[
        DocumentType {
            type_name: "test.sample",
            schema_version: 1,
            file_extensions: &[".sample"],
            description_name: "SharedHeader",
        },
        DocumentType {
            type_name: "test.other",
            schema_version: 1,
            file_extensions: &[".other"],
            description_name: "SharedHeader",
        },
    ];

    fn identify(text: &str, file_name: &str) -> (Option<&'static str>, Vec<(String, String)>) {
        let parsed = read_json(text.as_bytes(), &ImportLimits::default()).unwrap();
        let mut diagnostics = Vec::new();
        let found = identify_document_type(
            &parsed.value,
            file_name,
            &parsed.positions,
            SAMPLE_TYPES,
            &mut diagnostics,
        );
        let reported = diagnostics.into_iter().map(|item| (item.code, item.path)).collect();
        (found.map(|row| row.type_name), reported)
    }

    fn pair(code: &str, path: &str) -> (String, String) {
        (code.to_string(), path.to_string())
    }

    #[test]
    fn the_type_field_names_the_type() {
        assert_eq!(
            identify(r#"{"type": "test.other"}"#, "x.json"),
            (Some("test.other"), vec![])
        );
        assert_eq!(
            identify(r#"{"type": "test.sample"}"#, "x.sample"),
            (Some("test.sample"), vec![])
        );
    }

    #[test]
    fn without_a_type_field_the_extension_decides() {
        assert_eq!(identify(r#"{"id": "a"}"#, "x.sample"), (Some("test.sample"), vec![]));
        assert_eq!(identify(r#"{"type": 5}"#, "x.other"), (Some("test.other"), vec![]));
        assert_eq!(identify(r#"{"id": "a"}"#, "x.json"), (None, vec![]));
    }

    #[test]
    fn an_unknown_type_is_e1004_at_the_type_field() {
        let expected = (None, vec![pair("RRS-E1004", "x.sample:1:2#/type")]);
        assert_eq!(identify(r#"{"type": "test.nope"}"#, "x.sample"), expected);
    }

    #[test]
    fn a_type_that_disagrees_with_the_extension_is_e1005() {
        let expected = (Some("test.other"), vec![pair("RRS-E1005", "x.sample:1:2#/type")]);
        assert_eq!(identify(r#"{"type": "test.other"}"#, "x.sample"), expected);
    }

    #[test]
    fn unknown_types_and_newer_files_keep_their_unknown_keys() {
        let all_types = vec![describe_shared_header()];
        let (_, report_unknown) = structure_description(Some(&SAMPLE_TYPES[0]), &all_types, false);
        assert!(report_unknown);
        let (_, report_unknown) = structure_description(Some(&SAMPLE_TYPES[0]), &all_types, true);
        assert!(!report_unknown);
        let (description, report_unknown) = structure_description(None, &all_types, false);
        assert_eq!(description.name, "SharedHeader");
        assert!(!report_unknown);
    }

    #[test]
    fn mismatches_become_coded_diagnostics_at_their_position() {
        let text = "{\n  \"type\": \"t\",\n  \"schema_version\": \"1\",\n  \"extra\": 1\n}";
        let parsed = read_json(text.as_bytes(), &ImportLimits::default()).unwrap();
        let all_types = vec![describe_shared_header()];
        let check = StructureCheck {
            all_types: &all_types,
            report_unknown_fields: true,
        };
        let diagnostics = check_structure(&parsed.value, &all_types[0], &check, "a.json", &parsed.positions);
        let reported: Vec<(String, String)> = diagnostics.into_iter().map(|item| (item.code, item.path)).collect();
        // A missing field points at the object that should hold it, and names the field in the pointer.
        let expected = vec![
            pair("RRS-E1001", "a.json:1:1#/$schema"),
            pair("RRS-E1002", "a.json:3:3#/schema_version"),
            pair("RRS-E1001", "a.json:1:1#/id"),
            pair("RRS-E1003", "a.json:4:3#/extra"),
        ];
        assert_eq!(reported, expected);
    }
}
