//! The strict reader's memory on the costliest shape found under the default `ImportLimits` (ADR-0032 decision 8):
//! chains of one-member objects, `[{"a":{"a":…{}…}},…]`, filled up to the value cap. Almost every value is a
//! non-empty object, and an object is the dearest value: serde_json gives each one a BTreeMap node of about 600
//! bytes, and the position map adds a member list and a copy of the key. limits.rs records the measured figures.
//!
//! Peak memory is read from Linux's `VmHWM` (the process's resident high-water mark). This is its own test file,
//! so its own process: reader_memory.rs measures another shape, and one high-water mark can't serve two tests.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use rastai_formats::json::positions::{find_position, position_of};
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;

// One chain is this many objects, each the only member of the one before; the last is `{}`. With the outer array
// the document nests 121 deep, under the depth cap of 128.
const CHAIN_OBJECTS: usize = 120;
const CHAIN_KEY: &str = "a";
// Measured in a release build: 741 MiB, of which serde_json's own tree is 635 MiB. A position map with a BTreeMap
// per object took 1.33 GiB on the same document, and the old value cap of 2 million let it reach 1.45 GiB.
const MAX_PEAK_GROWTH_BYTES: u64 = 1024 * 1024 * 1024;

/// `{"a":{"a":…{}…}}`, CHAIN_OBJECTS objects deep.
fn one_member_chain() -> String {
    let opening = format!("{{\"{CHAIN_KEY}\":");
    let mut chain = opening.repeat(CHAIN_OBJECTS - 1);
    chain.push_str("{}");
    chain.push_str(&"}".repeat(CHAIN_OBJECTS - 1));
    chain
}

/// An array of as many chains as the value cap lets through; the array itself is one value.
fn chains_at_the_value_cap(chain_count: usize) -> Vec<u8> {
    let chain = one_member_chain();
    let mut text = String::from("[");
    for index in 0..chain_count {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&chain);
    }
    text.push(']');
    text.into_bytes()
}

fn peak_resident_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|line| line.starts_with("VmHWM:")).unwrap();
    let kibibytes: u64 = line
        .trim_start_matches("VmHWM:")
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .unwrap();
    kibibytes * 1024
}

#[test]
fn one_member_object_chains_at_the_value_cap_stay_under_a_gibibyte() {
    let limits = ImportLimits::default();
    let chain_count = (limits.max_json_values as usize - 1) / CHAIN_OBJECTS;
    let document = chains_at_the_value_cap(chain_count);
    let peak_before = peak_resident_bytes();
    let parsed = read_json(&document, &limits).unwrap();
    let growth = peak_resident_bytes() - peak_before;
    assert!(
        growth < MAX_PEAK_GROWTH_BYTES,
        "reading {} bytes raised peak memory by {growth} bytes",
        document.len()
    );
    // The innermost object of the last chain is found, and a missing member falls back to it.
    let last_chain = chain_count - 1;
    let innermost = format!("/{last_chain}") + &format!("/{CHAIN_KEY}").repeat(CHAIN_OBJECTS - 1);
    let position = position_of(&parsed.positions, &innermost).unwrap();
    assert_eq!(position.line, 1);
    let missing_member = format!("{innermost}/missing");
    assert_eq!(find_position(&parsed.positions, &missing_member), Some(position));
}
