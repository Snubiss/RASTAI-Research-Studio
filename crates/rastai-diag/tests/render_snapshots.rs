//! Render snapshots (layout: rust-control-plane.md section 3.5): each `fixtures/diagnostics/<case>.json` renders to
//! `<case>.txt` in plain text, to the same text once the colors are stripped, and to `<case>.basic.txt` in Basic mode.
//! The cases cover every severity and every shape of the path grammar; `fixtures/diagnostics/README.md` has the format.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_diag::path::parse_path;
use rastai_diag::render_basic::render_basic_with_text;
use rastai_diag::render_text::{RenderStyle, render_text_with_title};
use serde_json::Value;

const FIXTURE_FOLDER: &str = "../../fixtures/diagnostics";
const CASE_EXTENSION: &str = "json";
const ANSI_ESCAPE: char = '\x1b';
const ANSI_END: char = 'm';

/// One fixture case, read from its JSON file.
struct Case {
    name: String,
    diagnostic: Diagnostic,
    title: String,
    basic_text: String,
    source_text: Option<String>,
}

/// The parts a path can have; together the cases must show every combination the grammar allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathShape {
    FileLineColumnPointer,
    FileLineColumn,
    FilePointer,
    FileOnly,
    PointerOnly,
    Empty,
}

#[test]
fn every_case_matches_its_plain_snapshot() {
    for case in read_cases() {
        let rendered = render_text_with_title(
            &case.diagnostic,
            &case.title,
            case.source_text.as_deref(),
            RenderStyle::Plain,
        );
        assert_eq!(
            rendered,
            read_expected(&case.name, "txt"),
            "plain text of case {}",
            case.name
        );
    }
}

#[test]
fn every_case_matches_its_plain_snapshot_once_colors_are_stripped() {
    for case in read_cases() {
        let colored = render_text_with_title(
            &case.diagnostic,
            &case.title,
            case.source_text.as_deref(),
            RenderStyle::Color,
        );
        assert!(colored.contains(ANSI_ESCAPE), "case {} has no colors", case.name);
        assert_eq!(
            strip_colors(&colored),
            read_expected(&case.name, "txt"),
            "colored text of case {}",
            case.name
        );
    }
}

#[test]
fn every_case_matches_its_basic_snapshot() {
    for case in read_cases() {
        let rendered = render_basic_with_text(&case.diagnostic, &case.basic_text);
        assert_eq!(
            rendered,
            read_expected(&case.name, "basic.txt"),
            "Basic text of case {}",
            case.name
        );
    }
}

#[test]
fn the_cases_cover_every_severity_and_path_shape() {
    let cases = read_cases();
    for severity in [Severity::Error, Severity::Warning, Severity::Info] {
        assert!(
            cases.iter().any(|case| case.diagnostic.severity == severity),
            "no case for {severity:?}"
        );
    }
    let all_shapes = [
        PathShape::FileLineColumnPointer,
        PathShape::FileLineColumn,
        PathShape::FilePointer,
        PathShape::FileOnly,
        PathShape::PointerOnly,
        PathShape::Empty,
    ];
    for shape in all_shapes {
        assert!(
            cases.iter().any(|case| path_shape(&case.diagnostic.path) == shape),
            "no case for {shape:?}"
        );
    }
}

fn fixture_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_FOLDER)
}

/// Every `<case>.json` in the folder, sorted by name so failures come out in a stable order.
fn read_cases() -> Vec<Case> {
    let mut case_paths: Vec<PathBuf> = fs::read_dir(fixture_folder())
        .expect("fixtures/diagnostics exists")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == CASE_EXTENSION))
        .collect();
    case_paths.sort();
    assert!(!case_paths.is_empty(), "no cases in fixtures/diagnostics");
    case_paths.iter().map(|path| read_case(path)).collect()
}

fn read_case(path: &Path) -> Case {
    let json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let source_text = json["source_file"]
        .as_str()
        .map(|file_name| fs::read_to_string(fixture_folder().join(file_name)).unwrap());
    Case {
        name: path.file_stem().unwrap().to_string_lossy().into_owned(),
        diagnostic: diagnostic_from_json(&json["diagnostic"]),
        title: text_field(&json, "title"),
        basic_text: text_field(&json, "basic_text"),
        source_text,
    }
}

fn diagnostic_from_json(json: &Value) -> Diagnostic {
    Diagnostic {
        code: text_field(json, "code"),
        severity: severity_from_name(&text_field(json, "severity")),
        message: text_field(json, "message"),
        path: text_field(json, "path"),
        fix_text: text_field(json, "fix_text"),
        fix_command_id: text_field(json, "fix_command_id"),
        fix_arguments_json: text_field(json, "fix_arguments_json"),
    }
}

fn text_field(json: &Value, key: &str) -> String {
    json[key]
        .as_str()
        .unwrap_or_else(|| panic!("field {key} is missing or not text"))
        .to_string()
}

fn severity_from_name(name: &str) -> Severity {
    match name {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "info" => Severity::Info,
        other => panic!("unknown severity {other}"),
    }
}

fn read_expected(case_name: &str, extension: &str) -> String {
    let path = fixture_folder().join(format!("{case_name}.{extension}"));
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing snapshot {}", path.display()))
}

/// Removes ANSI color codes (`ESC [ ... m`), the only escapes `render_text` writes.
fn strip_colors(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut inside_escape = false;
    for character in text.chars() {
        if character == ANSI_ESCAPE {
            inside_escape = true;
        } else if inside_escape {
            inside_escape = character != ANSI_END;
        } else {
            plain.push(character);
        }
    }
    plain
}

fn path_shape(path_text: &str) -> PathShape {
    let path = parse_path(path_text);
    let has_file = !path.file.is_empty();
    let has_line = path.line > 0;
    let has_pointer = !path.json_pointer.is_empty();
    match (has_file, has_line, has_pointer) {
        (true, true, true) => PathShape::FileLineColumnPointer,
        (true, true, false) => PathShape::FileLineColumn,
        (true, false, true) => PathShape::FilePointer,
        (true, false, false) => PathShape::FileOnly,
        (false, _, true) => PathShape::PointerOnly,
        (false, _, false) => PathShape::Empty,
    }
}
