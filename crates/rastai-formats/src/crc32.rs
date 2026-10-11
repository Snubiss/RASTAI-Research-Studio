//! The one CRC-32 table in the workspace (contracts 7.1): CRC-32/IEEE, the checksum of zip entries and of event-log
//! frames. It equals Python's `zlib.crc32`, so a log the Python worker writes and one this code writes carry the same
//! checksums.
//!
//! An in-house table version, not a crate: the spike showed that library choice swamps language choice (crc-fast),
//! and frames are small, so a byte-at-a-time table is fast enough (layout section 6.8). A benchmark in `tools/bench/`
//! comes before anything faster.

/// The reversed form of the IEEE 802.3 polynomial 0x04C11DB7. Reversed because CRC-32/IEEE feeds each byte in
/// least-significant bit first.
const IEEE_POLYNOMIAL_REVERSED: u32 = 0xEDB8_8320;
/// One entry per possible byte value.
const TABLE_SIZE: usize = 256;
/// The table, built once at compile time from the polynomial.
const CRC32_TABLE: [u32; TABLE_SIZE] = build_crc32_table();

/// The CRC-32/IEEE checksum of `bytes`. `crc32(b"123456789")` is `0xCBF43926`, the standard check value.
pub fn crc32(bytes: &[u8]) -> u32 {
    update_crc32(0, bytes)
}

/// Continues a checksum over more bytes, for data that arrives in pieces (a zip entry read in chunks).
/// `running` is the checksum of everything before `bytes` (0 to start), so
/// `update_crc32(crc32(first), second) == crc32(first + second)`, exactly like `zlib.crc32(second, running)`.
pub fn update_crc32(running: u32, bytes: &[u8]) -> u32 {
    // The algorithm works on the inverted value; inverting on the way in and out is what lets a finished checksum
    // be passed back in as `running`.
    let mut register = !running;
    for byte in bytes {
        let table_index = ((register ^ u32::from(*byte)) & 0xFF) as usize;
        register = CRC32_TABLE[table_index] ^ (register >> 8);
    }
    !register
}

/// Entry n is the remainder of dividing byte value n (as 8 bits, least significant first) by the polynomial.
/// A `const fn` with plain loops, so the table is part of the binary and nothing runs at startup.
const fn build_crc32_table() -> [u32; TABLE_SIZE] {
    let mut table = [0u32; TABLE_SIZE];
    let mut byte_value = 0;
    while byte_value < TABLE_SIZE {
        let mut remainder = byte_value as u32;
        let mut bit = 0;
        while bit < 8 {
            if remainder & 1 == 1 {
                remainder = (remainder >> 1) ^ IEEE_POLYNOMIAL_REVERSED;
            } else {
                remainder >>= 1;
            }
            bit += 1;
        }
        table[byte_value] = remainder;
        byte_value += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    // The standard check value of CRC-32/IEEE (the CRC catalogue's "check" for CRC-32/ISO-HDLC).
    const CHECK_INPUT: &[u8] = b"123456789";
    const CHECK_VALUE: u32 = 0xCBF4_3926;

    #[test]
    fn the_standard_check_value() {
        assert_eq!(crc32(CHECK_INPUT), CHECK_VALUE);
    }

    #[test]
    fn values_python_zlib_gives() {
        // zlib.crc32 of each input, from CPython.
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
        assert_eq!(crc32(b"RRSEVLG1"), 0xA562_830E);
    }

    #[test]
    fn streaming_equals_one_call() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for split in [0, 1, 7, 500, 999, 1000] {
            let (first, second) = bytes.split_at(split);
            assert_eq!(update_crc32(crc32(first), second), crc32(&bytes), "split at {split}");
        }
    }

    #[test]
    fn the_table_has_the_known_entries() {
        // Entries 0, 1 and 255 of the well-known IEEE table.
        assert_eq!(CRC32_TABLE[0], 0);
        assert_eq!(CRC32_TABLE[1], 0x7707_3096);
        assert_eq!(CRC32_TABLE[255], 0x2D02_EF8D);
    }
}
