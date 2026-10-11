//! The JavaScript output: `packages/catalog/src/generated/catalog.js`, the module behind `@rastai/catalog`
//! (layout rust-control-plane.md 4.9). Plain JavaScript with JSDoc, never TypeScript (ADR-0036): one `@typedef` for an
//! error entry, one for a term, and the two arrays. Fields keep the catalog JSON's snake_case (contracts 12.4).

use std::path::Path;

use crate::catalog_source::{CatalogSource, ErrorRecord, TermRecord};
use crate::error::CodegenError;
use crate::literals::{CATALOG_SOURCE_NOTE, GENERATED_NOTICE, json_string_literal, severity_word};
use crate::write_if_changed::write_if_changed;

const CATALOG_MODULE_FILE: &str = "packages/catalog/src/generated/catalog.js";
const INDENT: &str = "  ";
const FIELD_INDENT: &str = "    ";

/// The JSDoc for one error entry. Each property says what the field means, since the generated file is what an
/// editor shows when someone hovers a field.
const CATALOG_ENTRY_TYPEDEF: &str = "/**
 * One error catalog entry (contracts 10).
 * @typedef {object} CatalogEntry
 * @property {string} code The code, such as \"RRS-E1003\". A code is never reused; a retired one stays listed.
 * @property {\"error\" | \"warning\" | \"info\"} severity
 * @property {string} title A short title, shown after the code.
 * @property {string} message The full message, with {snake_case} placeholders a Diagnostic fills.
 * @property {string} basic_text The plain-English text Basic mode shows.
 * @property {string} fix_text What to do about it.
 * @property {string} fix_command_id The command the fix button runs; empty when there is none.
 * @property {string} docs_anchor The anchor of the entry in the reference docs.
 * @property {boolean} retired True for a code that is no longer raised.
 */
";

const TERM_ENTRY_TYPEDEF: &str = "/**
 * One plain-English term definition: the glossary the UI shows (contracts 13).
 * @typedef {object} TermEntry
 * @property {string} term_id The term's id, such as \"logit_lens\".
 * @property {string} title The term as the UI writes it.
 * @property {string} basic_text The definition Basic mode shows.
 * @property {string} advanced_text The longer definition Advanced mode shows.
 * @property {string} docs_anchor The anchor of the term in the reference docs.
 */
";

/// Writes catalog.js under `output_root`. Returns how many files changed (0 or 1).
pub fn write_catalog_javascript(output_root: &Path, catalog: &CatalogSource) -> Result<usize, CodegenError> {
    let changed = write_if_changed(
        &output_root.join(CATALOG_MODULE_FILE),
        &render_catalog_javascript(catalog),
    )?;
    Ok(usize::from(changed))
}

/// The whole module: notice, the two typedefs, then `CATALOG` and `TERMS`.
pub fn render_catalog_javascript(catalog: &CatalogSource) -> String {
    let mut text = format!("// {GENERATED_NOTICE}\n// {CATALOG_SOURCE_NOTE}\n\n");
    text.push_str(CATALOG_ENTRY_TYPEDEF);
    text.push('\n');
    text.push_str(TERM_ENTRY_TYPEDEF);
    text.push_str("\n/** @type {CatalogEntry[]} */\nexport const CATALOG = [");
    if !catalog.errors.is_empty() {
        text.push('\n');
        for record in &catalog.errors {
            text.push_str(&render_catalog_entry(record));
        }
    }
    text.push_str("];\n\n/** @type {TermEntry[]} */\nexport const TERMS = [");
    if !catalog.terms.is_empty() {
        text.push('\n');
        for record in &catalog.terms {
            text.push_str(&render_term_entry(record));
        }
    }
    text.push_str("];\n");
    text
}

fn render_catalog_entry(record: &ErrorRecord) -> String {
    let mut text = format!("{INDENT}{{\n");
    push_text_field(&mut text, "code", &record.code);
    push_text_field(&mut text, "severity", severity_word(record.severity));
    push_text_field(&mut text, "title", &record.title);
    push_text_field(&mut text, "message", &record.message);
    push_text_field(&mut text, "basic_text", &record.basic_text);
    push_text_field(&mut text, "fix_text", &record.fix_text);
    push_text_field(&mut text, "fix_command_id", &record.fix_command_id);
    push_text_field(&mut text, "docs_anchor", &record.docs_anchor);
    text.push_str(&format!("{FIELD_INDENT}retired: {},\n", record.retired));
    text.push_str(&format!("{INDENT}}},\n"));
    text
}

fn render_term_entry(record: &TermRecord) -> String {
    let mut text = format!("{INDENT}{{\n");
    push_text_field(&mut text, "term_id", &record.term_id);
    push_text_field(&mut text, "title", &record.title);
    push_text_field(&mut text, "basic_text", &record.basic_text);
    push_text_field(&mut text, "advanced_text", &record.advanced_text);
    push_text_field(&mut text, "docs_anchor", &record.docs_anchor);
    text.push_str(&format!("{INDENT}}},\n"));
    text
}

fn push_text_field(text: &mut String, name: &str, value: &str) {
    text.push_str(&format!("{FIELD_INDENT}{name}: {},\n", json_string_literal(value)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::Severity;

    fn catalog_with_one_of_each() -> CatalogSource {
        CatalogSource {
            errors: vec![ErrorRecord {
                code: "RRS-E0003".to_string(),
                severity: Severity::Error,
                title: "Duplicate key in a JSON object".to_string(),
                message: "The key \"{key}\" appears twice.".to_string(),
                basic_text: "A name is written twice.".to_string(),
                fix_text: "Remove one.".to_string(),
                fix_command_id: String::new(),
                docs_anchor: "json-duplicate-key".to_string(),
                retired: false,
                source_file: "catalog/errors/RRS-E.json".to_string(),
            }],
            terms: vec![TermRecord {
                term_id: "logit_lens".to_string(),
                title: "Logit lens".to_string(),
                basic_text: "What each layer would predict.".to_string(),
                advanced_text: "The residual stream, unembedded.".to_string(),
                docs_anchor: "logit-lens".to_string(),
                source_file: "catalog/terms/terms.json".to_string(),
            }],
        }
    }

    #[test]
    fn an_empty_catalog_exports_two_empty_typed_arrays() {
        let text = render_catalog_javascript(&CatalogSource::default());
        assert!(text.starts_with(&format!("// {GENERATED_NOTICE}\n")));
        assert!(text.contains("@typedef {object} CatalogEntry\n"));
        assert!(text.contains("@typedef {object} TermEntry\n"));
        assert!(text.ends_with(
            "/** @type {CatalogEntry[]} */\nexport const CATALOG = [];\n\n/** @type {TermEntry[]} */\nexport const TERMS = [];\n"
        ));
    }

    #[test]
    fn entries_are_object_literals_with_snake_case_fields() {
        let text = render_catalog_javascript(&catalog_with_one_of_each());
        let expected = r#"export const CATALOG = [
  {
    code: "RRS-E0003",
    severity: "error",
    title: "Duplicate key in a JSON object",
    message: "The key \"{key}\" appears twice.",
    basic_text: "A name is written twice.",
    fix_text: "Remove one.",
    fix_command_id: "",
    docs_anchor: "json-duplicate-key",
    retired: false,
  },
];

/** @type {TermEntry[]} */
export const TERMS = [
  {
    term_id: "logit_lens",
    title: "Logit lens",
    basic_text: "What each layer would predict.",
    advanced_text: "The residual stream, unembedded.",
    docs_anchor: "logit-lens",
  },
];
"#;
        assert!(text.ends_with(expected), "{text}");
    }

    #[test]
    fn no_typescript_syntax_is_written() {
        let text = render_catalog_javascript(&catalog_with_one_of_each());
        for typescript_only in ["interface ", "export type ", ": string", " as const", "readonly "] {
            assert!(!text.contains(typescript_only), "{typescript_only}");
        }
    }
}
