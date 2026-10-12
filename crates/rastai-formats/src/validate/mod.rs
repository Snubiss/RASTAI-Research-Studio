//! `read_document`, the one entry point for untrusted document bytes (layout 4.3, 4.4): layer 1 (syntax), the
//! migration chain, layer 2 (structure), then the typed document and layer 3 (meaning), with every diagnostic
//! collected in one pass. Layer 4 (environment) is a separate call, `check_environment`. Its way back out is
//! `write_document`, which fills the content hash and writes the stable pretty form.
//!
//! | Layer | Codes | Blocks loading? |
//! |---|---|---|
//! | 1 syntax | `RRS-E0xxx`, limits `RRS-E5xxx` | yes, and nothing after it runs |
//! | versions | `RRS-E4001` newer file (read-only), `RRS-E4002` failed step | only a failed step, which stops the read |
//! | 2 structure | `RRS-E1xxx` | yes |
//! | 3 meaning | `RRS-E2xxx`, and `RRS-P0xxx` for a plugin manifest | no: the document comes back with its errors |

pub mod environment;
pub mod meaning;
pub mod structure;
pub mod syntax;

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at, has_errors};
use serde_json::Value;

use crate::byte_source::ByteSource;
use crate::describe::registry::describe_all_types;
use crate::describe::structure_check::StructureCheck;
use crate::describe::types::TypeDescription;
use crate::documents::{DOCUMENT_TYPES, Document, DocumentType, document_from_value, document_header_mut};
use crate::error::FormatsError;
use crate::hashing::content_hash_of_value;
use crate::json::positions::PositionMap;
use crate::json::pretty::pretty_json;
use crate::limits::ImportLimits;
use crate::migrate::chain_table::MIGRATION_STEPS;
use crate::migrate::{MigrationInput, MigrationResult, migrate_to_current};
use crate::validate::meaning::{MeaningSource, check_meaning};
use crate::validate::structure::{check_structure, identify_document_type, structure_description};
use crate::validate::syntax::read_raw_json;

/// serde refuses a value layer 2 passed in two cases: an integer outside its field's Rust type (a negative step, a
/// font size past `u32`), because `FieldKind::Integer` says nothing about sign or width; and a description that
/// disagrees with its struct, which tests/describe_matches_struct.rs keeps from happening. Either way the read
/// fails with this code at the file, since serde's reason names no field (fixtures/documents/rastai.resume/invalid/
/// negative_step.json). The words below fill the catalog sentence "{field} should be {expected}, but it is
/// {found}." without serde's Rust type names (`u64`), which mean nothing to the person reading it.
const CODE_WRONG_KIND: &str = "RRS-E1002";
const FIELD_SOME_NUMBER: &str = "A number in this file";
const EXPECTED_FITTING_NUMBER: &str = "a whole number of zero or more that fits its field";
const FIELD_SOME_VALUE: &str = "A value in this file";
const EXPECTED_FITTING_VALUE: &str = "the kind of value its field holds";
const FOUND_SOMETHING_ELSE: &str = "something else";
/// serde writes an integer it refused as "integer `-1`"; the value sits between the two backticks.
const SERDE_INTEGER_MARK: &str = "integer `";
const SERDE_VALUE_END: char = '`';

/// What `read_document` found.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadOutcome {
    /// None when layer 1 or 2 found errors, or when no document type matched. Layer-3 errors don't clear it: the
    /// document opens with them shown, and the actions that need a sound document refuse it.
    pub document: Option<Document>,
    /// Every layer's findings, collected in one pass.
    pub diagnostics: Vec<Diagnostic>,
    /// The file is from a newer schema version (`RRS-E4001`).
    pub read_only: bool,
    /// 0 when no migration ran.
    pub migrated_from_version: u32,
}

/// Layers 1 and 2 and the migration chain over the raw value, before any typed struct is built.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedDocument {
    /// The value at the current schema version; None when a diagnostic is an error.
    pub value: Option<Value>,
    /// The document type's name; empty when no type matched.
    pub type_name: String,
    pub diagnostics: Vec<Diagnostic>,
    pub read_only: bool,
    pub migrated_from_version: u32,
    /// Where each value starts in the text, for the layers that run after this one; empty when layer 1 failed.
    pub positions: PositionMap,
}

/// Reads one document from untrusted bytes. `file_name` is used for the extension and in every diagnostic path;
/// nothing is opened by name.
pub fn read_document(file_name: &str, source: &dyn ByteSource, limits: &ImportLimits) -> ReadOutcome {
    let checked = check_document(file_name, source, limits, DOCUMENT_TYPES, &describe_all_types());
    let mut diagnostics = checked.diagnostics;
    let document = match checked.value {
        Some(value) => typed_document(file_name, &checked.type_name, value, &mut diagnostics),
        None => None,
    };
    if let Some(document) = &document {
        let meaning_source = MeaningSource {
            file_name,
            positions: &checked.positions,
        };
        diagnostics.extend(check_meaning(document, &meaning_source));
    }
    ReadOutcome {
        document,
        diagnostics,
        read_only: checked.read_only,
        migrated_from_version: checked.migrated_from_version,
    }
}

/// A typed document as the bytes of its file: its content hash filled in, then the stable pretty form (two-space
/// indent, fields in the struct's order). Writing the result and reading it back gives the same document and hash.
pub fn write_document(document: &Document) -> Result<Vec<u8>, FormatsError> {
    let mut hashed = document.clone();
    document_header_mut(&mut hashed).content_hash = document_content_hash(document)?;
    // The struct, not a JSON value, is written: a value's keys come out sorted, the struct's in description order.
    pretty_json(&hashed)
}

/// A typed document's content hash (contracts 5.1): BLAKE3 of the canonical JSON of the whole document, its own
/// `content_hash` left out. Fields the file left to their defaults count with their default values, so a file and
/// the same file written back by `write_document` have one hash.
pub fn document_content_hash(document: &Document) -> Result<String, FormatsError> {
    let value = serde_json::to_value(document).map_err(|error| FormatsError::NotSerializable {
        reason: error.to_string(),
    })?;
    Ok(content_hash_of_value(&value))
}

/// The typed struct for a value layers 1 and 2 passed, built with serde; None when no type matched.
fn typed_document(
    file_name: &str,
    type_name: &str,
    value: Value,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Document> {
    match document_from_value(type_name, value)? {
        Ok(document) => Some(document),
        Err(reason) => {
            diagnostics.push(wrong_kind_diagnostic(file_name, &reason));
            None
        }
    }
}

/// `RRS-E1002` at the file for a value serde refused. An integer out of its field's range is the case layer 2 lets
/// through, so it gets a sentence of its own with the value shown: "A number in this file should be a whole number of
/// zero or more that fits its field, but it is -1."
pub(crate) fn wrong_kind_diagnostic(file_name: &str, serde_reason: &str) -> Diagnostic {
    let values = match refused_integer(serde_reason) {
        Some(integer) => [
            ("field", FIELD_SOME_NUMBER),
            ("expected", EXPECTED_FITTING_NUMBER),
            ("found", integer),
        ],
        None => [
            ("field", FIELD_SOME_VALUE),
            ("expected", EXPECTED_FITTING_VALUE),
            ("found", FOUND_SOMETHING_ELSE),
        ],
    };
    diagnostic_at(CODE_WRONG_KIND, &values, file_name)
}

/// The integer in serde's "invalid value: integer `-1`, expected u64"; None for any other reason.
fn refused_integer(serde_reason: &str) -> Option<&str> {
    let start = serde_reason.find(SERDE_INTEGER_MARK)? + SERDE_INTEGER_MARK.len();
    let rest = &serde_reason[start..];
    let end = rest.find(SERDE_VALUE_END)?;
    Some(&rest[..end])
}

/// The layers `read_document` runs, over any table of document types and descriptions. `read_document` passes the
/// built-in ones; a test can pass a type of its own.
pub fn check_document(
    file_name: &str,
    source: &dyn ByteSource,
    limits: &ImportLimits,
    document_types: &[DocumentType],
    all_types: &[TypeDescription],
) -> CheckedDocument {
    let mut diagnostics = Vec::new();
    let Some(raw) = read_raw_json(file_name, source, limits, &mut diagnostics) else {
        return stopped(String::new(), diagnostics, PositionMap::default());
    };
    let document_type = identify_document_type(&raw.value, file_name, &raw.positions, document_types, &mut diagnostics);
    let type_name = document_type.map(|row| row.type_name.to_string()).unwrap_or_default();
    let migration = match document_type {
        Some(row) => {
            let input = MigrationInput {
                document_type: row,
                steps: MIGRATION_STEPS,
                file_name,
                positions: &raw.positions,
            };
            migrate_to_current(raw.value, &input, &mut diagnostics)
        }
        None => MigrationResult {
            value: raw.value,
            migrated_from_version: 0,
            read_only: false,
            failed: false,
        },
    };
    if migration.failed {
        return stopped(type_name, diagnostics, raw.positions);
    }
    let (description, report_unknown_fields) = structure_description(document_type, all_types, migration.read_only);
    let check = StructureCheck {
        all_types,
        report_unknown_fields,
    };
    diagnostics.extend(check_structure(
        &migration.value,
        &description,
        &check,
        file_name,
        &raw.positions,
    ));
    let value = if has_errors(&diagnostics) {
        None
    } else {
        Some(migration.value)
    };
    CheckedDocument {
        value,
        type_name,
        diagnostics,
        read_only: migration.read_only,
        migrated_from_version: migration.migrated_from_version,
        positions: raw.positions,
    }
}

/// The result of a read that stopped before layer 2.
fn stopped(type_name: String, diagnostics: Vec<Diagnostic>, positions: PositionMap) -> CheckedDocument {
    CheckedDocument {
        value: None,
        type_name,
        diagnostics,
        read_only: false,
        migrated_from_version: 0,
        positions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_source::SliceSource;

    fn read(text: &str) -> ReadOutcome {
        read_document(
            "doc.json",
            &SliceSource { bytes: text.as_bytes() },
            &ImportLimits::default(),
        )
    }

    fn codes(outcome: &ReadOutcome) -> Vec<&str> {
        outcome.diagnostics.iter().map(|item| item.code.as_str()).collect()
    }

    #[test]
    fn a_syntax_error_stops_before_layer_two() {
        let outcome = read("{\"type\": ");
        assert_eq!(codes(&outcome), vec!["RRS-E0001"]);
        assert!(outcome.document.is_none());
    }

    #[test]
    fn header_problems_and_an_unknown_type_come_in_one_pass() {
        let outcome = read(r#"{"$schema": 1, "type": "rastai.no_such_type", "schema_version": "1", "notes": "x"}"#);
        // The unknown `notes` key isn't judged: without a known type only the header is checked.
        assert_eq!(
            codes(&outcome),
            vec!["RRS-E1004", "RRS-E1002", "RRS-E1002", "RRS-E1001"]
        );
        assert!(!outcome.read_only);
        assert_eq!(outcome.migrated_from_version, 0);
    }

    #[test]
    fn a_refused_integer_is_shown_without_rust_type_names() {
        let reason = "invalid value: integer `-1`, expected u64";
        assert_eq!(refused_integer(reason), Some("-1"));
        let message = wrong_kind_diagnostic("run.json", reason).message;
        assert_eq!(
            message,
            "A number in this file should be a whole number of zero or more that fits its field, but it is -1."
        );
    }

    #[test]
    fn any_other_refusal_still_reads_as_a_sentence() {
        let reason = "missing field `step`";
        assert_eq!(refused_integer(reason), None);
        let message = wrong_kind_diagnostic("run.json", reason).message;
        assert_eq!(
            message,
            "A value in this file should be the kind of value its field holds, but it is something else."
        );
    }

    #[test]
    fn a_root_that_is_not_an_object_is_one_wrong_kind() {
        let outcome = read("[1, 2]");
        assert_eq!(codes(&outcome), vec!["RRS-E1002"]);
        assert_eq!(outcome.diagnostics[0].path, "doc.json:1:1");
    }
}
