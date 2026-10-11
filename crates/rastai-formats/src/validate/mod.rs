//! `read_document`, the one entry point for untrusted document bytes (layout 4.3, 4.4): layer 1 (syntax), the
//! migration chain, layer 2 (structure), then the typed document and layer 3 (meaning), with every diagnostic
//! collected in one pass. Layer 4 (environment) is a separate call, `check_environment`.
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
use crate::documents::{DOCUMENT_TYPES, Document, DocumentType, document_from_value};
use crate::json::positions::PositionMap;
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
/// negative_step.json).
const CODE_WRONG_KIND: &str = "RRS-E1002";

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
            let values = [("field", ""), ("expected", type_name), ("found", reason.as_str())];
            diagnostics.push(diagnostic_at(CODE_WRONG_KIND, &values, file_name));
            None
        }
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
    fn a_root_that_is_not_an_object_is_one_wrong_kind() {
        let outcome = read("[1, 2]");
        assert_eq!(codes(&outcome), vec!["RRS-E1002"]);
        assert_eq!(outcome.diagnostics[0].path, "doc.json:1:1");
    }
}
