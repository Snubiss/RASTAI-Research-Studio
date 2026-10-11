//! Fuzz target for `read_document` (contracts 12.5): arbitrary bytes, read once per known extension and once as a
//! plain `.json` file, never panic, and every outcome keeps the promises callers rely on.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_diag::diagnostic::has_errors;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::DOCUMENT_TYPES;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::read_document;

const FILE_STEM: &str = "fuzz";
// Documents found by their place (run.json, settings.json) have no extension of their own.
const PLAIN_EXTENSION: &str = ".json";
const FORMATS_CODE_PREFIX: &str = "RRS-E";

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

fuzz_target!(|bytes: &[u8]| {
    let limits = ImportLimits::default();
    let source = SliceSource { bytes };
    for file_name in file_names() {
        let outcome = read_document(&file_name, &source, &limits);
        // A typed document only ever comes without errors.
        assert!(outcome.document.is_none() || !has_errors(&outcome.diagnostics));
        for diagnostic in &outcome.diagnostics {
            // Every diagnostic is a formats code and says which file it is about.
            assert!(diagnostic.code.starts_with(FORMATS_CODE_PREFIX), "{diagnostic:?}");
            assert!(diagnostic.path.starts_with(&file_name), "{diagnostic:?}");
        }
    }
});
