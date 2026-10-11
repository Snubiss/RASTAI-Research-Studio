//! The Python output: `python/rastai_plugin/rastai_plugin/catalog/generated.py`, read as `rastai_plugin.catalog`
//! (layout rust-control-plane.md 4.9; python.md 1.6). One constant per code, so Python names a code without spelling
//! the string (`RRS_E1003`), and the entries as frozen dataclasses (contracts 12.3: data is a dataclass).

use std::path::Path;

use crate::catalog_source::{CatalogSource, ErrorRecord, TermRecord};
use crate::error::CodegenError;
use crate::literals::{CATALOG_SOURCE_NOTE, GENERATED_NOTICE, json_string_literal, severity_word};
use crate::write_if_changed::write_if_changed;

const CATALOG_MODULE_FILE: &str = "python/rastai_plugin/rastai_plugin/catalog/generated.py";
const INDENT: &str = "    ";
const FIELD_INDENT: &str = "        ";

/// Everything before the constants: the docstring and the two dataclasses.
const MODULE_HEAD: &str = r#""""The error and term catalog as Python data (contracts 10): one constant per code, every entry, every term."""

from dataclasses import dataclass


@dataclass(frozen=True)
class CatalogEntry:
    """One error catalog entry. The fields and their meaning are the catalog JSON's (contracts 10)."""

    code: str
    severity: str  # "error", "warning" or "info"
    title: str
    message: str  # with {snake_case} placeholders a Diagnostic fills
    basic_text: str
    fix_text: str
    fix_command_id: str  # empty when there is no fix button
    docs_anchor: str
    retired: bool


@dataclass(frozen=True)
class TermEntry:
    """One plain-English term definition: the glossary the UI shows (contracts 13)."""

    term_id: str
    title: str
    basic_text: str
    advanced_text: str
    docs_anchor: str
"#;

/// Writes generated.py under `output_root`. Returns how many files changed (0 or 1).
pub fn write_catalog_python(output_root: &Path, catalog: &CatalogSource) -> Result<usize, CodegenError> {
    let changed = write_if_changed(&output_root.join(CATALOG_MODULE_FILE), &render_catalog_python(catalog))?;
    Ok(usize::from(changed))
}

/// The whole module: notice, docstring, dataclasses, one constant per code, then `CATALOG` and `TERMS`.
pub fn render_catalog_python(catalog: &CatalogSource) -> String {
    let mut text = format!("# {GENERATED_NOTICE}\n# {CATALOG_SOURCE_NOTE}\n\n");
    text.push_str(MODULE_HEAD);
    text.push_str("\n\n# One constant per code (RRS_E1003 = \"RRS-E1003\"), for error classes and tests.\n");
    for record in &catalog.errors {
        text.push_str(&format!(
            "{} = {}\n",
            constant_name(&record.code),
            json_string_literal(&record.code)
        ));
    }
    text.push_str("\nCATALOG: tuple[CatalogEntry, ...] = (");
    if !catalog.errors.is_empty() {
        text.push('\n');
        for record in &catalog.errors {
            text.push_str(&render_catalog_entry(record));
        }
    }
    text.push_str(")\n\nTERMS: tuple[TermEntry, ...] = (");
    if !catalog.terms.is_empty() {
        text.push('\n');
        for record in &catalog.terms {
            text.push_str(&render_term_entry(record));
        }
    }
    text.push_str(")\n");
    text
}

/// The Python constant for a code: "RRS-E1003" is `RRS_E1003`. Codes are checked to be "RRS-", a capital letter
/// and four digits (tests/catalog_checks.rs), so this is always a valid name.
pub fn constant_name(code: &str) -> String {
    code.replace('-', "_")
}

fn render_catalog_entry(record: &ErrorRecord) -> String {
    let mut text = format!("{INDENT}CatalogEntry(\n");
    text.push_str(&format!("{FIELD_INDENT}code={},\n", constant_name(&record.code)));
    push_text_field(&mut text, "severity", severity_word(record.severity));
    push_text_field(&mut text, "title", &record.title);
    push_text_field(&mut text, "message", &record.message);
    push_text_field(&mut text, "basic_text", &record.basic_text);
    push_text_field(&mut text, "fix_text", &record.fix_text);
    push_text_field(&mut text, "fix_command_id", &record.fix_command_id);
    push_text_field(&mut text, "docs_anchor", &record.docs_anchor);
    text.push_str(&format!("{FIELD_INDENT}retired={},\n", python_bool(record.retired)));
    text.push_str(&format!("{INDENT}),\n"));
    text
}

fn render_term_entry(record: &TermRecord) -> String {
    let mut text = format!("{INDENT}TermEntry(\n");
    push_text_field(&mut text, "term_id", &record.term_id);
    push_text_field(&mut text, "title", &record.title);
    push_text_field(&mut text, "basic_text", &record.basic_text);
    push_text_field(&mut text, "advanced_text", &record.advanced_text);
    push_text_field(&mut text, "docs_anchor", &record.docs_anchor);
    text.push_str(&format!("{INDENT}),\n"));
    text
}

fn push_text_field(text: &mut String, name: &str, value: &str) {
    text.push_str(&format!("{FIELD_INDENT}{name}={},\n", json_string_literal(value)));
}

fn python_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::Severity;

    fn catalog_with_one_entry() -> CatalogSource {
        CatalogSource {
            errors: vec![ErrorRecord {
                code: "RRS-M4001".to_string(),
                severity: Severity::Info,
                title: "Decode fell back".to_string(),
                message: "Used {mode} instead.".to_string(),
                basic_text: "It still works, more slowly.".to_string(),
                fix_text: String::new(),
                fix_command_id: String::new(),
                docs_anchor: "decode-fallback".to_string(),
                retired: true,
                source_file: "catalog/errors/RRS-M.json".to_string(),
            }],
            terms: Vec::new(),
        }
    }

    #[test]
    fn constant_names_replace_the_dash() {
        assert_eq!(constant_name("RRS-M4001"), "RRS_M4001");
    }

    #[test]
    fn an_empty_catalog_has_empty_tuples_and_no_constants() {
        let text = render_catalog_python(&CatalogSource::default());
        assert!(text.starts_with(&format!("# {GENERATED_NOTICE}\n")));
        assert!(text.contains("\n@dataclass(frozen=True)\nclass CatalogEntry:\n"));
        assert!(text.ends_with(
            "for error classes and tests.\n\nCATALOG: tuple[CatalogEntry, ...] = ()\n\nTERMS: tuple[TermEntry, ...] = ()\n"
        ));
    }

    #[test]
    fn an_entry_uses_its_constant_and_python_literals() {
        let text = render_catalog_python(&catalog_with_one_entry());
        let expected = r#"RRS_M4001 = "RRS-M4001"

CATALOG: tuple[CatalogEntry, ...] = (
    CatalogEntry(
        code=RRS_M4001,
        severity="info",
        title="Decode fell back",
        message="Used {mode} instead.",
        basic_text="It still works, more slowly.",
        fix_text="",
        fix_command_id="",
        docs_anchor="decode-fallback",
        retired=True,
    ),
)

TERMS: tuple[TermEntry, ...] = ()
"#;
        assert!(text.ends_with(expected), "{text}");
    }
}
