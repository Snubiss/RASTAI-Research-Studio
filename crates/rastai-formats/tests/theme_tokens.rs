//! No theme can set a locked security-UI token (contracts 5.6, owner decision N7): each `--rr-locked-*` name, in
//! `colors` or anywhere else in the file, is an unknown field (`RRS-E1003`), and so is a misspelled role. The color
//! roles are struct fields, so there is no free map a locked name could slip into (layout 4.2a).
//!
//! The built-in light, dark and high-contrast themes of `@rastai/ui` (M1-51, DESIGN-03) are read through the same
//! `read_document`: the bytes the web app imports are the bytes rastai-formats validates, and each sets every color
//! role.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;

use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::Document;
use rastai_formats::documents::theme::COLOR_ROLES;
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{ReadOutcome, read_document};
use serde_json::{Value, json};

const BASE_THEME: &str = "../../fixtures/documents/rastai.theme/valid/night.theme.json";
const THEME_FILE: &str = "test.theme.json";
/// `@rastai/ui`'s built-in themes (layout frontend-and-plugins.md 2.10), with the base each must declare. High
/// contrast is a theme of its own with the dark base, so it takes the dark locked set (ui-design.md 4.1).
const BUILT_IN_THEMES: &[(&str, &str)] = &[
    ("../../packages/ui/src/tokens/light.theme.json", "light"),
    ("../../packages/ui/src/tokens/dark.theme.json", "dark"),
    ("../../packages/ui/src/tokens/high_contrast.theme.json", "dark"),
];
/// The dark theme's canvas background, the design's value (ui-design.md 4.1).
const DARK_CANVAS_BACKGROUND: &str = "#0d0f13";
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
    serde_json::from_slice(&read_file(BASE_THEME)).unwrap()
}

fn read_file(relative_path: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path)).unwrap()
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

/// The base theme with the two tokens of DESIGN-02: an optional display size and a chat tint.
fn theme_with_new_tokens() -> Value {
    let mut theme = base_theme();
    theme["typography"]["font_sizes_px"]["xxl"] = json!(32);
    theme["colors"]["confidence_tint"] = json!("#fff1c2");
    theme
}

#[test]
fn a_theme_with_the_new_tokens_validates_and_reads_them_back() {
    let outcome = read_theme(&theme_with_new_tokens());
    assert_eq!(outcome.diagnostics, vec![]);
    let Some(Document::Theme(theme)) = outcome.document else {
        panic!("the theme with the new tokens must read as a theme");
    };
    assert_eq!(theme.typography.font_sizes_px.xxl, Some(32));
    assert_eq!(theme.colors.confidence_tint.as_deref(), Some("#fff1c2"));
}

#[test]
fn a_theme_without_the_new_tokens_validates_and_leaves_them_unset() {
    let outcome = read_theme(&base_theme());
    assert_eq!(outcome.diagnostics, vec![]);
    let Some(Document::Theme(theme)) = outcome.document else {
        panic!("the base theme must read as a theme");
    };
    assert_eq!(theme.typography.font_sizes_px.xxl, None);
    assert_eq!(theme.colors.confidence_tint, None);
}

#[test]
fn a_new_color_that_is_not_rrggbb_is_a_meaning_error() {
    let mut theme = theme_with_new_tokens();
    theme["colors"]["confidence_tint"] = json!("amber");
    assert_eq!(
        codes_and_pointers(&read_theme(&theme).diagnostics),
        one(CODE_VALUE_FORM, "/colors/confidence_tint")
    );
}

#[test]
fn a_misspelled_new_token_is_an_unknown_field() {
    let mut theme = theme_with_new_tokens();
    theme["colors"]["confidence-tint"] = json!("#fff1c2");
    theme["typography"]["font_sizes_px"]["xxxl"] = json!(40);
    let outcome = read_theme(&theme);
    let expected = vec![
        (CODE_UNKNOWN_FIELD.to_string(), "/colors/confidence-tint".to_string()),
        (
            CODE_UNKNOWN_FIELD.to_string(),
            "/typography/font_sizes_px/xxxl".to_string(),
        ),
    ];
    assert_eq!(codes_and_pointers(&outcome.diagnostics), expected);
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

#[test]
fn the_built_in_themes_are_valid_and_set_every_role() {
    for (path, base) in BUILT_IN_THEMES {
        let bytes = read_file(path);
        let file_name = Path::new(path).file_name().unwrap().to_str().unwrap();
        let outcome = read_document(file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
        assert_eq!(outcome.diagnostics, vec![], "{path}");
        assert!(outcome.document.is_some(), "{path}");

        let theme: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(theme["base"], json!(base), "{path}");
        for (role, _) in COLOR_ROLES {
            assert!(theme["colors"][*role].is_string(), "{path} leaves out the role {role}");
        }
    }
}

#[test]
fn the_dark_theme_has_the_designs_canvas_background() {
    let (dark_path, _) = BUILT_IN_THEMES[1];
    let theme: Value = serde_json::from_slice(&read_file(dark_path)).unwrap();
    assert_eq!(theme["colors"]["canvas_background"], json!(DARK_CANVAS_BACKGROUND));
}

#[test]
fn the_built_in_display_size_is_28_and_a_wrong_type_is_refused() {
    let (light_path, _) = BUILT_IN_THEMES[0];
    let theme: Value = serde_json::from_slice(&read_file(light_path)).unwrap();
    assert_eq!(theme["typography"]["font_sizes_px"]["xxl"], json!(28));

    // A display size written as text is a wrong kind of field (RRS-E1002), not a value out of range.
    let mut wrong = base_theme();
    wrong["typography"]["font_sizes_px"]["xxl"] = json!("28");
    assert_eq!(
        codes_and_pointers(&read_theme(&wrong).diagnostics),
        one("RRS-E1002", "/typography/font_sizes_px/xxl")
    );
}
