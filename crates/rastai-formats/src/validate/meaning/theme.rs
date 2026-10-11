//! Layer 3 for `rastai.theme/1`: every color that is set is `#rrggbb` (`RRS-E2006` at the color's path).
//!
//! Locked tokens and unknown roles never get here: they are unknown fields, refused by layer 2 (`RRS-E1003`). The
//! M8 accessibility check (WCAG 2.2 AA contrast, 24 px targets) joins this file, so the theme picker, `plugin lint`
//! and the plugin host all get the same verdict (layout 4.2a).

use rastai_diag::diagnostic::Diagnostic;

use crate::documents::theme::{ThemeDocument, set_colors};
use crate::json::pointer::join_pointer;
use crate::validate::meaning::{MeaningSource, meaning_diagnostic};

const CODE_VALUE_FORM: &str = "RRS-E2006";
const COLORS_POINTER: &str = "/colors";
const EXPECTED_COLOR: &str = "a #rrggbb color, such as #1f6feb";
const COLOR_PREFIX: char = '#';
const COLOR_HEX_DIGITS: usize = 6;

/// Every meaning problem of a theme.
pub fn check_theme(theme: &ThemeDocument, source: &MeaningSource) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (role, color) in set_colors(&theme.colors) {
        if !is_hex_color(color) {
            let values = [("found", color), ("expected", EXPECTED_COLOR)];
            let pointer = join_pointer(COLORS_POINTER, role);
            diagnostics.push(meaning_diagnostic(CODE_VALUE_FORM, &values, source, &pointer));
        }
    }
    diagnostics
}

/// `#` and six hex digits, either case: `#1f6feb`, `#1F6FEB`. Not `#fff`, `rgb(...)` or a color name: CSS would
/// accept those, but renderers read the six digits straight into RGB (`canvasColors`).
pub fn is_hex_color(text: &str) -> bool {
    let Some(digits) = text.strip_prefix(COLOR_PREFIX) else {
        return false;
    };
    digits.len() == COLOR_HEX_DIGITS && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_hex_digits_after_a_hash_is_a_color() {
        for color in ["#000000", "#ffffff", "#1F6FEB", "#a0B1c2"] {
            assert!(is_hex_color(color), "{color}");
        }
    }

    #[test]
    fn other_css_color_forms_are_not() {
        for text in [
            "",
            "#",
            "#fff",
            "#ffffff00",
            "1f6feb",
            "#gggggg",
            "rgb(0, 0, 0)",
            "red",
            "#12345",
            "#１２３４５６",
        ] {
            assert!(!is_hex_color(text), "{text}");
        }
    }
}
