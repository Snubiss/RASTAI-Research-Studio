//! Fuzz target for the zip reader (contracts 12.5, ADR-0037): arbitrary bytes, listed the way every caller lists an
//! archive, never panic; and every entry the reader accepts copies out at exactly the size its directory declares.
//! The default limits stand, so the fuzzer can't make the target allocate more than the caps allow.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_formats::byte_source::{SliceSource, VecSink};
use rastai_formats::limits::ImportLimits;
use rastai_formats::zip::reader::{copy_zip_entry, list_zip_entries};

fuzz_target!(|bytes: &[u8]| {
    let source = SliceSource { bytes };
    let limits = ImportLimits::default();
    let Ok(entries) = list_zip_entries(&source, &limits) else {
        return;
    };
    for entry in &entries {
        let mut sink = VecSink { bytes: Vec::new() };
        if copy_zip_entry(&source, entry, &mut sink, &limits).is_ok() {
            assert_eq!(
                sink.bytes.len() as u64,
                entry.uncompressed_bytes,
                "{} has the wrong size",
                entry.path
            );
        }
    }
});
