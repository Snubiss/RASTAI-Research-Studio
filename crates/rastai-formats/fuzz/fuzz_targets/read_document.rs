//! Fuzz target for `read_document` (contracts 12.5): arbitrary bytes, read once per known extension and once as a
//! plain `.json` file, never panic, and every outcome keeps the promises callers rely on.
//!
//! No document type is registered before M1-17, so `read_document` alone stops after the header. The same bytes
//! also go through `check_document` with a fuzz-only type that has every nested kind (Type, List, Map, Enum and a
//! type that holds itself), so the layer-2 body walk is fuzzed too, as tests/documents_corpus.rs tests it.

#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use rastai_diag::diagnostic::{Diagnostic, has_errors};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::describe::types::{FieldKind, OPTIONAL, REQUIRED, TypeDescription, enum_of, field, list_of};
use rastai_formats::documents::{DOCUMENT_TYPES, DocumentType};
use rastai_formats::header::{describe_shared_header, header_fields};
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{check_document, read_document};

const FILE_STEM: &str = "fuzz";
// Documents found by their place (run.json, settings.json) have no extension of their own.
const PLAIN_EXTENSION: &str = ".json";
const FORMATS_CODE_PREFIX: &str = "RRS-E";

const FUZZ_TYPE_NAME: &str = "fuzz.sample";
const FUZZ_FILE_NAME: &str = "fuzz.sample";
const FUZZ_DESCRIPTION: &str = "FuzzSample";
const FUZZ_NODE: &str = "FuzzNode";
const FUZZ_TYPES: &[DocumentType] = &[DocumentType {
    type_name: FUZZ_TYPE_NAME,
    schema_version: 1,
    file_extensions: &[".sample"],
    description_name: FUZZ_DESCRIPTION,
}];

/// One file name per extension `read_document` treats differently.
fn file_names() -> Vec<String> {
    let mut names = vec![format!("{FILE_STEM}{PLAIN_EXTENSION}")];
    for document_type in DOCUMENT_TYPES {
        for extension in document_type.file_extensions {
            names.push(format!("{FILE_STEM}{extension}"));
        }
    }
    names
}

/// The fuzz type's descriptions: the header, a body with every kind of field, and a node that lists itself, so
/// the walk goes as deep as the reader's nesting limit allows.
fn fuzz_descriptions() -> Vec<TypeDescription> {
    let mut sample_fields = header_fields();
    sample_fields.extend([
        field("name", FieldKind::String, REQUIRED, "Name."),
        field("mode", enum_of(&["fast", "slow"]), OPTIONAL, "Mode."),
        field("ratio", FieldKind::Number, OPTIONAL, "Ratio."),
        field("enabled", FieldKind::Boolean, OPTIONAL, "Enabled."),
        field("settings", FieldKind::Json, OPTIONAL, "Settings."),
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
        field("root", FieldKind::Type(FUZZ_NODE.to_string()), OPTIONAL, "Tree."),
    ]);
    let sample = TypeDescription {
        name: FUZZ_DESCRIPTION.to_string(),
        schema_id: "fuzz.sample/1".to_string(),
        doc: "A fuzz document.".to_string(),
        fields: sample_fields,
    };
    let node = TypeDescription {
        name: FUZZ_NODE.to_string(),
        schema_id: "fuzz.sample/1".to_string(),
        doc: "A tree node.".to_string(),
        fields: vec![
            field("value", FieldKind::Integer, REQUIRED, "Value."),
            field(
                "children",
                list_of(FieldKind::Type(FUZZ_NODE.to_string())),
                OPTIONAL,
                "Children.",
            ),
        ],
    };
    vec![describe_shared_header(), sample, node]
}

/// Built once: the target runs millions of times and the descriptions never change.
fn fuzz_descriptions_once() -> &'static [TypeDescription] {
    static DESCRIPTIONS: OnceLock<Vec<TypeDescription>> = OnceLock::new();
    DESCRIPTIONS.get_or_init(fuzz_descriptions)
}

/// Every diagnostic is a formats code and says which file it is about.
fn check_diagnostics(file_name: &str, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        assert!(diagnostic.code.starts_with(FORMATS_CODE_PREFIX), "{diagnostic:?}");
        assert!(diagnostic.path.starts_with(file_name), "{diagnostic:?}");
    }
}

fuzz_target!(|bytes: &[u8]| {
    let limits = ImportLimits::default();
    let source = SliceSource { bytes };
    for file_name in file_names() {
        let outcome = read_document(&file_name, &source, &limits);
        // A typed document only ever comes without errors.
        assert!(outcome.document.is_none() || !has_errors(&outcome.diagnostics));
        check_diagnostics(&file_name, &outcome.diagnostics);
    }
    let checked = check_document(FUZZ_FILE_NAME, &source, &limits, FUZZ_TYPES, fuzz_descriptions_once());
    // A checked value only ever comes without errors.
    assert!(checked.value.is_none() || !has_errors(&checked.diagnostics));
    check_diagnostics(FUZZ_FILE_NAME, &checked.diagnostics);
});
