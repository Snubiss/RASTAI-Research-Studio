//! Fuzz target for the DEFLATE wrapper (contracts 12.5, ADR-0037): the first four bytes give the declared size and the
//! rest is a raw DEFLATE stream, fed in one chunk at a time as the reader does. The wrapper never panics, and it never
//! writes more bytes than the declared size, so a lying stream can't inflate past its cap.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_formats::byte_source::VecSink;
use rastai_formats::zip::inflate::{inflate_chunk, new_inflate_stream};

const DECLARED_SIZE_BYTES: usize = 4;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() < DECLARED_SIZE_BYTES {
        return;
    }
    let mut declared = [0u8; DECLARED_SIZE_BYTES];
    declared.copy_from_slice(&bytes[..DECLARED_SIZE_BYTES]);
    let declared_size = u64::from(u32::from_le_bytes(declared));
    let stream_bytes = &bytes[DECLARED_SIZE_BYTES..];
    let mut stream = new_inflate_stream(declared_size);
    let mut sink = VecSink { bytes: Vec::new() };
    let mut at = 0;
    while at < stream_bytes.len() {
        let Ok(progress) = inflate_chunk(&mut stream, &stream_bytes[at..], &mut sink) else {
            break;
        };
        at += progress.consumed;
        if progress.finished {
            break;
        }
    }
    assert!(sink.bytes.len() as u64 <= declared_size, "wrote past the declared size");
});
