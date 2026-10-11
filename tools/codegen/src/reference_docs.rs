//! The reference docs output: `docs/reference/errors.md`, every catalog code grouped by prefix in contracts 10.1's
//! order (layout rust-control-plane.md 4.9). commands.md and settings.md join with rastai-formats-codegen (M1-16).
//!
//! Each entry gets an HTML anchor named by its `docs_anchor`, so a fix link can point at it.

use std::path::Path;

use crate::catalog_ranges::{CATALOG_PREFIXES, CatalogPrefix, split_code};
use crate::catalog_source::{CatalogSource, ErrorRecord};
use crate::error::CodegenError;
use crate::literals::{CATALOG_SOURCE_NOTE, GENERATED_NOTICE, severity_word};
use crate::write_if_changed::write_if_changed;

const ERRORS_REFERENCE_FILE: &str = "docs/reference/errors.md";
const EMPTY_PREFIX_LINE: &str = "No codes yet.\n";

const INTRODUCTION: &str = "# Error catalog

Every code Rastai reports, from the catalog sources. A code means one thing forever: it is never reused, and a code
that is no longer raised stays here marked retired (contracts §10). To add one, list it in contracts §10.3 first,
then add its entry to the owner's catalog file.
";

/// Writes errors.md under `output_root`. Returns how many files changed (0 or 1).
pub fn write_errors_reference(output_root: &Path, catalog: &CatalogSource) -> Result<usize, CodegenError> {
    let changed = write_if_changed(
        &output_root.join(ERRORS_REFERENCE_FILE),
        &render_errors_reference(catalog),
    )?;
    Ok(usize::from(changed))
}

/// The whole page: notice, introduction, then one section per prefix of contracts 10.1. A code whose prefix has no
/// row there still appears, in a last section, so the page never hides an entry; the catalog checks fail on it.
pub fn render_errors_reference(catalog: &CatalogSource) -> String {
    let mut text = format!("<!-- {GENERATED_NOTICE} -->\n<!-- {CATALOG_SOURCE_NOTE} -->\n\n{INTRODUCTION}");
    for row in CATALOG_PREFIXES {
        let entries: Vec<&ErrorRecord> = catalog
            .errors
            .iter()
            .filter(|record| prefix_of(record) == row.prefix)
            .collect();
        text.push_str(&render_prefix_section(row, &entries));
    }
    let unowned: Vec<&ErrorRecord> = catalog.errors.iter().filter(|record| !has_owner(record)).collect();
    if !unowned.is_empty() {
        text.push_str("\n## Codes without an owner\n\nThese codes match no prefix of contracts §10.1.\n");
        for record in unowned {
            text.push_str(&render_entry(record));
        }
    }
    text
}

fn render_prefix_section(row: &CatalogPrefix, entries: &[&ErrorRecord]) -> String {
    let mut text = format!("\n## {}: {}\n\n", row.prefix, row.area);
    text.push_str(&format!("Source: `{}`.\n", row.source_file));
    if entries.is_empty() {
        text.push('\n');
        text.push_str(EMPTY_PREFIX_LINE);
    }
    for record in entries {
        text.push_str(&render_entry(record));
    }
    text
}

fn render_entry(record: &ErrorRecord) -> String {
    let mut text = String::from("\n");
    if !record.docs_anchor.is_empty() {
        text.push_str(&format!("<a id=\"{}\"></a>\n\n", escape_attribute(&record.docs_anchor)));
    }
    text.push_str(&format!("### {}: {}\n\n", record.code, escape_html(&record.title)));
    text.push_str(&format!("- **Severity:** {}\n", severity_word(record.severity)));
    if record.retired {
        text.push_str("- **Retired:** no longer raised; the code stays reserved.\n");
    }
    text.push_str(&format!("- **Message:** {}\n", escape_html(&record.message)));
    text.push_str(&format!("- **In plain words:** {}\n", escape_html(&record.basic_text)));
    if !record.fix_text.is_empty() {
        text.push_str(&format!("- **Fix:** {}\n", escape_html(&record.fix_text)));
    }
    if !record.fix_command_id.is_empty() {
        text.push_str(&format!("- **Fix command:** `{}`\n", record.fix_command_id));
    }
    text
}

fn prefix_of(record: &ErrorRecord) -> &str {
    split_code(&record.code).map_or("", |(prefix, _)| prefix)
}

fn has_owner(record: &ErrorRecord) -> bool {
    let prefix = prefix_of(record);
    CATALOG_PREFIXES.iter().any(|row| row.prefix == prefix)
}

/// Catalog text is plain text, but Markdown passes HTML through: "<file>" in a message would vanish as an unknown
/// tag. Escaping the three HTML characters keeps the text as written. Line breaks become spaces, so a multi-line
/// message stays inside its list item.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', " ")
}

/// Text inside a double-quoted HTML attribute: the HTML characters and the quote itself. catalog_checks already
/// refuses an anchor that isn't a plain slug, but the page is written before the tests run, so it must stay
/// well-formed whatever the source says.
fn escape_attribute(text: &str) -> String {
    escape_html(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::Severity;

    fn record(code: &str, retired: bool) -> ErrorRecord {
        ErrorRecord {
            code: code.to_string(),
            severity: Severity::Error,
            title: "Not enough free disk space".to_string(),
            message: "{operation} needs space on <volume>.".to_string(),
            basic_text: "Your disk is almost full.".to_string(),
            fix_text: "Free up space.".to_string(),
            fix_command_id: "storage.gc".to_string(),
            docs_anchor: "storage-disk-full".to_string(),
            retired,
            source_file: "catalog/errors/RRS-S.json".to_string(),
        }
    }

    #[test]
    fn an_empty_catalog_lists_every_prefix_with_no_codes() {
        let text = render_errors_reference(&CatalogSource::default());
        assert!(text.starts_with(&format!("<!-- {GENERATED_NOTICE} -->\n")));
        for row in CATALOG_PREFIXES {
            assert!(
                text.contains(&format!("\n## {}: {}\n", row.prefix, row.area)),
                "{}",
                row.prefix
            );
        }
        assert_eq!(text.matches(EMPTY_PREFIX_LINE).count(), CATALOG_PREFIXES.len());
        assert!(!text.contains("without an owner"));
    }

    #[test]
    fn an_entry_has_its_anchor_heading_and_fields() {
        let catalog = CatalogSource {
            errors: vec![record("RRS-S3001", true)],
            terms: Vec::new(),
        };
        let text = render_errors_reference(&catalog);
        let expected = "## RRS-S: Storage and durability

Source: `catalog/errors/RRS-S.json`.

<a id=\"storage-disk-full\"></a>

### RRS-S3001: Not enough free disk space

- **Severity:** error
- **Retired:** no longer raised; the code stays reserved.
- **Message:** {operation} needs space on &lt;volume&gt;.
- **In plain words:** Your disk is almost full.
- **Fix:** Free up space.
- **Fix command:** `storage.gc`
";
        assert!(text.contains(expected), "{text}");
        assert_eq!(text.matches(EMPTY_PREFIX_LINE).count(), CATALOG_PREFIXES.len() - 1);
    }

    #[test]
    fn a_quote_in_an_anchor_cannot_break_the_attribute() {
        let mut entry = record("RRS-S3001", false);
        entry.docs_anchor = "disk\" onclick=\"x".to_string();
        let catalog = CatalogSource {
            errors: vec![entry],
            terms: Vec::new(),
        };
        let text = render_errors_reference(&catalog);
        assert!(text.contains("<a id=\"disk&quot; onclick=&quot;x\"></a>"), "{text}");
    }

    #[test]
    fn a_code_without_an_owner_is_still_listed() {
        let catalog = CatalogSource {
            errors: vec![record("RRS-Z0001", false)],
            terms: Vec::new(),
        };
        let text = render_errors_reference(&catalog);
        assert!(text.contains("## Codes without an owner"));
        assert!(text.contains("### RRS-Z0001: "));
    }
}
