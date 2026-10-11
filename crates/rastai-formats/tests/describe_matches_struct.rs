//! Every description lists exactly the fields its struct's serde writes, with matching kinds (layout 4.11). A
//! description that drifts from its struct would let layer 2 pass a file that serde then refuses, or refuse a file
//! serde would read; this test catches both in CI.
//!
//! Each type is checked with a filled value: every optional field set, so serde writes every key. A document type's
//! PR adds one test here with its own filled value.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use rastai_formats::describe::registry::{describe_all_types, find_description};
use rastai_formats::describe::structure_check::{StructureCheck, find_mismatches};
use rastai_formats::header::{SHARED_HEADER_TYPE_NAME, SharedHeader};
use rastai_formats::json::pointer::join_pointer;
use rastai_formats::json::positions::position_of;
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The checks every type gets: same keys, every value of its described kind, the written order is the description
/// order, and the written value reads back into an equal struct.
fn assert_description_matches<T: Serialize + DeserializeOwned + PartialEq + Debug>(filled: &T, type_name: &str) {
    let all_types = describe_all_types();
    let description = find_description(&all_types, type_name).unwrap();
    let written = serde_json::to_value(filled).unwrap();

    let mut written_keys: Vec<&str> = written.as_object().unwrap().keys().map(String::as_str).collect();
    let mut described_keys: Vec<&str> = description.fields.iter().map(|field| field.name.as_str()).collect();
    written_keys.sort_unstable();
    described_keys.sort_unstable();
    assert_eq!(
        written_keys, described_keys,
        "{type_name}: serde keys and described fields differ"
    );

    let check = StructureCheck {
        all_types: &all_types,
        report_unknown_fields: true,
    };
    assert_eq!(
        find_mismatches(&written, description, &check),
        vec![],
        "{type_name}: kinds differ"
    );

    assert_written_in_description_order(filled, type_name);
    let read_back: T = serde_json::from_value(written).unwrap();
    assert_eq!(&read_back, filled);
}

/// Files list fields in the struct's order (json/pretty.rs) and the codegen in the description's; they must agree,
/// so each described field must start on a later line of the written file than the one before it.
fn assert_written_in_description_order<T: Serialize>(filled: &T, type_name: &str) {
    let all_types = describe_all_types();
    let description = find_description(&all_types, type_name).unwrap();
    let text = pretty_json(filled).unwrap();
    let positions = read_json(&text, &ImportLimits::default()).unwrap().positions;
    let lines: Vec<u32> = description
        .fields
        .iter()
        .map(|field| position_of(&positions, &join_pointer("", &field.name)))
        .map(|position| position.unwrap().line)
        .collect();
    let mut sorted_lines = lines.clone();
    sorted_lines.sort_unstable();
    assert_eq!(
        lines, sorted_lines,
        "{type_name}: written order differs from description order"
    );
}

fn filled_header() -> SharedHeader {
    let mut requires = BTreeMap::new();
    requires.insert("psychology".to_string(), "^1.2".to_string());
    SharedHeader {
        schema_url: "https://schemas.example/rastai.model/1.json".to_string(),
        document_type: "rastai.model".to_string(),
        schema_version: 1,
        created_with: "rastai 0.1.0".to_string(),
        id: "model_01J".to_string(),
        content_hash: format!("blake3:{}", "0".repeat(64)),
        derived_from: vec![format!("model_00A@blake3:{}", "1".repeat(64))],
        authors: vec!["Ada".to_string()],
        license: "Apache-2.0".to_string(),
        source_url: "https://example.org/model".to_string(),
        requires,
    }
}

#[test]
fn the_shared_header_matches_its_description() {
    assert_description_matches(&filled_header(), SHARED_HEADER_TYPE_NAME);
}

#[test]
fn a_header_with_only_required_fields_still_writes_every_key() {
    let minimal: SharedHeader = serde_json::from_value(serde_json::json!({
        "$schema": "s", "type": "rastai.model", "schema_version": 1, "id": "model_1"
    }))
    .unwrap();
    let written = serde_json::to_value(&minimal).unwrap();
    let all_types = describe_all_types();
    let description = find_description(&all_types, SHARED_HEADER_TYPE_NAME).unwrap();
    for field in &description.fields {
        assert!(written.get(&field.name).is_some(), "{} is not written", field.name);
    }
    assert_eq!(written["license"], Value::String(String::new()));
}

#[test]
fn every_registered_description_has_fields() {
    for description in describe_all_types() {
        assert!(!description.fields.is_empty(), "{} has no fields", description.name);
        assert!(!description.doc.is_empty(), "{} has no doc", description.name);
    }
}
