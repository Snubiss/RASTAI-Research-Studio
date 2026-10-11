//! Fuzz target for `parse_rfc3339` (contracts 12.5): timestamps arrive in user files, so arbitrary text never
//! panics the parser (or overflows its arithmetic), and the text `format_rfc3339` writes for any accepted instant
//! always parses back to text that formats the same way.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_core::time::{format_rfc3339, parse_rfc3339};

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(unix_ms) = parse_rfc3339(text) else {
        return;
    };
    // An offset can move an instant just outside the years 0000 to 9999; formatting clamps it, so the check is on
    // the formatted text, not on `unix_ms`.
    let formatted = format_rfc3339(unix_ms);
    let reparsed = match parse_rfc3339(&formatted) {
        Ok(reparsed) => reparsed,
        Err(error) => panic!("formatted text was refused: {error:?}"),
    };
    assert_eq!(format_rfc3339(reparsed), formatted);
});
