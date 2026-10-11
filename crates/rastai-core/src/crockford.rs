//! Crockford base32 (https://www.crockford.com/base32.html) for ids and node ids (contracts 8.6).
//!
//! The bytes are read as one big-endian number, padded with zero bits at the front to a multiple of 5, and written
//! 5 bits per character, most significant first. 16 bytes give 26 characters (the ULID layout, whose first
//! character is at most `7`), 32 bytes give 52. Because every encoding of a given byte count has the same length
//! and the alphabet is in ASCII order, comparing two encodings as text compares the bytes: that is what makes ids
//! sort by creation time.
//!
//! Decoding is strict: upper-case canonical characters only, and the padding bits must be zero. Crockford allows
//! lower case and reads `I`, `L` and `O` as `1` and `0`, but ids are compared as text, so one value must have
//! exactly one spelling.

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const BITS_PER_CHARACTER: usize = 5;
const BITS_PER_BYTE: usize = 8;

/// The number of characters `encode_base32` writes for this many bytes.
pub fn base32_length(byte_count: usize) -> usize {
    (byte_count * BITS_PER_BYTE).div_ceil(BITS_PER_CHARACTER)
}

/// The bytes as Crockford base32, zero-padded at the front.
pub fn encode_base32(bytes: &[u8]) -> String {
    let character_count = base32_length(bytes.len());
    let padding_bits = character_count * BITS_PER_CHARACTER - bytes.len() * BITS_PER_BYTE;
    let mut text = String::with_capacity(character_count);
    for character_index in 0..character_count {
        let mut value = 0usize;
        for bit_index in 0..BITS_PER_CHARACTER {
            let padded_position = character_index * BITS_PER_CHARACTER + bit_index;
            value = (value << 1) | padded_bit(bytes, padded_position, padding_bits);
        }
        text.push(char::from(ALPHABET[value]));
    }
    text
}

/// The `byte_count` bytes that `text` encodes, or None when it isn't their exact canonical encoding (wrong length,
/// a character outside the alphabet, or a padding bit set).
pub fn decode_base32(text: &str, byte_count: usize) -> Option<Vec<u8>> {
    if text.len() != base32_length(byte_count) {
        return None;
    }
    let padding_bits = text.len() * BITS_PER_CHARACTER - byte_count * BITS_PER_BYTE;
    let mut bytes = vec![0u8; byte_count];
    for (character_index, character) in text.bytes().enumerate() {
        let value = character_value(character)?;
        for bit_index in 0..BITS_PER_CHARACTER {
            let bit = (value >> (BITS_PER_CHARACTER - 1 - bit_index)) & 1;
            let padded_position = character_index * BITS_PER_CHARACTER + bit_index;
            if padded_position < padding_bits {
                if bit != 0 {
                    return None;
                }
                continue;
            }
            let position = padded_position - padding_bits;
            bytes[position / BITS_PER_BYTE] |= (bit as u8) << (BITS_PER_BYTE - 1 - position % BITS_PER_BYTE);
        }
    }
    Some(bytes)
}

/// Bit `padded_position` of the zero-padded number, counted from the most significant end.
fn padded_bit(bytes: &[u8], padded_position: usize, padding_bits: usize) -> usize {
    if padded_position < padding_bits {
        return 0;
    }
    let position = padded_position - padding_bits;
    usize::from((bytes[position / BITS_PER_BYTE] >> (BITS_PER_BYTE - 1 - position % BITS_PER_BYTE)) & 1)
}

fn character_value(character: u8) -> Option<usize> {
    ALPHABET.iter().position(|candidate| *candidate == character)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // A fixed seed: the garbage below is the same on every run, so a failure reproduces.
    const GARBAGE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
    const GARBAGE_CASES: usize = 2000;

    /// xorshift64: enough to make varied test input without a dependency.
    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// `count` bytes of test input.
    fn random_test_bytes(state: &mut u64, count: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(count);
        for _ in 0..count {
            bytes.push(next_random(state) as u8);
        }
        bytes
    }

    #[test]
    fn known_values() {
        assert_eq!(encode_base32(&[]), "");
        assert_eq!(encode_base32(&[0]), "00");
        assert_eq!(encode_base32(&[0xFF]), "7Z");
        assert_eq!(encode_base32(&[0x01, 0x00]), "0080");
        assert_eq!(encode_base32(&[0xFF; 16]), "7ZZZZZZZZZZZZZZZZZZZZZZZZZ");
        assert_eq!(encode_base32(&[0; 16]), "00000000000000000000000000");
    }

    #[test]
    fn lengths_match_the_id_and_node_id_formats() {
        assert_eq!(base32_length(16), 26);
        assert_eq!(base32_length(32), 52);
        assert_eq!(encode_base32(&[7; 32]).len(), 52);
    }

    #[test]
    fn every_byte_count_round_trips() {
        let mut state = GARBAGE_SEED;
        for byte_count in 0..40 {
            let bytes = random_test_bytes(&mut state, byte_count);
            let text = encode_base32(&bytes);
            assert_eq!(text.len(), base32_length(byte_count));
            assert_eq!(decode_base32(&text, byte_count).unwrap(), bytes, "{text}");
        }
    }

    #[test]
    fn text_order_is_byte_order() {
        let mut state = GARBAGE_SEED;
        for _ in 0..GARBAGE_CASES {
            let first = random_test_bytes(&mut state, 16);
            let second = random_test_bytes(&mut state, 16);
            assert_eq!(encode_base32(&first).cmp(&encode_base32(&second)), first.cmp(&second));
        }
    }

    #[test]
    fn decoding_is_strict() {
        // Wrong length, lower case, letters Crockford leaves out, and a set padding bit (8 is 0b01000).
        assert_eq!(decode_base32("0", 1), None);
        assert_eq!(decode_base32("000", 1), None);
        assert_eq!(decode_base32("7z", 1), None);
        assert_eq!(decode_base32("0I", 1), None);
        assert_eq!(decode_base32("0U", 1), None);
        assert_eq!(decode_base32("80", 1), None);
        assert_eq!(decode_base32("8ZZZZZZZZZZZZZZZZZZZZZZZZZ", 16), None);
        assert_eq!(decode_base32("é0", 1), None);
    }

    // Ids arrive from files and requests: no input may panic the decoder.
    #[test]
    fn garbage_never_panics() {
        let mut state = GARBAGE_SEED;
        for _ in 0..GARBAGE_CASES {
            let length = (next_random(&mut state) % 60) as usize;
            let bytes = random_test_bytes(&mut state, length);
            let text = String::from_utf8_lossy(&bytes);
            for byte_count in [1, 16, 32] {
                if let Some(decoded) = decode_base32(&text, byte_count) {
                    assert_eq!(encode_base32(&decoded), text);
                }
            }
        }
    }
}
