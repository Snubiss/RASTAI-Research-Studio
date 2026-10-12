//! The document corpus (contracts 12.5, layout 4.11): every file under `fixtures/documents/<folder>/valid/` and
//! `.../invalid/` goes through `read_document`. A valid file must give no error and its typed document; an invalid
//! file must give exactly the codes and paths its `<name>.expected.json` lists, in order, all from one read, layer 3
//! included. fixtures/documents/README.md describes the folders and the expected-file format.
//!
//! The built-in design templates (`templates/designs/`) must read with no diagnostic at all.
//!
//! The second half of this file runs layers 1 and 2 through `check_document` with a test type of its own, which has
//! every kind of field, so the walk is tested apart from any real document's description.

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::{Diagnostic, has_errors};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};
use rastai_formats::documents::model::ModelSpec;
use rastai_formats::documents::{Document, DocumentType};
use rastai_formats::header::{describe_shared_header, header_fields};
use rastai_formats::json::pretty::pretty_json;
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
        if has_errors(&outcome.diagnostics) {
            return Some(message);
        }
        let missing = format!("{}: valid, but no typed document came back", fixture.display());
        return outcome.document.is_none().then_some(missing);
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

/// The messages a person reads, for the two cases whose values have to be reworded: serde's refusal of an integer
/// out of range (no Rust type names) and a blank value (a word, not a leading space).
fn corpus_messages(relative_path: &str) -> Vec<String> {
    let fixture = corpus_root().join(relative_path);
    let bytes = fs::read(&fixture).unwrap();
    let outcome = read_document(
        &file_name_of(&fixture),
        &SliceSource { bytes: &bytes },
        &ImportLimits::default(),
    );
    outcome.diagnostics.into_iter().map(|item| item.message).collect()
}

#[test]
fn a_negative_step_reads_as_a_sentence_without_rust_types() {
    let messages = corpus_messages("rastai.resume/invalid/negative_step.json");
    assert_eq!(
        messages,
        vec!["A number in this file should be a whole number of zero or more that fits its field, but it is -1."]
    );
}

#[test]
fn a_blank_project_name_is_shown_as_a_word() {
    let messages = corpus_messages("rastai.project/invalid/blank_name_and_host_name.json");
    assert_eq!(
        messages[0],
        "(blank) isn't a project name: it needs at least one letter, digit or other visible character."
    );
}

// The built-in design templates (templates/designs/, contracts 1): each reads with no diagnostic at all, as a
// ModelSpec, and is written in the stable form, so saving an unchanged template changes nothing in git.

const TEMPLATES_FOLDER: &str = "../../templates/designs";
const TEXTCNN_TEMPLATE: &str = "textcnn.rrsmodel";

fn template_files() -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join(TEMPLATES_FOLDER);
    fixture_files(&folder)
}

fn read_template(path: &Path) -> (Vec<u8>, ModelSpec) {
    let bytes = fs::read(path).unwrap();
    let outcome = read_document(
        &file_name_of(path),
        &SliceSource { bytes: &bytes },
        &ImportLimits::default(),
    );
    assert_eq!(codes_and_paths(&outcome.diagnostics), vec![], "{}", path.display());
    let Some(Document::Model(model)) = outcome.document else {
        panic!("{} is not a ModelSpec", path.display());
    };
    (bytes, model)
}

#[test]
fn every_design_template_reads_with_no_diagnostics_in_the_stable_form() {
    let files = template_files();
    assert!(files.iter().any(|path| file_name_of(path) == TEXTCNN_TEMPLATE));
    for path in files {
        let (bytes, model) = read_template(&path);
        assert_eq!(
            pretty_json(&model).unwrap(),
            bytes,
            "{} isn't in the stable form",
            path.display()
        );
    }
}

/// The owner's 5-layer TextCNN (AITestLab-1 `create_model.py`): five conv1d of 64 filters with kernels 3, 4, 5, 4, 3
/// and padding kernel // 2, global max pooling, dropout 0.5 and 4 classes.
#[test]
fn the_textcnn_template_is_the_owners_five_layer_model() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(TEMPLATES_FOLDER)
        .join(TEXTCNN_TEMPLATE);
    let (_, model) = read_template(&path);
    let convs: Vec<(u64, u64, u64)> = model
        .graph
        .nodes
        .iter()
        .filter(|node| node.op == "conv1d")
        .map(|node| {
            let size = |key: &str| node.attributes[key].as_u64().unwrap();
            (size("out_channels"), size("kernel_size"), size("padding"))
        })
        .collect();
    assert_eq!(convs, vec![(64, 3, 1), (64, 4, 2), (64, 5, 2), (64, 4, 2), (64, 3, 1)]);
    let classifier = model
        .graph
        .nodes
        .iter()
        .find(|node| node.op == "classifier_head")
        .unwrap();
    assert_eq!(classifier.attributes["class_count"], 4);
    assert_eq!(model.outputs[0].graph_node, classifier.node_id);
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
