//! Golden messages (layout: rust-node-gateway-proto.md section 3.5): every `fixtures/proto-golden/common/<Name>.binpb`
//! decodes into its prost struct and encodes back to exactly the same bytes. The Python and JavaScript tests compare
//! against the same files, so the three encoders agree byte for byte. prost drops fields it doesn't know, so a struct
//! that lags behind the .proto loses a field here and the bytes differ.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;

use prost::Message;
use rastai_proto::common::v1::{
    BlobRef, Diagnostic, GpuInfo, GpuTelemetry, HardwareFingerprint, HardwareProfile, NodeTelemetry, ProcessVram,
    ResourceRequest,
};

const GOLDEN_FOLDER: &str = "../../fixtures/proto-golden/common";
const BINARY_EXTENSION: &str = "binpb";
const JSON_EXTENSION: &str = "json";

/// Decodes golden bytes as one message type and encodes the message again.
type RoundTrip = fn(&[u8]) -> Vec<u8>;

/// Every message of `rastai.common.v1`, by name, with its round trip. prost has no reflection, so this list is kept
/// by hand; `every_golden_has_a_round_trip` fails when a golden file has no line here.
const COMMON_MESSAGES: &[(&str, RoundTrip)] = &[
    ("BlobRef", round_trip::<BlobRef>),
    ("Diagnostic", round_trip::<Diagnostic>),
    ("GpuInfo", round_trip::<GpuInfo>),
    ("GpuTelemetry", round_trip::<GpuTelemetry>),
    ("HardwareFingerprint", round_trip::<HardwareFingerprint>),
    ("HardwareProfile", round_trip::<HardwareProfile>),
    ("NodeTelemetry", round_trip::<NodeTelemetry>),
    ("ProcessVram", round_trip::<ProcessVram>),
    ("ResourceRequest", round_trip::<ResourceRequest>),
];

fn round_trip<M: Message + Default>(bytes: &[u8]) -> Vec<u8> {
    let message = M::decode(bytes).expect("the golden bytes decode");
    message.encode_to_vec()
}

fn read_golden_bytes(message_name: &str) -> Vec<u8> {
    let path = format!("{GOLDEN_FOLDER}/{message_name}.{BINARY_EXTENSION}");
    fs::read(&path).unwrap_or_else(|error| panic!("can't read {path}: {error}"))
}

/// The names of the golden messages in the folder: every `<Name>.json`, which each have a `<Name>.binpb` beside it.
fn golden_message_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(GOLDEN_FOLDER).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) == Some(JSON_EXTENSION) {
            names.insert(path.file_stem().unwrap().to_string_lossy().into_owned());
        }
    }
    names
}

#[test]
fn every_golden_round_trips_byte_identical() {
    for (message_name, round_trip) in COMMON_MESSAGES {
        let golden_bytes = read_golden_bytes(message_name);
        assert!(
            !golden_bytes.is_empty(),
            "{message_name}: a golden sets every field, so it is never empty"
        );
        let encoded = round_trip(&golden_bytes);
        assert_eq!(
            encoded, golden_bytes,
            "{message_name}: decode then encode changed the bytes"
        );
    }
}

#[test]
fn every_golden_has_a_round_trip() {
    let listed: BTreeSet<String> = COMMON_MESSAGES.iter().map(|(name, _)| name.to_string()).collect();
    assert_eq!(
        golden_message_names(),
        listed,
        "add the missing message to COMMON_MESSAGES, or its golden file"
    );
}
