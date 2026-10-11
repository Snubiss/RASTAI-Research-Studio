//! `read_document`, the one entry point for untrusted document bytes (layout 4.3, 4.4): layer 1 (syntax), the
//! migration chain, then layer 2 (structure), with every diagnostic collected in one pass. Layer 3 (meaning) joins
//! with the first document types (M1-17); layer 4 (environment) is a separate call, `check_environment`.
//!
//! | Layer | Codes | Blocks loading? |
//! |---|---|---|
//! | 1 syntax | `RRS-E0xxx`, limits `RRS-E5xxx` | yes, and nothing after it runs |
//! | versions | `RRS-E4001` newer file (read-only), `RRS-E4002` failed step | only a failed step, which stops the read |
//! | 2 structure | `RRS-E1xxx` | yes |

pub mod environment;
pub mod structure;
pub mod syntax;

use rastai_diag::diagnostic::{Diagnostic, has_errors};
use rastai_diag::path::{DiagnosticPath, format_path};
use serde_json::Value;

use crate::byte_source::ByteSource;
use crate::describe::registry::describe_all_types;
use crate::describe::structure_check::StructureCheck;
use crate::describe::types::TypeDescription;
use crate::documents::{DOCUMENT_TYPES, Document, DocumentType};
use crate::json::positions::{PositionMap, find_position};
use crate::limits::ImportLimits;
use crate::migrate::chain_table::MIGRATION_STEPS;
use crate::migrate::{MigrationInput, MigrationResult, migrate_to_current};
use crate::validate::structure::{check_structure, identify_document_type, structure_description};
use crate::validate::syntax::read_raw_json;

/// What `read_document` found.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadOutcome {
    /// None when layer 1 or 2 found errors, or when no document type matched.
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
}

/// Reads one document from untrusted bytes. `file_name` is used for the extension and in every diagnostic path;
/// nothing is opened by name.
pub fn read_document(file_name: &str, source: &dyn ByteSource, limits: &ImportLimits) -> ReadOutcome {
    let checked = check_document(file_name, source, limits, DOCUMENT_TYPES, &describe_all_types());
    // The typed struct is built here, with serde from the checked value, once a document type exists (M1-17).
    ReadOutcome {
        document: None,
        diagnostics: checked.diagnostics,
        read_only: checked.read_only,
        migrated_from_version: checked.migrated_from_version,
    }
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
        return stopped(String::new(), diagnostics);
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
        return stopped(type_name, diagnostics);
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
    }
}

/// The result of a read that stopped before layer 2.
fn stopped(type_name: String, diagnostics: Vec<Diagnostic>) -> CheckedDocument {
    CheckedDocument {
        value: None,
        type_name,
        diagnostics,
        read_only: false,
        migrated_from_version: 0,
    }
}

/// `file:line:column#pointer` for a value in the document. The position is the pointer's own or, for a missing
/// field, its nearest recorded parent's (the object that should hold it).
pub(crate) fn diagnostic_path(file_name: &str, positions: &PositionMap, pointer: &str) -> String {
    let position = find_position(positions, pointer);
    format_path(&DiagnosticPath {
        file: file_name.to_string(),
        line: position.map_or(0, |place| place.line),
        column: position.map_or(0, |place| place.column),
        json_pointer: pointer.to_string(),
    })
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
    fn a_root_that_is_not_an_object_is_one_wrong_kind() {
        let outcome = read("[1, 2]");
        assert_eq!(codes(&outcome), vec!["RRS-E1002"]);
        assert_eq!(outcome.diagnostics[0].path, "doc.json:1:1");
    }

    #[test]
    fn a_missing_field_points_at_the_object_that_should_hold_it() {
        let parsed = crate::json::reader::read_json(b"{\n  \"a\": {}\n}", &ImportLimits::default());
        let positions = parsed.map(|parsed| parsed.positions).unwrap_or_default();
        assert_eq!(diagnostic_path("f.json", &positions, "/a/b"), "f.json:2:3#/a/b");
        assert_eq!(diagnostic_path("f.json", &positions, ""), "f.json:1:1");
    }
}
