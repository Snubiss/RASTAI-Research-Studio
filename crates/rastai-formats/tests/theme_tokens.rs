//! No theme can set a locked security-UI token (contracts 5.6, owner decision N7): each `--rr-locked-*` name, in
//! `colors` or anywhere else in the file, is an unknown field (`RRS-E1003`), and so is a misspelled role. The color
//! roles are struct fields, so there is no free map a locked name could slip into (layout 4.2a).
//!
//! The built-in light and dark themes of `@rastai/ui` join this file when they exist (M1-51), read through the same
//! `read_document`.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;

use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{ReadOutcome, read_document};
use serde_json::{Value, json};

const BASE_THEME: &str = "../../fixtures/documents/rastai.theme/valid/night.theme.json";
const THEME_FILE: &str = "test.theme.json";
const CODE_UNKNOWN_FIELD: &str = "RRS-E1003";
const CODE_VALUE_FORM: &str = "RRS-E2006";
/// The locked tokens of contracts 5.6, as they are written in CSS.
const LOCKED_TOKENS: &[&str] = &[
    "--rr-locked-severity-error",
    "--rr-locked-severity-warning",
    "--rr-locked-severity-info",
    "--rr-locked-egress",
    "--rr-locked-egress-text",
    "--rr-locked-security-surface",
    "--rr-locked-security-text",
    "--rr-locked-security-danger",
];

fn base_theme() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(BASE_THEME);
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn read_theme(theme: &Value) -> ReadOutcome {
    let bytes = pretty_json(theme).unwrap();
    read_document(THEME_FILE, &SliceSource { bytes: &bytes }, &ImportLimits::default())
}

/// The (code, JSON pointer) of each diagnostic; the line and column depend on the writer, not on the rule.
fn codes_and_pointers(diagnostics: &[Diagnostic]) -> Vec<(String, String)> {
    diagnostics
        .iter()
        .map(|item| {
            let pointer = item
                .path
                .split_once('#')
                .map(|(_, pointer)| pointer)
                .unwrap_or_default();
            (item.code.clone(), pointer.to_string())
        })
        .collect()
}

fn one(code: &str, pointer: &str) -> Vec<(String, String)> {
    vec![(code.to_string(), pointer.to_string())]
}

#[test]
fn the_base_theme_is_valid() {
    let outcome = read_theme(&base_theme());
    assert_eq!(outcome.diagnostics, vec![]);
    assert!(outcome.document.is_some());
}

#[test]
fn every_locked_token_as_a_color_is_an_unknown_field() {
    for token in LOCKED_TOKENS {
        let mut theme = base_theme();
        theme["colors"][*token] = json!("#ff0000");
        let outcome = read_theme(&theme);
        let pointer = format!("/colors/{token}");
        assert_eq!(
            codes_and_pointers(&outcome.diagnostics),
            one(CODE_UNKNOWN_FIELD, &pointer)
        );
        assert!(
            outcome.document.is_none(),
            "{token}: a theme that names it must not load"
        );
    }
}

#[test]
fn a_locked_token_anywhere_else_is_an_unknown_field_too() {
    for token in LOCKED_TOKENS {
        let mut theme = base_theme();
        theme[*token] = json!("#ff0000");
        theme["typography"][*token] = json!("#ff0000");
        let outcome = read_theme(&theme);
        let expected = vec![
            (CODE_UNKNOWN_FIELD.to_string(), format!("/typography/{token}")),
            (CODE_UNKNOWN_FIELD.to_string(), format!("/{token}")),
        ];
        assert_eq!(codes_and_pointers(&outcome.diagnostics), expected, "{token}");
    }
}

#[test]
fn a_css_variable_name_or_a_misspelled_role_is_not_a_role() {
    for key in ["--rr-color-accent", "accentColor", "backgrund", "series_9"] {
        let mut theme = base_theme();
        theme["colors"][key] = json!("#ff0000");
        let pointer = format!("/colors/{key}");
        assert_eq!(
            codes_and_pointers(&read_theme(&theme).diagnostics),
            one(CODE_UNKNOWN_FIELD, &pointer)
        );
    }
}

#[test]
fn a_color_that_is_not_rrggbb_is_a_meaning_error_and_the_theme_still_reads() {
    let mut theme = base_theme();
    theme["colors"]["focus_ring"] = json!("hsl(210, 80%, 50%)");
    let outcome = read_theme(&theme);
    assert_eq!(
        codes_and_pointers(&outcome.diagnostics),
        one(CODE_VALUE_FORM, "/colors/focus_ring")
    );
    // Layer 3 doesn't block loading: the editor shows the document with its error (layout 4.4).
    assert!(outcome.document.is_some());
}
