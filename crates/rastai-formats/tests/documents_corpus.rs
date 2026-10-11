//! The document corpus (contracts 12.5, layout 4.11): every file under `fixtures/documents/<folder>/valid/` and
//! `.../invalid/` goes through `read_document`. A valid file must give no error; an invalid file must give exactly
//! the codes and paths its `<name>.expected.json` lists, in order, all from one read. fixtures/documents/README.md
//! describes the folders and the expected-file format.
//!
//! No document type is registered before M1-17, so the second half of this file runs the same layers through
//! `check_document` with a test type of its own: that is where a whole body is checked (unknown fields included).

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::{Diagnostic, has_errors};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};
use rastai_formats::documents::DocumentType;
use rastai_formats::header::{describe_shared_header, header_fields};
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{CheckedDocument, check_document, read_document};
use serde_json::Value;

const VALID_FOLDER: &str = "valid";
const INVALID_FOLDER: &str = "invalid";
const EXPECTED_SUFFIX: &str = ".expected.json";
const EXPECTED_DIAGNOSTICS_KEY: &str = "diagnostics";

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/documents")
}

/// (code, path) of each diagnostic, the two things a corpus case pins.
fn codes_and_paths(diagnostics: &[Diagnostic]) -> Vec<(String, String)> {
    diagnostics
        .iter()
        .map(|item| (item.code.clone(), item.path.clone()))
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(code, path)| (code.to_string(), path.to_string()))
        .collect()
}

/// The fixture files of one folder, sorted, without the expected files.
fn fixture_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && !file_name_of(path).ends_with(EXPECTED_SUFFIX))
        .collect();
    files.sort();
    files
}

fn file_name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Reads `<file name>.expected.json`: `{"diagnostics": [{"code": "...", "path": "..."}, ...]}`. A missing or
/// malformed expected file is a failure of its own case, not of the whole run.
fn read_expected(fixture: &Path) -> Result<Vec<(String, String)>, String> {
    let expected_path = fixture.with_file_name(format!("{}{EXPECTED_SUFFIX}", file_name_of(fixture)));
    let bytes = fs::read(&expected_path).map_err(|_| format!("{} has no expected file", fixture.display()))?;
    let malformed = || format!("{} is not a diagnostics list", expected_path.display());
    let expected = read_json(&bytes, &ImportLimits::default())
        .map_err(|_| malformed())?
        .value;
    let Some(Value::Array(entries)) = expected.get(EXPECTED_DIAGNOSTICS_KEY) else {
        return Err(malformed());
    };
    let mut pairs = Vec::new();
    for entry in entries {
        let (Some(code), Some(path)) = (entry["code"].as_str(), entry["path"].as_str()) else {
            return Err(malformed());
        };
        pairs.push((code.to_string(), path.to_string()));
    }
    Ok(pairs)
}

/// Runs one case and returns a failure message, or None when it passed. Every case runs before the test fails, so
/// one run lists every broken case.
fn check_case(fixture: &Path, expect_valid: bool) -> Option<String> {
    let bytes = fs::read(fixture).unwrap();
    let file_name = file_name_of(fixture);
    let outcome = read_document(&file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
    let actual = codes_and_paths(&outcome.diagnostics);
    if expect_valid {
        let message = format!("{}: expected valid, got {actual:?}", fixture.display());
        return has_errors(&outcome.diagnostics).then_some(message);
    }
    match read_expected(fixture) {
        Ok(expected) if expected == actual => None,
        Ok(expected) => Some(format!(
            "{}:\n  expected {expected:?}\n  got      {actual:?}",
            fixture.display()
        )),
        Err(message) => Some(message),
    }
}

/// Every expected file must belong to a fixture, so a renamed fixture can't leave its expectations behind.
fn check_expected_files_have_fixtures(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| file_name_of(path).ends_with(EXPECTED_SUFFIX))
        .filter(|path| {
            !folder
                .join(file_name_of(path).trim_end_matches(EXPECTED_SUFFIX))
                .is_file()
        })
        .map(|path| format!("{} has no fixture", path.display()))
        .collect()
}

#[test]
fn every_corpus_file_gets_its_expected_diagnostics_in_one_read() {
    let mut failures = Vec::new();
    let mut case_count = 0;
    let mut type_folders: Vec<PathBuf> = fs::read_dir(corpus_root())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    type_folders.sort();
    for type_folder in type_folders {
        for (folder_name, expect_valid) in [(VALID_FOLDER, true), (INVALID_FOLDER, false)] {
            let folder = type_folder.join(folder_name);
            for fixture in fixture_files(&folder) {
                case_count += 1;
                failures.extend(check_case(&fixture, expect_valid));
            }
            failures.extend(check_expected_files_have_fixtures(&folder));
        }
    }
    assert!(case_count > 0, "the corpus is empty: the harness would test nothing");
    assert!(
        failures.is_empty(),
        "{} corpus failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// A test document type with every kind of field, run through the same layers as read_document.

const SAMPLE_TYPE: &str = "test.sample";
const SAMPLE_TYPES: &[DocumentType] = &[
    DocumentType {
        type_name: SAMPLE_TYPE,
        schema_version: 1,
        file_extensions: &[".sample"],
        description_name: "Sample",
    },
    DocumentType {
        type_name: "test.other",
        schema_version: 1,
        file_extensions: &[".other"],
        description_name: "Sample",
    },
];

fn sample_types() -> Vec<TypeDescription> {
    let mut sample_fields = header_fields();
    sample_fields.extend([
        field("name", FieldKind::String, REQUIRED, "Name."),
        field("mode", enum_of(&["fast", "slow"]), OPTIONAL, "Mode."),
        field(
            "points",
            list_of(FieldKind::Type("Point".to_string())),
            OPTIONAL,
            "Points.",
        ),
        field(
            "limits",
            FieldKind::Map(Box::new(FieldKind::Integer)),
            OPTIONAL,
            "Limits.",
        ),
        field(
            "job_spec",
            FieldKind::Proto("rastai.node.v1.JobSpec".to_string()),
            OPTIONAL,
            "Job.",
        ),
    ]);
    let sample = TypeDescription {
        name: "Sample".to_string(),
        schema_id: "test.sample/1".to_string(),
        doc: "A test document.".to_string(),
        fields: sample_fields,
    };
    let point = TypeDescription {
        name: "Point".to_string(),
        schema_id: "test.sample/1".to_string(),
        doc: "A point.".to_string(),
        fields: vec![
            field("x", FieldKind::Integer, REQUIRED, "X."),
            field("y", FieldKind::Integer, REQUIRED, "Y."),
        ],
    };
    vec![describe_shared_header(), sample, point]
}

fn check_sample(file_name: &str, text: &str) -> CheckedDocument {
    let source = SliceSource { bytes: text.as_bytes() };
    check_document(
        file_name,
        &source,
        &ImportLimits::default(),
        SAMPLE_TYPES,
        &sample_types(),
    )
}

/// A sample document: the header on its first lines, then the body members given.
fn sample_text(schema_version: u32, body: &str) -> String {
    format!(
        "{{\n  \"$schema\": \"https://schemas.example/test.sample/1.json\",\n  \"type\": \"test.sample\",\n  \
         \"schema_version\": {schema_version},\n  \"id\": \"sample_01\",\n{body}\n}}\n"
    )
}

#[test]
fn a_valid_sample_has_no_diagnostics() {
    let text = sample_text(
        1,
        "  \"name\": \"a\",\n  \"points\": [{\"x\": 1, \"y\": 2}],\n  \"job_spec\": {}",
    );
    let checked = check_sample("a.sample", &text);
    assert_eq!(codes_and_paths(&checked.diagnostics), vec![]);
    assert_eq!(checked.type_name, SAMPLE_TYPE);
    assert_eq!(checked.value.unwrap()["points"][0]["y"], 2);
}

#[test]
fn an_unknown_field_is_e1003_at_its_key() {
    let checked = check_sample("a.sample", &sample_text(1, "  \"name\": \"a\",\n  \"nmae\": \"typo\""));
    assert_eq!(
        codes_and_paths(&checked.diagnostics),
        pairs(&[("RRS-E1003", "a.sample:7:3#/nmae")])
    );
    assert!(checked.value.is_none());
}

#[test]
fn every_structure_problem_is_reported_in_one_pass_with_its_path() {
    let body = "  \"mode\": \"medium\",\n  \"points\": [{\"x\": 1.5, \"z\": 0}, 3],\n  \"limits\": {\"steps\": \"ten\"},\n  \
                \"job_spec\": [],\n  \"extra\": null";
    let text = sample_text(1, body).replace("\"id\": \"sample_01\"", "\"id\": 7");
    let checked = check_sample("b.sample", &text);
    let expected = pairs(&[
        ("RRS-E1002", "b.sample:5:3#/id"),
        ("RRS-E1001", "b.sample:1:1#/name"),
        ("RRS-E1004", "b.sample:6:3#/mode"),
        ("RRS-E1002", "b.sample:7:15#/points/0/x"),
        ("RRS-E1001", "b.sample:7:14#/points/0/y"),
        ("RRS-E1003", "b.sample:7:25#/points/0/z"),
        ("RRS-E1002", "b.sample:7:34#/points/1"),
        ("RRS-E1002", "b.sample:8:14#/limits/steps"),
        ("RRS-E1002", "b.sample:9:3#/job_spec"),
        ("RRS-E1003", "b.sample:10:3#/extra"),
    ]);
    assert_eq!(codes_and_paths(&checked.diagnostics), expected);
}

#[test]
fn a_type_field_that_disagrees_with_the_extension_is_e1005_and_the_body_is_still_checked() {
    let text = sample_text(1, "  \"nmae\": \"typo\"").replace("\"test.sample\"", "\"test.other\"");
    let checked = check_sample("c.sample", &text);
    let expected = pairs(&[
        ("RRS-E1005", "c.sample:3:3#/type"),
        ("RRS-E1001", "c.sample:1:1#/name"),
        ("RRS-E1003", "c.sample:6:3#/nmae"),
    ]);
    assert_eq!(codes_and_paths(&checked.diagnostics), expected);
    assert_eq!(checked.type_name, "test.other");
}

#[test]
fn a_newer_file_is_read_only_and_keeps_the_fields_this_version_does_not_know() {
    let checked = check_sample(
        "d.sample",
        &sample_text(2, "  \"name\": \"a\",\n  \"added_in_v2\": true"),
    );
    assert_eq!(
        codes_and_paths(&checked.diagnostics),
        pairs(&[("RRS-E4001", "d.sample:4:3#/schema_version")])
    );
    assert!(checked.read_only);
    assert_eq!(checked.migrated_from_version, 0);
}

#[test]
fn a_file_with_no_matching_type_is_checked_against_the_header_alone() {
    let checked = check_sample("e.json", "{\"id\": \"x\", \"body\": 1}");
    let expected = pairs(&[
        ("RRS-E1001", "e.json:1:1#/$schema"),
        ("RRS-E1001", "e.json:1:1#/type"),
        ("RRS-E1001", "e.json:1:1#/schema_version"),
    ]);
    assert_eq!(codes_and_paths(&checked.diagnostics), expected);
    assert_eq!(checked.type_name, "");
}
