//! Read, write, read: equal values and a stable content hash (layout 4.11).
//!
//! The first half writes seeded random headers from an in-house generator (no property-testing crate) and reads
//! them through `check_document` with a test type whose description is the shared header alone. The second half
//! takes every valid file of the document corpus through `read_document`: the typed document, written with the
//! stable writer and read again, is equal, writes the same bytes and has the same content hash.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rastai_formats::byte_source::SliceSource;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::documents::{Document, DocumentType};
use rastai_formats::error::FormatsError;
use rastai_formats::hashing::content_hash_of_value;
use rastai_formats::header::{SHARED_HEADER_TYPE_NAME, SharedHeader};
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{CheckedDocument, check_document, read_document};
use serde::Serialize;
use serde_json::Value;

const HEADER_ONLY_TYPE: &str = "test.header_only";
const HEADER_ONLY_FILE: &str = "sample.headeronly";
const TEST_TYPES: &[DocumentType] = &[DocumentType {
    type_name: HEADER_ONLY_TYPE,
    schema_version: 1,
    file_extensions: &[".headeronly"],
    description_name: SHARED_HEADER_TYPE_NAME,
}];

const SEED: u64 = 0x5EED_0015;
/// The content hash of the first header SEED generates.
const GOLDEN_FIRST_HEADER_HASH: &str = "blake3:7a65b0277f61d6682d3ad0fe96ce82280c0fd8cb6de2ccb869fbd275bbb67314";
const DOCUMENT_COUNT: usize = 200;
/// The corpus's valid files, one folder per document type.
const CORPUS_FOLDER: &str = "../../fixtures/documents";
const VALID_FOLDER: &str = "valid";
/// The content hash of `fixtures/documents/rastai.theme/valid/night.theme.json` as read. A change here changes the
/// stored hash of every theme (contracts 5.1).
const NIGHT_THEME_FILE: &str = "rastai.theme/valid/night.theme.json";
const GOLDEN_NIGHT_THEME_HASH: &str = "blake3:de79ce5ff11222694df1eae654fc5a2b8351062faaee8733a3c987a1d107ba7e";
// Text pieces that stress the writer and the reader: quotes, escapes, a pointer separator, non-ASCII and an
// astral-plane character (a surrogate pair in JSON escapes).
const TEXT_PIECES: &[&str] = &[
    "a", "Zeta", " ", "\"", "\\", "/", "~", "\n", "é", "東京", "😀", "\u{1}", "{}", "0",
];

// The same 64-bit LCG as src/test_random.rs (Knuth's MMIX constants), which integration tests can't reach.
const LCG_MULTIPLIER: u64 = 6364136223846793005;
const LCG_INCREMENT: u64 = 1442695040888963407;
const LCG_DISCARDED_LOW_BITS: u32 = 33;

fn next_random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(LCG_MULTIPLIER).wrapping_add(LCG_INCREMENT);
    *seed >> LCG_DISCARDED_LOW_BITS
}

fn random_below(seed: &mut u64, bound: u64) -> usize {
    (next_random(seed) % bound) as usize
}

fn random_text(seed: &mut u64) -> String {
    let piece_count = random_below(seed, 6);
    let mut text = String::new();
    for _ in 0..piece_count {
        text.push_str(TEXT_PIECES[random_below(seed, TEXT_PIECES.len() as u64)]);
    }
    text
}

fn random_texts(seed: &mut u64) -> Vec<String> {
    let count = random_below(seed, 4);
    (0..count).map(|_| random_text(seed)).collect()
}

fn random_header(seed: &mut u64) -> SharedHeader {
    let mut requires = BTreeMap::new();
    for _ in 0..random_below(seed, 3) {
        requires.insert(random_text(seed), random_text(seed));
    }
    SharedHeader {
        schema_url: random_text(seed),
        document_type: HEADER_ONLY_TYPE.to_string(),
        schema_version: 1,
        created_with: random_text(seed),
        id: random_text(seed),
        content_hash: String::new(),
        derived_from: random_texts(seed),
        authors: random_texts(seed),
        license: random_text(seed),
        source_url: random_text(seed),
        requires,
    }
}

fn read_back(bytes: &[u8]) -> CheckedDocument {
    let source = SliceSource { bytes };
    check_document(
        HEADER_ONLY_FILE,
        &source,
        &ImportLimits::default(),
        TEST_TYPES,
        &describe_all_types(),
    )
}

/// Writes a header, reads it back, and checks the value, the struct, the bytes and the hash all survive.
fn assert_round_trip(header: &SharedHeader) {
    let written = pretty_json(header).unwrap();
    let checked = read_back(&written);
    assert_eq!(checked.diagnostics, vec![], "{}", String::from_utf8_lossy(&written));
    assert_eq!(checked.type_name, HEADER_ONLY_TYPE);
    let value = checked.value.unwrap();
    let read_header: SharedHeader = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(&read_header, header);
    assert_eq!(
        pretty_json(&read_header).unwrap(),
        written,
        "writing again changes the bytes"
    );
    assert_eq!(
        content_hash_of_value(&value),
        content_hash_of_value(&serde_json::to_value(header).unwrap())
    );
}

#[test]
fn seeded_random_headers_survive_write_and_read() {
    let mut seed = SEED;
    for _ in 0..DOCUMENT_COUNT {
        assert_round_trip(&random_header(&mut seed));
    }
}

/// Compact JSON text with every object's members in reverse key order. serde_json keeps object members sorted by
/// key, so a different order exists only in text; this writes one by hand.
fn reversed_compact_text(value: &Value) -> String {
    match value {
        Value::Object(members) => {
            let mut parts = Vec::new();
            for (key, member) in members.iter().rev() {
                let key_text = serde_json::to_string(key).unwrap();
                parts.push(format!("{key_text}:{}", reversed_compact_text(member)));
            }
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(elements) => {
            let parts: Vec<String> = elements.iter().map(reversed_compact_text).collect();
            format!("[{}]", parts.join(","))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}

#[test]
fn the_content_hash_ignores_its_own_field_spacing_and_key_order() {
    let mut seed = SEED;
    let mut header = random_header(&mut seed);
    let value = serde_json::to_value(&header).unwrap();
    let hash = content_hash_of_value(&value);
    header.content_hash = hash.clone();
    let written = pretty_json(&header).unwrap();
    let reread = read_back(&written).value.unwrap();
    assert_eq!(content_hash_of_value(&reread), hash);
    // The same members, compact and in reverse key order, read back and hash the same.
    let reordered = reversed_compact_text(&reread);
    assert_ne!(reordered.as_bytes(), written.as_slice());
    assert!(
        reordered.starts_with("{\"type\""),
        "the text must really be in another order: {reordered}"
    );
    let reordered_value = read_back(reordered.as_bytes()).value.unwrap();
    assert_eq!(content_hash_of_value(&reordered_value), hash);
}

#[test]
fn hashes_are_stable_across_runs_and_machines() {
    let mut seed = SEED;
    let header = random_header(&mut seed);
    let mut seed_again = SEED;
    assert_eq!(
        header,
        random_header(&mut seed_again),
        "the generator must be deterministic"
    );
    // Written down once from this test's first run: a change here changes every stored content hash (contracts 5.1).
    let hash = content_hash_of_value(&serde_json::to_value(&header).unwrap());
    assert_eq!(hash, GOLDEN_FIRST_HEADER_HASH);
}

/// Every valid corpus file, sorted.
fn valid_corpus_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS_FOLDER);
    let mut files = Vec::new();
    for type_folder in fs::read_dir(&root).unwrap() {
        let valid_folder = type_folder.unwrap().path().join(VALID_FOLDER);
        let Ok(entries) = fs::read_dir(&valid_folder) else {
            continue;
        };
        files.extend(entries.map(|entry| entry.unwrap().path()));
    }
    files.sort();
    files
}

fn read_typed(file_name: &str, bytes: &[u8]) -> Document {
    let outcome = read_document(file_name, &SliceSource { bytes }, &ImportLimits::default());
    assert_eq!(outcome.diagnostics, vec![], "{file_name}");
    outcome.document.unwrap()
}

/// The stable pretty text of a typed document and its JSON value.
fn written(document: &Document) -> (Vec<u8>, Value) {
    match document {
        Document::RunRecord(run_record) => text_and_value(run_record),
        Document::Resume(resume) => text_and_value(resume),
        Document::Settings(settings) => text_and_value(settings),
        Document::Project(project) => text_and_value(project),
        Document::Lock(lock) => text_and_value(lock),
        Document::PluginManifest(manifest) => text_and_value(manifest),
        Document::Theme(theme) => text_and_value(theme),
    }
}

fn text_and_value<T: Serialize>(typed: &T) -> (Vec<u8>, Value) {
    let text: Result<Vec<u8>, FormatsError> = pretty_json(typed);
    (text.unwrap(), serde_json::to_value(typed).unwrap())
}

#[test]
fn every_valid_corpus_document_survives_write_and_read() {
    let files = valid_corpus_files();
    assert!(files.len() >= 7, "every step-1 type has a valid file: {files:?}");
    for path in files {
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let first = read_typed(&file_name, &fs::read(&path).unwrap());
        let (first_text, first_value) = written(&first);
        let second = read_typed(&file_name, &first_text);
        assert_eq!(second, first, "{file_name}: the typed document changed");
        let (second_text, second_value) = written(&second);
        assert_eq!(second_text, first_text, "{file_name}: writing again changes the bytes");
        assert_eq!(
            content_hash_of_value(&second_value),
            content_hash_of_value(&first_value),
            "{file_name}"
        );
    }
}

#[test]
fn a_theme_hash_is_stable_across_runs_and_machines() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CORPUS_FOLDER)
        .join(NIGHT_THEME_FILE);
    let theme = read_typed("night.theme.json", &fs::read(path).unwrap());
    let (_, value) = written(&theme);
    assert_eq!(content_hash_of_value(&value), GOLDEN_NIGHT_THEME_HASH);
}
