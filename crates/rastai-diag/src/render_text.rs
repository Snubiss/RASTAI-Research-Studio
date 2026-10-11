//! Compiler-style text for a diagnostic: code and title, location, a source snippet with a caret, and the fix
//! (layout: rust-control-plane.md section 3.2). The CLI uses it with `Color` on a terminal and `Plain` otherwise.
//!
//! ```text
//! error[RRS-E2004]: Input ranges overlap
//!   --> designs/textcnn.rrsmodel:14:9  #/inputs/1/range
//!    |
//! 14 |         "range": [0, 128],
//!    |         ^^^^^^^^^^^^^^^^^ overlaps input "title" (0..64)
//!    = fix: shrink one of the ranges, or merge the two inputs
//! ```

use crate::catalog::find_entry;
use crate::diagnostic::{Diagnostic, Severity, severity_name};
use crate::path::{DiagnosticPath, format_path, parse_path};

/// A tab in the snippet is shown as this many spaces, so the caret line can line up under it.
/// Wide characters (CJK, emoji) count as one column; terminals differ on them and the corpus has none.
const TAB_WIDTH: usize = 4;
const CARET: char = '^';
/// Gutter width when there is no snippet line number to size it.
const MINIMUM_GUTTER_WIDTH: usize = 1;
const FIX_LABEL: &str = "fix: ";
/// Shown in place of a control character, so text from a user's file or a plugin can't move the cursor, change the
/// colors or retitle the terminal it is printed to.
const REPLACEMENT_CHARACTER: char = '\u{FFFD}';

// ANSI terminal colors, the same roles rustc uses: red errors, yellow warnings, cyan info, blue gutter.
const COLOR_ERROR: &str = "\x1b[1;31m";
const COLOR_WARNING: &str = "\x1b[1;33m";
const COLOR_INFO: &str = "\x1b[1;36m";
const COLOR_GUTTER: &str = "\x1b[1;34m";
const COLOR_BOLD: &str = "\x1b[1m";
const COLOR_RESET: &str = "\x1b[0m";

/// Plain text, or text with ANSI colors for a terminal. Both have the same characters once the colors are removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStyle {
    Plain,
    Color,
}

/// The source line a diagnostic points at, ready to print: tabs expanded, caret position in display columns.
struct Snippet {
    line_number: u32,
    shown_line: String,
    caret_start: usize,
    caret_length: usize,
}

/// Renders a diagnostic with its catalog title. `source_text` is the whole file the path points into; without it
/// (or when the path has no line) the message is printed on its own line instead of under a caret.
pub fn render_text(diagnostic: &Diagnostic, source_text: Option<&str>, style: RenderStyle) -> String {
    let title = match find_entry(&diagnostic.code) {
        Some(entry) => entry.title,
        None => "",
    };
    render_text_with_title(diagnostic, title, source_text, style)
}

/// Renders a diagnostic with a title the caller supplies, for codes the compiled catalog doesn't hold.
/// An empty title leaves the header as just the severity and code.
pub fn render_text_with_title(
    diagnostic: &Diagnostic,
    title: &str,
    source_text: Option<&str>,
    style: RenderStyle,
) -> String {
    let diagnostic = &printable_copy(diagnostic);
    let title = &replace_control_characters(title);
    let path = parse_path(&diagnostic.path);
    let snippet = source_text.and_then(|text| find_snippet(text, &path));
    let gutter_width = match &snippet {
        Some(found) => found.line_number.to_string().len(),
        None => MINIMUM_GUTTER_WIDTH,
    };

    let mut lines = vec![header_line(diagnostic, title, style)];
    if !diagnostic.path.is_empty() {
        lines.push(location_line(&path, gutter_width, style));
    }
    match &snippet {
        Some(found) => lines.extend(snippet_lines(found, diagnostic, gutter_width, style)),
        None if !diagnostic.message.is_empty() => lines.push(note_line(&diagnostic.message, gutter_width, style)),
        None => {}
    }
    if !diagnostic.fix_text.is_empty() {
        let fix_line = format!("{FIX_LABEL}{}", diagnostic.fix_text);
        lines.push(note_line(&fix_line, gutter_width, style));
    }

    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// `error[RRS-E2004]: Input ranges overlap`
fn header_line(diagnostic: &Diagnostic, title: &str, style: RenderStyle) -> String {
    let label = format!("{}[{}]", severity_name(diagnostic.severity), diagnostic.code);
    let mut line = paint(&label, severity_color(diagnostic.severity), style);
    if !title.is_empty() {
        line.push_str(&paint(&format!(": {title}"), COLOR_BOLD, style));
    }
    line
}

/// `  --> designs/textcnn.rrsmodel:14:9  #/inputs/1/range`: the JSON pointer is set apart so it reads as a second
/// way to find the spot, not as part of the file name.
fn location_line(path: &DiagnosticPath, gutter_width: usize, style: RenderStyle) -> String {
    let file_part = DiagnosticPath {
        json_pointer: String::new(),
        ..path.clone()
    };
    let mut location = format_path(&file_part);
    if !path.json_pointer.is_empty() {
        if !location.is_empty() {
            location.push_str("  ");
        }
        location.push('#');
        location.push_str(&path.json_pointer);
    }
    let arrow = paint("-->", COLOR_GUTTER, style);
    format!("{}{arrow} {location}", " ".repeat(gutter_width))
}

/// The empty gutter line, the numbered source line and the caret line with the message beside it.
fn snippet_lines(snippet: &Snippet, diagnostic: &Diagnostic, gutter_width: usize, style: RenderStyle) -> Vec<String> {
    let empty_gutter = paint(&format!("{} |", " ".repeat(gutter_width)), COLOR_GUTTER, style);
    let numbered_gutter = paint(&format!("{} |", snippet.line_number), COLOR_GUTTER, style);

    let mut source_line = numbered_gutter;
    if !snippet.shown_line.is_empty() {
        source_line.push(' ');
        source_line.push_str(&snippet.shown_line);
    }

    let mut marker = CARET.to_string().repeat(snippet.caret_length);
    if !diagnostic.message.is_empty() {
        marker.push(' ');
        marker.push_str(&diagnostic.message);
    }
    let caret_line = format!(
        "{empty_gutter} {}{}",
        " ".repeat(snippet.caret_start),
        paint(&marker, severity_color(diagnostic.severity), style)
    );

    vec![empty_gutter, source_line, caret_line]
}

/// `   = fix: ...`, and the message when there is no snippet to hang it on.
fn note_line(text: &str, gutter_width: usize, style: RenderStyle) -> String {
    let equals = paint("=", COLOR_GUTTER, style);
    format!("{} {equals} {text}", " ".repeat(gutter_width))
}

/// The snippet for a path's line and column, or None when the path has no line or the source is shorter.
fn find_snippet(source_text: &str, path: &DiagnosticPath) -> Option<Snippet> {
    if path.line == 0 {
        return None;
    }
    let line_index = usize::try_from(path.line - 1).ok()?;
    let line = source_text.lines().nth(line_index)?;
    let characters: Vec<char> = line.trim_end().chars().collect();

    // Column 0 means "somewhere on this line": point at the first character.
    let column_index = usize::try_from(path.column.max(1) - 1).ok()?.min(characters.len());

    Some(Snippet {
        line_number: path.line,
        shown_line: shown_source_line(&characters),
        caret_start: display_width(&characters[..column_index]),
        caret_length: caret_length(&characters, column_index),
    })
}

/// Where the underline stops: the end of the line, less one trailing comma, so `"range": [0, 128],` underlines the
/// value and not the separator after it.
fn underline_end(characters: &[char]) -> usize {
    match characters.last() {
        Some(',') => characters.len() - 1,
        _ => characters.len(),
    }
}

/// From the column to the underline end, and at least one caret even past the end of the line
/// (a missing value at the end of a line is still a place).
fn caret_length(characters: &[char], column_index: usize) -> usize {
    let end = underline_end(characters);
    if column_index >= end {
        return 1;
    }
    display_width(&characters[column_index..end])
}

/// The source line as printed: tabs expanded, other control characters replaced, one column per character.
fn shown_source_line(characters: &[char]) -> String {
    let mut shown = String::new();
    for character in characters {
        if *character == '\t' {
            shown.push_str(&" ".repeat(TAB_WIDTH));
        } else if character.is_control() {
            shown.push(REPLACEMENT_CHARACTER);
        } else {
            shown.push(*character);
        }
    }
    shown
}

/// A tab becomes a space and any other control character (newlines and terminal escapes included) the replacement
/// character, so a rendered diagnostic keeps its layout and can't send commands to a terminal.
pub(crate) fn replace_control_characters(text: &str) -> String {
    let mut printable = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\t' {
            printable.push(' ');
        } else if character.is_control() {
            printable.push(REPLACEMENT_CHARACTER);
        } else {
            printable.push(character);
        }
    }
    printable
}

/// The diagnostic with every text field made printable.
fn printable_copy(diagnostic: &Diagnostic) -> Diagnostic {
    Diagnostic {
        code: replace_control_characters(&diagnostic.code),
        severity: diagnostic.severity,
        message: replace_control_characters(&diagnostic.message),
        path: replace_control_characters(&diagnostic.path),
        fix_text: replace_control_characters(&diagnostic.fix_text),
        fix_command_id: diagnostic.fix_command_id.clone(),
        fix_arguments_json: diagnostic.fix_arguments_json.clone(),
    }
}

fn display_width(characters: &[char]) -> usize {
    characters
        .iter()
        .map(|character| if *character == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

fn severity_color(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => COLOR_ERROR,
        Severity::Warning => COLOR_WARNING,
        Severity::Info => COLOR_INFO,
    }
}

fn paint(text: &str, color: &str, style: RenderStyle) -> String {
    match style {
        RenderStyle::Plain => text.to_string(),
        RenderStyle::Color => format!("{color}{text}{COLOR_RESET}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "{\n\t\"name\": \"größe\",\n  \"range\": [0, 128],\n  \"last\": true\n}";

    fn located(path: &str, message: &str) -> Diagnostic {
        Diagnostic {
            code: "RRS-E2004".to_string(),
            severity: Severity::Error,
            message: message.to_string(),
            path: path.to_string(),
            fix_text: String::new(),
            fix_command_id: String::new(),
            fix_arguments_json: String::new(),
        }
    }

    fn snippet_of(path: &str) -> Vec<String> {
        let text = render_text_with_title(&located(path, "here"), "Title", Some(SOURCE), RenderStyle::Plain);
        text.lines().skip(2).take(3).map(str::to_string).collect()
    }

    #[test]
    fn caret_on_the_first_line() {
        assert_eq!(snippet_of("a.json:1:1"), ["  |", "1 | {", "  | ^ here"]);
    }

    #[test]
    fn caret_on_the_last_line_without_a_trailing_newline() {
        assert_eq!(snippet_of("a.json:5:1"), ["  |", "5 | }", "  | ^ here"]);
    }

    #[test]
    fn tabs_are_expanded_in_the_line_and_the_caret() {
        // Column 2 is the `"` after the tab, shown four columns in.
        assert_eq!(
            snippet_of("a.json:2:2"),
            ["  |", "2 |     \"name\": \"größe\",", "  |     ^^^^^^^^^^^^^^^ here"]
        );
    }

    #[test]
    fn multi_byte_characters_count_as_one_column() {
        // Column 13 is the `ö`; the caret runs to the closing quote and skips the comma.
        assert_eq!(
            snippet_of("a.json:2:13"),
            ["  |", "2 |     \"name\": \"größe\",", "  |                ^^^^ here"]
        );
    }

    #[test]
    fn a_column_past_the_end_gets_one_caret_after_the_line() {
        assert_eq!(
            snippet_of("a.json:4:40"),
            ["  |", "4 |   \"last\": true", "  |               ^ here"]
        );
    }

    #[test]
    fn a_line_past_the_end_has_no_snippet() {
        let text = render_text_with_title(
            &located("a.json:9:1", "here"),
            "Title",
            Some(SOURCE),
            RenderStyle::Plain,
        );
        assert_eq!(text, "error[RRS-E2004]: Title\n --> a.json:9:1\n  = here\n");
    }

    #[test]
    fn control_characters_never_reach_the_terminal() {
        let mut hostile = located("a.json:6:1", "red\x1b[31m\ttext\nmore");
        hostile.fix_text = "fix\x07".to_string();
        let source = "{\n}\n\n\n\nbad\x1b]0;title\x07";
        let text = render_text_with_title(&hostile, "Ti\x1btle", Some(source), RenderStyle::Plain);
        assert!(
            !text
                .chars()
                .any(|character| character.is_control() && character != '\n')
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "error[RRS-E2004]: Ti\u{FFFD}tle",
                " --> a.json:6:1",
                "  |",
                "6 | bad\u{FFFD}]0;title\u{FFFD}",
                "  | ^^^^^^^^^^^^^ red\u{FFFD}[31m text\u{FFFD}more",
                "  = fix: fix\u{FFFD}",
            ]
        );
    }

    #[test]
    fn an_unknown_code_renders_without_a_title() {
        let mut unknown = located("", "unknown diagnostic code");
        unknown.code = "RRS-E9999".to_string();
        let text = render_text(&unknown, None, RenderStyle::Plain);
        assert_eq!(text, "error[RRS-E9999]\n  = unknown diagnostic code\n");
    }
}
