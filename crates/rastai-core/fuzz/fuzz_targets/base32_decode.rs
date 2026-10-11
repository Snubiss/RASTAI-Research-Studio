//! Fuzz target for `decode_base32` (contracts 12.5): ids and node ids arrive in files and requests, so arbitrary
//! text never panics the decoder, and whatever it accepts is the one canonical spelling of its bytes: encoding the
//! decoded bytes gives back the same text.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rastai_core::crockford::{decode_base32, encode_base32};

// The byte counts ids and node ids use (contracts 8.6), then one the input picks, so other lengths are reached too.
const ID_BYTE_COUNTS: [usize; 2] = [16, 32];
const LARGEST_PICKED_BYTE_COUNT: usize = 40;

fuzz_target!(|bytes: &[u8]| {
    let Some((first_byte, rest)) = bytes.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let picked_byte_count = usize::from(*first_byte) % (LARGEST_PICKED_BYTE_COUNT + 1);
    let mut byte_counts = ID_BYTE_COUNTS.to_vec();
    byte_counts.push(picked_byte_count);
    for byte_count in byte_counts {
        let Some(decoded) = decode_base32(text, byte_count) else {
            continue;
        };
        assert_eq!(decoded.len(), byte_count);
        assert_eq!(encode_base32(&decoded), text);
    }
});
