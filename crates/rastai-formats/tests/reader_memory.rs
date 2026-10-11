//! The strict reader's memory on hostile documents, under the default `ImportLimits` (ADR-0032 decision 8: limits
//! bound allocation, so a file under them can't exhaust memory, in the browser's 32-bit WASM either).
//!
//! Peak memory is read from Linux's `VmHWM` (the process's resident high-water mark), so this file holds one test:
//! a second test running in parallel would raise the same mark.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use rastai_formats::json::positions::{find_position, position_of};
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;

const NESTING_LEVELS: usize = 63;
const KEY_BYTES: usize = 256 * 1024;
// The document is 16 MiB. A position map keyed by full pointer strings copied every parent's keys into every
// child, which took 1.1 GB here; the tree stores each key once and takes under 100 MiB.
const MAX_PEAK_GROWTH_BYTES: u64 = 256 * 1024 * 1024;

/// `{"kkk…": {"kkk…": … 0 …}}`: long keys, deeply nested.
fn deep_document_with_long_keys() -> Vec<u8> {
    let key = "k".repeat(KEY_BYTES);
    let mut text = String::new();
    for _ in 0..NESTING_LEVELS {
        text.push_str("{\"");
        text.push_str(&key);
        text.push_str("\": ");
    }
    text.push('0');
    text.push_str(&"}".repeat(NESTING_LEVELS));
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
fn deep_long_keys_cost_memory_linear_in_the_document() {
    let document = deep_document_with_long_keys();
    let peak_before = peak_resident_bytes();
    let parsed = read_json(&document, &ImportLimits::default()).unwrap();
    let growth = peak_resident_bytes() - peak_before;
    assert!(
        growth < MAX_PEAK_GROWTH_BYTES,
        "reading {} bytes raised peak memory by {growth} bytes",
        document.len()
    );
    // The innermost value is still found, KEY_BYTES * 63 characters into its pointer.
    let key = "k".repeat(KEY_BYTES);
    let innermost = format!("/{key}").repeat(NESTING_LEVELS);
    let position = position_of(&parsed.positions, &innermost).unwrap();
    assert_eq!(position.line, 1);
    let missing_child = format!("{innermost}/missing");
    assert_eq!(find_position(&parsed.positions, &missing_child), Some(position));
}
