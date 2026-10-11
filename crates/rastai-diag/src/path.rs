//! The `Diagnostic.path` grammar (contracts 4.1): `[<file>[:<line>:<column>]][#<json pointer>]`.
//! Example: `designs/textcnn.rrsmodel:14:9#/inputs/1/range`.
//!
//! Lines and columns start at 1 and 0 means "not known". A column counts characters (Unicode scalar values), not
//! bytes, so a caret lines up under the same character in every language that renders the path.

/// Separates the file part from the JSON pointer. A JSON pointer always starts with `/`, so the separator is `#/`:
/// a file named `notes#2.json` stays whole.
const POINTER_SEPARATOR: &str = "#/";
const POINTER_MARK: char = '#';
const POSITION_SEPARATOR: char = ':';

/// A diagnostic location split into its parts. Empty strings and zeros mean the part is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticPath {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub json_pointer: String,
}

/// Builds the path text. A line and column are written only with a file (the grammar nests them),
/// and an empty JSON pointer is left out.
pub fn format_path(path: &DiagnosticPath) -> String {
    let mut text = path.file.clone();
    if !path.file.is_empty() && path.line > 0 {
        text.push_str(&format!(":{}:{}", path.line, path.column));
    }
    if !path.json_pointer.is_empty() {
        text.push(POINTER_MARK);
        text.push_str(&path.json_pointer);
    }
    text
}

/// Splits path text into its parts. It never fails: text that doesn't match the grammar becomes the file name,
/// so a diagnostic always shows where it points, even when another program wrote the path.
pub fn parse_path(text: &str) -> DiagnosticPath {
    let (location, json_pointer) = split_pointer(text);
    let (file, line, column) = split_line_and_column(location);
    DiagnosticPath {
        file: file.to_string(),
        line,
        column,
        json_pointer: json_pointer.to_string(),
    }
}

/// Returns (location, json pointer). The pointer keeps its leading `/`.
fn split_pointer(text: &str) -> (&str, &str) {
    match text.find(POINTER_SEPARATOR) {
        Some(index) => (&text[..index], &text[index + 1..]),
        None => (text, ""),
    }
}

/// Returns (file, line, column). The numbers are read from the right, because a Windows file name
/// (`C:\designs\a.rrsmodel`) has a colon of its own.
fn split_line_and_column(location: &str) -> (&str, u32, u32) {
    let Some((before_column, column_text)) = location.rsplit_once(POSITION_SEPARATOR) else {
        return (location, 0, 0);
    };
    let Some((file, line_text)) = before_column.rsplit_once(POSITION_SEPARATOR) else {
        return (location, 0, 0);
    };
    match (parse_number(line_text), parse_number(column_text)) {
        (Some(line), Some(column)) if !file.is_empty() => (file, line, column),
        _ => (location, 0, 0),
    }
}

/// Digits only: `u32::from_str` would also take a leading `+`, which isn't part of the grammar.
fn parse_number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(file: &str, line: u32, column: u32, json_pointer: &str) -> DiagnosticPath {
        DiagnosticPath {
            file: file.to_string(),
            line,
            column,
            json_pointer: json_pointer.to_string(),
        }
    }

    #[test]
    fn every_shape_round_trips() {
        let shapes = [
            (
                "designs/textcnn.rrsmodel:14:9#/inputs/1/range",
                path("designs/textcnn.rrsmodel", 14, 9, "/inputs/1/range"),
            ),
            (
                "designs/textcnn.rrsmodel:14:9",
                path("designs/textcnn.rrsmodel", 14, 9, ""),
            ),
            (
                "designs/textcnn.rrsmodel#/inputs/0",
                path("designs/textcnn.rrsmodel", 0, 0, "/inputs/0"),
            ),
            ("designs/textcnn.rrsmodel", path("designs/textcnn.rrsmodel", 0, 0, "")),
            ("#/layers/3/kind", path("", 0, 0, "/layers/3/kind")),
            ("", path("", 0, 0, "")),
            (
                "C:\\designs\\a.rrsmodel:2:5#/a~1b",
                path("C:\\designs\\a.rrsmodel", 2, 5, "/a~1b"),
            ),
            ("notes#2.json:3:1", path("notes#2.json", 3, 1, "")),
            ("a.json#/key#with#marks", path("a.json", 0, 0, "/key#with#marks")),
        ];
        for (text, expected) in shapes {
            assert_eq!(parse_path(text), expected, "parsing {text:?}");
            assert_eq!(format_path(&expected), text, "formatting {expected:?}");
        }
    }

    #[test]
    fn text_outside_the_grammar_stays_in_the_file_name() {
        for text in ["a.json:3", "a.json:x:4", "a.json:+3:4", ":3:4", "a.json:99999999999:1"] {
            assert_eq!(parse_path(text), path(text, 0, 0, ""), "parsing {text:?}");
        }
    }

    #[test]
    fn line_and_column_need_a_file() {
        assert_eq!(format_path(&path("", 3, 4, "/a")), "#/a");
    }
}
