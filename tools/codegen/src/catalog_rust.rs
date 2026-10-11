//! The Rust output: `crates/rastai-diag/src/generated/`, the compiled catalog `rastai_diag::catalog::find_entry`
//! and `rastai_diag::terms::find_term` search (layout rust-control-plane.md 3.1).
//!
//! The codegen owns the whole folder, `mod.rs` included, so check-generated can compare it as one unit. `mod.rs`
//! marks both tables `#[rustfmt::skip]`: they are compared byte for byte with a fresh run, so they stay exactly as
//! written here, whatever a later rustfmt would prefer.

use std::path::Path;

use rastai_diag::diagnostic::Severity;

use crate::catalog_source::{CatalogSource, ErrorRecord, TermRecord};
use crate::error::CodegenError;
use crate::literals::{CATALOG_SOURCE_NOTE, GENERATED_NOTICE, rust_string_literal};
use crate::write_if_changed::write_if_changed;

const GENERATED_FOLDER: &str = "crates/rastai-diag/src/generated";
const MOD_FILE: &str = "mod.rs";
const CATALOG_TABLE_FILE: &str = "catalog_table.rs";
const TERMS_TABLE_FILE: &str = "terms_table.rs";
const INDENT: &str = "    ";
const FIELD_INDENT: &str = "        ";

/// Writes the three files of rastai-diag's generated folder under `output_root`. Returns how many changed.
pub fn write_catalog_rust(output_root: &Path, catalog: &CatalogSource) -> Result<usize, CodegenError> {
    let folder = output_root.join(GENERATED_FOLDER);
    let outputs = [
        (MOD_FILE, render_generated_mod()),
        (CATALOG_TABLE_FILE, render_catalog_table(&catalog.errors)),
        (TERMS_TABLE_FILE, render_terms_table(&catalog.terms)),
    ];
    let mut changed_count = 0;
    for (file_name, contents) in outputs {
        if write_if_changed(&folder.join(file_name), &contents)? {
            changed_count += 1;
        }
    }
    Ok(changed_count)
}

/// `generated/mod.rs`: the two `pub mod` lines.
pub fn render_generated_mod() -> String {
    let mut text = notice_lines();
    text.push_str("//! The compiled error and term catalog (layout rust-control-plane.md 3.1).\n");
    text.push_str(
        "//! rustfmt skips the tables: check-generated compares them byte for byte with a fresh codegen run.\n\n",
    );
    text.push_str("#[rustfmt::skip]\npub mod catalog_table;\n#[rustfmt::skip]\npub mod terms_table;\n");
    text
}

/// `generated/catalog_table.rs`: `CATALOG`, one `CatalogEntry` per error record, in the records' order.
pub fn render_catalog_table(errors: &[ErrorRecord]) -> String {
    let mut text = notice_lines();
    text.push_str("use crate::catalog::CatalogEntry;\n");
    if errors.is_empty() {
        text.push_str("\npub const CATALOG: &[CatalogEntry] = &[];\n");
        return text;
    }
    // Only a non-empty table names a severity; an unused import would be a warning, and warnings fail `just check`.
    text.push_str("use crate::diagnostic::Severity;\n\npub const CATALOG: &[CatalogEntry] = &[\n");
    for record in errors {
        text.push_str(&render_catalog_entry(record));
    }
    text.push_str("];\n");
    text
}

/// `generated/terms_table.rs`: `TERMS`, one `TermEntry` per term record.
pub fn render_terms_table(terms: &[TermRecord]) -> String {
    let mut text = notice_lines();
    text.push_str("use crate::terms::TermEntry;\n");
    if terms.is_empty() {
        text.push_str("\npub const TERMS: &[TermEntry] = &[];\n");
        return text;
    }
    text.push_str("\npub const TERMS: &[TermEntry] = &[\n");
    for record in terms {
        text.push_str(&render_term_entry(record));
    }
    text.push_str("];\n");
    text
}

fn render_catalog_entry(record: &ErrorRecord) -> String {
    let mut text = format!("{INDENT}CatalogEntry {{\n");
    push_text_field(&mut text, "code", &record.code);
    text.push_str(&format!(
        "{FIELD_INDENT}severity: {},\n",
        severity_path(record.severity)
    ));
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
    let mut text = format!("{INDENT}TermEntry {{\n");
    push_text_field(&mut text, "term_id", &record.term_id);
    push_text_field(&mut text, "title", &record.title);
    push_text_field(&mut text, "basic_text", &record.basic_text);
    push_text_field(&mut text, "advanced_text", &record.advanced_text);
    push_text_field(&mut text, "docs_anchor", &record.docs_anchor);
    text.push_str(&format!("{INDENT}}},\n"));
    text
}

fn push_text_field(text: &mut String, name: &str, value: &str) {
    text.push_str(&format!("{FIELD_INDENT}{name}: {},\n", rust_string_literal(value)));
}

fn severity_path(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Severity::Error",
        Severity::Warning => "Severity::Warning",
        Severity::Info => "Severity::Info",
    }
}

fn notice_lines() -> String {
    format!("// {GENERATED_NOTICE}\n// {CATALOG_SOURCE_NOTE}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk_full_record() -> ErrorRecord {
        ErrorRecord {
            code: "RRS-S3001".to_string(),
            severity: Severity::Warning,
            title: "Not enough free disk space".to_string(),
            message: "{operation} needs \"{needed_bytes}\" bytes.".to_string(),
            basic_text: "Your disk is almost full.".to_string(),
            fix_text: "Free up space.\nThen retry.".to_string(),
            fix_command_id: "storage.gc".to_string(),
            docs_anchor: "storage-disk-full".to_string(),
            retired: true,
            source_file: "catalog/errors/RRS-S.json".to_string(),
        }
    }

    #[test]
    fn every_file_starts_with_the_generated_notice() {
        let notice = format!("// {GENERATED_NOTICE}\n");
        assert!(render_generated_mod().starts_with(&notice));
        assert!(render_catalog_table(&[]).starts_with(&notice));
        assert!(render_terms_table(&[]).starts_with(&notice));
    }

    #[test]
    fn an_empty_catalog_is_an_empty_table_without_the_severity_import() {
        let text = render_catalog_table(&[]);
        assert!(text.ends_with("use crate::catalog::CatalogEntry;\n\npub const CATALOG: &[CatalogEntry] = &[];\n"));
        assert!(!text.contains("Severity"));
        assert!(render_terms_table(&[]).ends_with("\npub const TERMS: &[TermEntry] = &[];\n"));
    }

    #[test]
    fn an_entry_is_written_field_by_field_with_escaped_text() {
        let text = render_catalog_table(&[disk_full_record()]);
        let expected_entry = r#"pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        code: "RRS-S3001",
        severity: Severity::Warning,
        title: "Not enough free disk space",
        message: "{operation} needs \"{needed_bytes}\" bytes.",
        basic_text: "Your disk is almost full.",
        fix_text: "Free up space.\nThen retry.",
        fix_command_id: "storage.gc",
        docs_anchor: "storage-disk-full",
        retired: true,
    },
];
"#;
        assert!(text.contains("use crate::diagnostic::Severity;\n"));
        assert!(text.ends_with(expected_entry), "{text}");
    }

    #[test]
    fn a_term_is_written_field_by_field() {
        let term = TermRecord {
            term_id: "logit_lens".to_string(),
            title: "Logit lens".to_string(),
            basic_text: "What each layer would predict.".to_string(),
            advanced_text: "The residual stream, unembedded.".to_string(),
            docs_anchor: "logit-lens".to_string(),
            source_file: "catalog/terms/terms.json".to_string(),
        };
        let expected = r#"pub const TERMS: &[TermEntry] = &[
    TermEntry {
        term_id: "logit_lens",
        title: "Logit lens",
        basic_text: "What each layer would predict.",
        advanced_text: "The residual stream, unembedded.",
        docs_anchor: "logit-lens",
    },
];
"#;
        assert!(render_terms_table(&[term]).ends_with(expected));
    }

    #[test]
    fn mod_file_declares_both_tables_and_skips_rustfmt() {
        let text = render_generated_mod();
        assert!(text.ends_with("#[rustfmt::skip]\npub mod catalog_table;\n#[rustfmt::skip]\npub mod terms_table;\n"));
    }
}
