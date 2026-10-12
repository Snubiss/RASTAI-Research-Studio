//! Fuzz target for the in-house semver reader (contracts 12.5, ADR-0037; threat model R20): plugin manifests carry
//! `engines.rastai`, `version` and `dependencies` ranges, and the plugin host reads them from untrusted files. Any
//! text, split at its first line break into a range and a version, never panics; a version that is read compares
//! equal to itself and to its own text read again; and a range check of a read version answers or refuses, never
//! anything else.

#![no_main]

use std::cmp::Ordering;

use libfuzzer_sys::fuzz_target;
use rastai_formats::semver::{compare_versions, parse_version, version_matches};

const RANGE_END: char = '\n';

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let (range, version_text) = text.split_once(RANGE_END).unwrap_or((text, text));
    let Ok(version) = parse_version(version_text) else {
        return;
    };
    assert_eq!(compare_versions(&version, &version), Ordering::Equal);
    let again = parse_version(version_text).unwrap_or_else(|error| panic!("read once, refused again: {error:?}"));
    assert_eq!(again, version);
    // Either answer is fine; only a panic is a failure.
    let _ = version_matches(range, &version);
});
