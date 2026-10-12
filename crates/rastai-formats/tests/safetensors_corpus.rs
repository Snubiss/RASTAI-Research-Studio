//! The safetensors corpus (fixtures/safetensors/, layout 4.11): every one of the 33 files gives the verdict, tensor
//! count and code its expected.json lists. The wheel is checked against the same file from Python
//! (python/rastai_formats/tests/test_parity.py), so both languages agree by construction and by test.
//!
//! The file is read the way every caller reads one: the length from the first 8 bytes first, then the header and
//! the data section's length.

#![allow(clippy::unwrap_used)]
// check_file returns the check's own Diagnostic, as every caller gets it.
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use rastai_diag::diagnostic::Diagnostic;
use rastai_formats::safetensors::header_check::{
    HEADER_LENGTH_BYTES, SafetensorsHeader, check_safetensors_header, read_safetensors_header_length,
};
use serde_json::Value;

const EXPECTED_FILE_NAME: &str = "expected.json";
const CORPUS_SIZE: usize = 33;
const VALID_COUNT: usize = 6;

fn corpus_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/safetensors")
}

/// A whole file through both checks.
fn check_file(bytes: &[u8]) -> Result<SafetensorsHeader, Diagnostic> {
    let file_length = bytes.len() as u64;
    let header_length = read_safetensors_header_length(bytes, file_length)?;
    let header_start = HEADER_LENGTH_BYTES as usize;
    let header_end = header_start + header_length as usize;
    let data_length = file_length - HEADER_LENGTH_BYTES - header_length;
    check_safetensors_header(&bytes[header_start..header_end], data_length)
}

fn expected_verdicts() -> serde_json::Map<String, Value> {
    let text = std::fs::read_to_string(corpus_folder().join(EXPECTED_FILE_NAME)).unwrap();
    match serde_json::from_str(&text).unwrap() {
        Value::Object(verdicts) => verdicts,
        other => panic!("expected.json is not an object: {other}"),
    }
}

fn verdict_of(bytes: &[u8]) -> Value {
    match check_file(bytes) {
        Ok(header) => serde_json::json!({"accepted": true, "tensors": header.entries.len()}),
        Err(diagnostic) => serde_json::json!({"accepted": false, "code": diagnostic.code}),
    }
}

#[test]
fn every_file_gives_its_expected_verdict_and_code() {
    let verdicts = expected_verdicts();
    assert_eq!(verdicts.len(), CORPUS_SIZE);
    for (file_name, expected) in &verdicts {
        let bytes = std::fs::read(corpus_folder().join(file_name)).unwrap();
        assert_eq!(&verdict_of(&bytes), expected, "{file_name}");
    }
}

#[test]
fn six_files_are_valid_and_the_named_edge_cases_have_their_codes() {
    let verdicts = expected_verdicts();
    let valid = verdicts.values().filter(|verdict| verdict["accepted"] == true).count();
    assert_eq!(valid, VALID_COUNT);
    // The two cases contracts 10.3 and the issue name: a wrapping shape and an offset past 64 bits.
    assert_eq!(verdicts["shape_overflows_64_bits.safetensors"]["code"], "RRS-E6012");
    assert_eq!(verdicts["offset_beyond_64_bits.safetensors"]["code"], "RRS-E6011");
}

#[test]
fn every_file_in_the_folder_has_an_expected_verdict() {
    let verdicts = expected_verdicts();
    for item in std::fs::read_dir(corpus_folder()).unwrap() {
        let name = item.unwrap().file_name().to_string_lossy().to_string();
        if name.ends_with(".safetensors") {
            assert!(verdicts.contains_key(&name), "{name} has no verdict in expected.json");
        }
    }
}

#[test]
fn a_valid_file_reads_to_its_tensors_and_metadata() {
    let bytes = std::fs::read(corpus_folder().join("valid_metadata.safetensors")).unwrap();
    let header = check_file(&bytes).unwrap();
    assert_eq!(header.metadata, [("format".to_string(), "pt".to_string())]);
    assert_eq!(header.entries[0].name, "a");
    assert_eq!(header.entries[0].shape, [2, 3]);
    assert_eq!(header.data_start + header.entries[0].end, bytes.len() as u64);
}
