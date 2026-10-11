//! The migration runner (contracts 5.1, ADR-0032 decision 5): read `schema_version`, run each step up to the
//! type's current version, and mark a file from a newer version read-only (`RRS-E4001`). It runs after layer 1
//! and before layer 2, so layer 2 checks the upgraded value against the current description.
//!
//! A missing or failed step (`RRS-E4002`) stops the read: layer 2 against the current description would only
//! repeat the same problem field by field.

pub mod chain_table;

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use serde_json::Value;

use crate::documents::DocumentType;
use crate::header::SCHEMA_VERSION_KEY;
use crate::json::positions::PositionMap;
use crate::json::positions::diagnostic_path;
use crate::migrate::chain_table::{MigrationStep, find_step};

const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";
const CODE_NEWER_VERSION: &str = "RRS-E4001";
const CODE_MIGRATION_FAILED: &str = "RRS-E4002";

/// Schema versions start at 1 (contracts 5.1).
const FIRST_SCHEMA_VERSION: u32 = 1;
/// What RRS-E1004's message says a schema version must be.
const SCHEMA_VERSION_EXPECTED: &str = "a whole number of 1 or more";
const NO_STEP_REASON: &str = "no migration step from this version";

/// What the runner did.
#[derive(Debug, Clone, PartialEq)]
pub struct MigrationResult {
    /// The value at the current version; unchanged when no step ran, or when a step failed.
    pub value: Value,
    /// The version the file had when at least one step ran; 0 otherwise.
    pub migrated_from_version: u32,
    /// The file is from a newer version: it opens, but saving it would lose what this version doesn't know.
    pub read_only: bool,
    /// A step was missing or failed; the read stops.
    pub failed: bool,
}

/// What `migrate_to_current` needs besides the value.
pub struct MigrationInput<'a> {
    pub document_type: &'a DocumentType,
    pub steps: &'a [MigrationStep],
    pub file_name: &'a str,
    pub positions: &'a PositionMap,
}

/// Upgrades a document to its type's current version. A `schema_version` that is missing or not an integer is
/// left to layer 2, which reports it; one that is an integer but not a version is `RRS-E1004` here.
pub fn migrate_to_current(value: Value, input: &MigrationInput, diagnostics: &mut Vec<Diagnostic>) -> MigrationResult {
    let unchanged = |value: Value| MigrationResult {
        value,
        migrated_from_version: 0,
        read_only: false,
        failed: false,
    };
    let Some(raw_version) = value
        .get(SCHEMA_VERSION_KEY)
        .filter(|version| version.is_i64() || version.is_u64())
    else {
        return unchanged(value);
    };
    let version = match raw_version.as_u64().and_then(|number| u32::try_from(number).ok()) {
        Some(version) if version >= FIRST_SCHEMA_VERSION => version,
        _ => {
            let found = raw_version.to_string();
            let values = [("found", found.as_str()), ("expected", SCHEMA_VERSION_EXPECTED)];
            diagnostics.push(version_diagnostic(CODE_VALUE_NOT_ALLOWED, &values, input));
            return unchanged(value);
        }
    };
    let current_version = input.document_type.schema_version;
    if version > current_version {
        let values = [
            ("version", version.to_string()),
            ("current_version", current_version.to_string()),
        ];
        diagnostics.push(version_diagnostic(CODE_NEWER_VERSION, &borrow_values(&values), input));
        return MigrationResult {
            read_only: true,
            ..unchanged(value)
        };
    }
    if version == current_version {
        return unchanged(value);
    }
    run_chain(value, version, input, diagnostics)
}

/// Runs each step from `from_version` up to the current version.
fn run_chain(
    value: Value,
    from_version: u32,
    input: &MigrationInput,
    diagnostics: &mut Vec<Diagnostic>,
) -> MigrationResult {
    let type_name = input.document_type.type_name;
    let mut upgraded = value.clone();
    let mut version = from_version;
    while version < input.document_type.schema_version {
        let step_result = match find_step(input.steps, type_name, version) {
            Some(step) => (step.migrate)(&upgraded),
            None => Err(NO_STEP_REASON.to_string()),
        };
        match step_result {
            Ok(next) => upgraded = next,
            Err(reason) => {
                let values = [("from_version", version.to_string()), ("reason", reason)];
                diagnostics.push(version_diagnostic(
                    CODE_MIGRATION_FAILED,
                    &borrow_values(&values),
                    input,
                ));
                return failed_result(value);
            }
        }
        version += 1;
        set_schema_version(&mut upgraded, version);
    }
    MigrationResult {
        value: upgraded,
        migrated_from_version: from_version,
        read_only: false,
        failed: false,
    }
}

fn failed_result(value: Value) -> MigrationResult {
    MigrationResult {
        value,
        migrated_from_version: 0,
        read_only: false,
        failed: true,
    }
}

/// Steps change fields only; the version number is the runner's job, so no step can forget it.
fn set_schema_version(value: &mut Value, version: u32) {
    if let Value::Object(members) = value {
        members.insert(SCHEMA_VERSION_KEY.to_string(), Value::from(version));
    }
}

/// Every migration diagnostic points at `schema_version`.
fn version_diagnostic(code: &str, values: &[(&str, &str)], input: &MigrationInput) -> Diagnostic {
    let pointer = format!("/{SCHEMA_VERSION_KEY}");
    diagnostic_at(
        code,
        values,
        &diagnostic_path(input.file_name, input.positions, &pointer),
    )
}

/// `diagnostic_at` takes `&str` pairs; this lends them from owned strings.
fn borrow_values<'a>(values: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    values.iter().map(|(name, text)| (*name, text.as_str())).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::json::reader::read_json;
    use crate::limits::ImportLimits;
    use serde_json::json;

    const SAMPLE_TYPE: DocumentType = DocumentType {
        type_name: "test.sample",
        schema_version: 3,
        file_extensions: &[],
        description_name: "Sample",
    };

    // v1 → v2 renames `title` to `name`; v2 → v3 refuses a document marked `broken`.
    fn rename_title(value: &Value) -> Result<Value, String> {
        let mut upgraded = value.clone();
        if let Some(members) = upgraded.as_object_mut()
            && let Some(title) = members.remove("title")
        {
            members.insert("name".to_string(), title);
        }
        Ok(upgraded)
    }

    fn refuse_broken(value: &Value) -> Result<Value, String> {
        if value.get("broken").is_some() {
            return Err("broken documents can't be upgraded".to_string());
        }
        Ok(value.clone())
    }

    const SAMPLE_STEPS: &[MigrationStep] = &[
        MigrationStep {
            type_name: "test.sample",
            from_version: 2,
            migrate: refuse_broken,
        },
        MigrationStep {
            type_name: "test.sample",
            from_version: 1,
            migrate: rename_title,
        },
    ];

    fn migrate(text: &str, steps: &[MigrationStep]) -> (MigrationResult, Vec<Diagnostic>) {
        let parsed = read_json(text.as_bytes(), &ImportLimits::default()).unwrap();
        let input = MigrationInput {
            document_type: &SAMPLE_TYPE,
            steps,
            file_name: "a.sample",
            positions: &parsed.positions,
        };
        let mut diagnostics = Vec::new();
        let result = migrate_to_current(parsed.value, &input, &mut diagnostics);
        (result, diagnostics)
    }

    fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|item| item.code.as_str()).collect()
    }

    #[test]
    fn an_old_file_runs_every_step_and_gets_the_current_version() {
        let (result, diagnostics) = migrate(r#"{"schema_version": 1, "title": "t"}"#, SAMPLE_STEPS);
        assert!(diagnostics.is_empty());
        assert_eq!(result.value, json!({"schema_version": 3, "name": "t"}));
        assert_eq!(result.migrated_from_version, 1);
        assert!(!result.read_only && !result.failed);
    }

    #[test]
    fn a_current_file_is_unchanged() {
        let (result, diagnostics) = migrate(r#"{"schema_version": 3, "title": "t"}"#, SAMPLE_STEPS);
        assert!(diagnostics.is_empty());
        assert_eq!(result.value, json!({"schema_version": 3, "title": "t"}));
        assert_eq!(result.migrated_from_version, 0);
    }

    #[test]
    fn a_newer_file_opens_read_only() {
        let (result, diagnostics) = migrate("{\n  \"schema_version\": 4\n}", SAMPLE_STEPS);
        assert_eq!(codes(&diagnostics), vec!["RRS-E4001"]);
        assert_eq!(diagnostics[0].path, "a.sample:2:3#/schema_version");
        assert!(result.read_only && !result.failed);
    }

    #[test]
    fn a_failed_or_missing_step_stops_with_e4002() {
        let (result, diagnostics) = migrate(r#"{"schema_version": 1, "broken": true}"#, SAMPLE_STEPS);
        assert_eq!(codes(&diagnostics), vec!["RRS-E4002"]);
        assert!(result.failed);
        assert_eq!(result.value, json!({"schema_version": 1, "broken": true}));
        let (result, diagnostics) = migrate(r#"{"schema_version": 1}"#, chain_table::MIGRATION_STEPS);
        assert_eq!(codes(&diagnostics), vec!["RRS-E4002"]);
        assert!(result.failed);
    }

    #[test]
    fn a_version_that_is_not_a_version_is_e1004_and_a_missing_one_is_left_to_layer_two() {
        for text in [
            r#"{"schema_version": 0}"#,
            r#"{"schema_version": -1}"#,
            r#"{"schema_version": 4294967296}"#,
        ] {
            let (result, diagnostics) = migrate(text, SAMPLE_STEPS);
            assert_eq!(codes(&diagnostics), vec!["RRS-E1004"], "{text}");
            // A placeholder without a value would stay visible as `{name}`.
            assert!(!diagnostics[0].message.contains('{'), "{}", diagnostics[0].message);
            assert!(!result.failed);
        }
        for text in [r#"{}"#, r#"{"schema_version": "1"}"#, r#"{"schema_version": 1.5}"#] {
            let (_, diagnostics) = migrate(text, SAMPLE_STEPS);
            assert!(diagnostics.is_empty(), "{text}");
        }
    }
}
