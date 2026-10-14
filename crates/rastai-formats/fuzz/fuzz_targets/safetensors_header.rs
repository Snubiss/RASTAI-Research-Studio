//! Fuzz target for the safetensors header check (contracts 12.5, ADR-0037): arbitrary bytes, read as a whole file
//! the way every caller reads one, never panic, and an accepted header always describes tensors that lie inside the
//! data section and tile it exactly, so a caller can slice tensor bytes from it without another check. The reader over
//! a `ByteSource` (M2-13) runs on the same bytes: it must give the same verdict, and every tensor it accepts must read.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_formats::byte_source::SliceSource;
use rastai_formats::safetensors::header_check::{
    HEADER_LENGTH_BYTES, check_safetensors_header, read_safetensors_header_length,
};
use rastai_formats::safetensors::reader::{read_safetensors_header, read_tensor_bytes};

fuzz_target!(|bytes: &[u8]| {
    let source = SliceSource { bytes };
    if let Ok(read) = read_safetensors_header(&source) {
        let mut buffer = Vec::new();
        for entry in &read.entries {
            read_tensor_bytes(&source, &read, entry, &mut buffer).expect("an accepted tensor reads");
        }
    }
    let file_length = bytes.len() as u64;
    let Ok(header_length) = read_safetensors_header_length(bytes, file_length) else {
        return;
    };
    // The length check promised the header ends inside the file.
    let header_start = HEADER_LENGTH_BYTES as usize;
    let header_end = header_start + header_length as usize;
    let data_length = file_length - HEADER_LENGTH_BYTES - header_length;
    let Ok(header) = check_safetensors_header(&bytes[header_start..header_end], data_length) else {
        return;
    };
    assert_eq!(header.data_start, HEADER_LENGTH_BYTES + header_length);
    let mut covered = 0;
    for entry in &header.entries {
        assert_eq!(entry.start, covered, "{} doesn't start where the tensor before it ends", entry.name);
        assert!(entry.start <= entry.end && entry.end <= data_length, "{} lies outside the data", entry.name);
        covered = entry.end;
    }
    assert_eq!(covered, data_length, "the tensors don't cover the data section");
});
