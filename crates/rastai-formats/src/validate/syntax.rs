//! Layer 1: bytes to a JSON value through the strict reader (json/reader.rs), and the reader's errors as coded
//! diagnostics with a line and column (`RRS-E0xxx`, and the `RRS-E5xxx` limits checked before allocating).
//!
//! The reader stops at the first problem, because JSON after a syntax error can't be trusted to mean anything, so
//! this layer gives at most one diagnostic.

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_diag::path::{DiagnosticPath, format_path};

use crate::byte_source::{ByteSource, read_all_bytes};
use crate::error::FormatsError;
use crate::json::positions::TextPosition;
use crate::json::reader::{ParsedJson, read_json};
use crate::limits::ImportLimits;

/// Reads the whole source and parses it. On any problem the diagnostic is added and the result is None.
pub fn read_raw_json(
    file_name: &str,
    source: &dyn ByteSource,
    limits: &ImportLimits,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ParsedJson> {
    let parsed = read_all_bytes(source, limits).and_then(|bytes| read_json(&bytes, limits));
    match parsed {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            diagnostics.push(syntax_diagnostic(&error, file_name));
            None
        }
    }
}

/// A reading error as a diagnostic: its catalog code, its position in the file (and its pointer, for a duplicate
/// key), and the numbers the message needs.
pub fn syntax_diagnostic(error: &FormatsError, file_name: &str) -> Diagnostic {
    let (position, json_pointer) = error_location(error);
    let path = DiagnosticPath {
        file: file_name.to_string(),
        line: position.map_or(0, |place| place.line),
        column: position.map_or(0, |place| place.column),
        json_pointer,
    };
    let values = error_values(error);
    let borrowed: Vec<(&str, &str)> = values.iter().map(|(name, text)| (*name, text.as_str())).collect();
    diagnostic_at(error.code(), &borrowed, &format_path(&path))
}

/// Where the error is. Size and read errors are about the whole file, so they have no position.
fn error_location(error: &FormatsError) -> (Option<TextPosition>, String) {
    match error {
        FormatsError::JsonSyntax { position, .. }
        | FormatsError::NotUtf8 { position }
        | FormatsError::NotANumber { position }
        | FormatsError::TooDeep { position, .. }
        | FormatsError::TooManyValues { position, .. } => (Some(*position), String::new()),
        FormatsError::DuplicateKey { pointer, position } => (Some(*position), pointer.clone()),
        _ => (None, String::new()),
    }
}

/// The placeholder values for the catalog message. Errors that only writers raise have none here.
fn error_values(error: &FormatsError) -> Vec<(&'static str, String)> {
    match error {
        FormatsError::JsonSyntax { expected, .. } => vec![("expected", expected.to_string())],
        FormatsError::FileTooLarge {
            size_bytes,
            limit_bytes,
        } => vec![
            ("size_bytes", size_bytes.to_string()),
            ("limit_bytes", limit_bytes.to_string()),
        ],
        FormatsError::TooDeep { limit_depth, .. } => vec![("limit_depth", limit_depth.to_string())],
        FormatsError::TooManyValues { limit_count, .. } => vec![("limit_count", limit_count.to_string())],
        FormatsError::ReadFailed { reason, .. } => vec![("reason", reason.clone())],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_source::SliceSource;

    fn read(text: &[u8], limits: &ImportLimits) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let parsed = read_raw_json("doc.json", &SliceSource { bytes: text }, limits, &mut diagnostics);
        assert_eq!(parsed.is_some(), diagnostics.is_empty());
        diagnostics
    }

    fn code_and_path(text: &[u8], limits: &ImportLimits) -> (String, String) {
        let diagnostics = read(text, limits);
        assert_eq!(diagnostics.len(), 1, "layer 1 reports one problem");
        (diagnostics[0].code.clone(), diagnostics[0].path.clone())
    }

    fn pair(code: &str, path: &str) -> (String, String) {
        (code.to_string(), path.to_string())
    }

    #[test]
    fn valid_json_gives_no_diagnostic() {
        assert!(read(b"{\"a\": [1, 2]}", &ImportLimits::default()).is_empty());
    }

    #[test]
    fn reader_errors_keep_their_code_line_and_column() {
        let limits = ImportLimits::default();
        assert_eq!(
            code_and_path(b"{\n  \"a\": 1,\n}", &limits),
            pair("RRS-E0001", "doc.json:3:1")
        );
        assert_eq!(
            code_and_path(b"{\"a\": \xff}", &limits),
            pair("RRS-E0002", "doc.json:1:7")
        );
        assert_eq!(
            code_and_path(b"{\"a\": NaN}", &limits),
            pair("RRS-E0004", "doc.json:1:7")
        );
    }

    #[test]
    fn a_duplicate_key_names_its_pointer_too() {
        let text = b"{\"outer\": {\"a\": 1,\n \"a\": 2}}";
        let expected = pair("RRS-E0003", "doc.json:2:2#/outer/a");
        assert_eq!(code_and_path(text, &ImportLimits::default()), expected);
    }

    #[test]
    fn limits_are_reported_with_their_numbers() {
        let small = ImportLimits {
            max_file_bytes: 4,
            ..ImportLimits::default()
        };
        assert_eq!(code_and_path(b"[1, 2, 3]", &small), pair("RRS-E5003", "doc.json"));
        let shallow = ImportLimits {
            max_json_depth: 1,
            ..ImportLimits::default()
        };
        assert_eq!(code_and_path(b"[[1]]", &shallow), pair("RRS-E5004", "doc.json:1:2"));
        let error = FormatsError::FileTooLarge {
            size_bytes: 9,
            limit_bytes: 4,
        };
        assert_eq!(
            error_values(&error),
            vec![("size_bytes", "9".to_string()), ("limit_bytes", "4".to_string())]
        );
    }
}
