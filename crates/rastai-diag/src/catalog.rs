//! Catalog lookup: each code's severity, title, message template, plain-English text and fix
//! (contracts section 10, layout: rust-control-plane.md section 2.6).
//!
//! The entries live in `generated/catalog_table.rs`, written by `rastai-catalog-codegen` from `catalog/errors/`.
//! Third-party `PLG-` codes and `RuntimeCatalog` arrive with the plugin host (M8, layout 3.7).

use crate::diagnostic::Severity;
use crate::generated::catalog_table::CATALOG;

const PLACEHOLDER_OPEN: char = '{';
const PLACEHOLDER_CLOSE: char = '}';

/// One catalog entry, as `rastai-catalog-codegen` writes it. Every text is fixed at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    pub code: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    /// The message template, with `{snake_case}` placeholders.
    pub message: &'static str,
    pub basic_text: &'static str,
    pub fix_text: &'static str,
    pub fix_command_id: &'static str,
    pub docs_anchor: &'static str,
    pub retired: bool,
}

/// The entry for a code, or None when the catalog doesn't have it.
/// A linear scan is enough: the catalog holds a few hundred entries and lookups happen only when something failed.
pub fn find_entry(code: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.code == code)
}

/// Replaces each `{name}` with its value. A placeholder without a value stays visible as `{name}`, so a missing
/// value shows up in the message instead of silently vanishing. Values are inserted as they are: a value that itself
/// contains `{name}` is never filled again.
pub fn fill_placeholders(template: &str, values: &[(&str, &str)]) -> String {
    let mut filled = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open_index) = rest.find(PLACEHOLDER_OPEN) {
        filled.push_str(&rest[..open_index]);
        let after_open = &rest[open_index + 1..];
        match placeholder_name(after_open) {
            Some(name) => {
                push_value_or_placeholder(&mut filled, name, values);
                // Skip the name and its closing brace.
                rest = &after_open[name.len() + 1..];
            }
            None => {
                // A lone `{` (or `{not a name}`) is ordinary text.
                filled.push(PLACEHOLDER_OPEN);
                rest = after_open;
            }
        }
    }
    filled.push_str(rest);
    filled
}

/// The name of the placeholder that starts right after a `{`, if the text up to the next `}` is a snake_case name.
fn placeholder_name(after_open: &str) -> Option<&str> {
    let close_index = after_open.find(PLACEHOLDER_CLOSE)?;
    let name = &after_open[..close_index];
    if is_snake_case_name(name) { Some(name) } else { None }
}

fn is_snake_case_name(name: &str) -> bool {
    let starts_with_letter = name.chars().next().is_some_and(|first| first.is_ascii_lowercase());
    let rest_is_valid = name
        .chars()
        .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_');
    starts_with_letter && rest_is_valid
}

fn push_value_or_placeholder(filled: &mut String, name: &str, values: &[(&str, &str)]) {
    match values.iter().find(|(value_name, _)| *value_name == name) {
        Some((_, value)) => filled.push_str(value),
        None => {
            filled.push(PLACEHOLDER_OPEN);
            filled.push_str(name);
            filled.push(PLACEHOLDER_CLOSE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_by_name() {
        let template = "{operation} needs {needed_bytes} bytes on {volume}.";
        let values = [("volume", "/home"), ("operation", "Saving"), ("needed_bytes", "4096")];
        assert_eq!(
            fill_placeholders(template, &values),
            "Saving needs 4096 bytes on /home."
        );
    }

    #[test]
    fn a_missing_placeholder_value_stays_visible() {
        let template = "{operation} needs {needed_bytes} bytes.";
        assert_eq!(
            fill_placeholders(template, &[("operation", "Saving")]),
            "Saving needs {needed_bytes} bytes."
        );
        assert_eq!(fill_placeholders(template, &[]), template);
    }

    #[test]
    fn braces_that_are_not_placeholders_are_text() {
        let values = [("name", "x")];
        assert_eq!(
            fill_placeholders("{} and {Name} and { name }", &values),
            "{} and {Name} and { name }"
        );
        assert_eq!(fill_placeholders("a {b", &values), "a {b");
        assert_eq!(fill_placeholders("{{name}}", &values), "{x}");
        assert_eq!(fill_placeholders("{1st}", &values), "{1st}");
    }

    #[test]
    fn values_are_not_filled_twice() {
        let values = [("first", "{second}"), ("second", "no")];
        assert_eq!(fill_placeholders("{first}", &values), "{second}");
    }

    #[test]
    fn multi_byte_text_around_placeholders_is_kept() {
        assert_eq!(fill_placeholders("é {name} ü", &[("name", "ß")]), "é ß ü");
    }

    #[test]
    fn unknown_code_has_no_entry() {
        assert_eq!(find_entry("RRS-E9999"), None);
    }
}
