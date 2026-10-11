//! Fuzz target for the strict JSON reader (contracts 12.5): arbitrary bytes never panic, and every document the
//! reader accepts has a canonical form that reads back to the same canonical form. That second check also covers
//! canonical.rs: its number and string output must always be JSON the strict reader accepts.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_formats::json::canonical::canonical_json;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;

fuzz_target!(|bytes: &[u8]| {
    let limits = ImportLimits::default();
    let Ok(parsed) = read_json(bytes, &limits) else {
        return;
    };
    let canonical = canonical_json(&parsed.value);
    // Canonical text can be a little longer than its source ("1e300" becomes "1e+300"), so only size is relaxed.
    let reread_limits = ImportLimits {
        max_file_bytes: u64::MAX,
        ..limits
    };
    let reread = match read_json(canonical.as_bytes(), &reread_limits) {
        Ok(reread) => reread,
        Err(error) => panic!("canonical JSON was refused: {error:?} for {canonical}"),
    };
    assert_eq!(canonical_json(&reread.value), canonical);
});
