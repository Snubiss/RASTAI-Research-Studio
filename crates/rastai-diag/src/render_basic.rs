//! Basic-mode text for a diagnostic: the catalog's plain-English `basic_text` and the fix
//! (layout: rust-control-plane.md section 3.2; ADR-0030 decision 7). The CLI prints it unless `--verbose`,
//! the WASM build uses it for form hints, and MCP tool errors carry it beside the code.

use crate::catalog::find_entry;
use crate::diagnostic::Diagnostic;
use crate::render_text::replace_control_characters;

const FIX_LABEL: &str = "Fix: ";

/// Renders a diagnostic with its catalog `basic_text`. A code the catalog doesn't hold falls back to the message,
/// so the reader always sees something.
pub fn render_basic(diagnostic: &Diagnostic) -> String {
    let basic_text = match find_entry(&diagnostic.code) {
        Some(entry) => entry.basic_text,
        None => "",
    };
    render_basic_with_text(diagnostic, basic_text)
}

/// Renders a diagnostic with plain-English text the caller supplies, for codes the compiled catalog doesn't hold.
/// An empty `basic_text` falls back to the message. Control characters are replaced, as in `render_text`.
pub fn render_basic_with_text(diagnostic: &Diagnostic, basic_text: &str) -> String {
    let main_text = if basic_text.is_empty() {
        diagnostic.message.as_str()
    } else {
        basic_text
    };
    let mut text = format!("{}\n", replace_control_characters(main_text));
    if !diagnostic.fix_text.is_empty() {
        text.push_str(&format!(
            "{FIX_LABEL}{}\n",
            replace_control_characters(&diagnostic.fix_text)
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;

    fn sample(fix_text: &str) -> Diagnostic {
        Diagnostic {
            code: "RRS-E9999".to_string(),
            severity: Severity::Warning,
            message: "the detailed message".to_string(),
            path: "a.json:1:1".to_string(),
            fix_text: fix_text.to_string(),
            fix_command_id: String::new(),
            fix_arguments_json: String::new(),
        }
    }

    #[test]
    fn basic_text_and_fix() {
        assert_eq!(
            render_basic_with_text(&sample("Do this."), "Plain words."),
            "Plain words.\nFix: Do this.\n"
        );
    }

    #[test]
    fn no_fix_line_without_fix_text() {
        assert_eq!(render_basic_with_text(&sample(""), "Plain words."), "Plain words.\n");
    }

    #[test]
    fn control_characters_are_replaced() {
        let rendered = render_basic_with_text(&sample("a\x1b[2J"), "b\nc");
        assert_eq!(rendered, "b\u{FFFD}c\nFix: a\u{FFFD}[2J\n");
    }

    #[test]
    fn unknown_code_falls_back_to_the_message() {
        assert_eq!(render_basic(&sample("")), "the detailed message\n");
    }
}
