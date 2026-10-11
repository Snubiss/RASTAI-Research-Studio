//! Read, write, read: equal values and a stable content hash (layout 4.11), over seeded random documents from an
//! in-house generator (no property-testing crate).
//!
//! No document type is registered before M1-17, so this test reads through `check_document` with one test type
//! whose description is the shared header alone. Each document type's PR adds its own round trip through
//! `read_document`.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rastai_formats::byte_source::SliceSource;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::documents::DocumentType;
use rastai_formats::hashing::content_hash_of_value;
use rastai_formats::header::{SHARED_HEADER_TYPE_NAME, SharedHeader};
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::{CheckedDocument, check_document};
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
