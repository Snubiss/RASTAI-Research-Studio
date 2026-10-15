//! Acceptance tests for the zip reader (plan M2-22, ADR-0037): stored and deflate archives read; zip slip is refused
//! with RRS-E5001; a bomb ratio is refused with RRS-E5002 when the archive is listed, before any entry is inflated;
//! entries that share archive bytes are refused when listed, so many records can't point at one compressed stream; a
//! truncated central directory is refused; inflate stops at the declared size; and a bad checksum, an unsupported
//! feature and a duplicate name are each refused with their own code. The archives are built here, byte by byte, so
//! every case says exactly what it changes.

#![allow(clippy::unwrap_used)]

use rastai_formats::byte_source::{SliceSource, VecSink};
use rastai_formats::crc32::crc32;
use rastai_formats::error::FormatsError;
use rastai_formats::limits::ImportLimits;
use rastai_formats::zip::reader::{ZipEntry, ZipMethod, copy_zip_entry, list_zip_entries, read_zip_entry};

const HELLO: &[u8] = b"hello hello hello hello";
/// `HELLO` as raw DEFLATE, from Python: `zlib.compressobj(9, zlib.DEFLATED, -15)`.
const DEFLATED_HELLO: &[u8] = &[0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01];
/// `b"{\"ok\": true}\n" * 1000` (13,000 bytes) as raw DEFLATE, from Python: `zlib.compressobj(9, zlib.DEFLATED, -15)`.
/// It compresses 213:1, past the 200:1 cap, the way real repetitive JSON files do.
const REPETITIVE_JSON_DEFLATED: &[u8] = &[
    0xed, 0xc7, 0xb1, 0x0d, 0x00, 0x10, 0x10, 0x00, 0xc0, 0xde, 0x14, 0xf2, 0x63, 0xd8, 0x47, 0xa5, 0x90, 0x08, 0xd5,
    0xc7, 0xee, 0xa6, 0xd0, 0xdd, 0x75, 0x97, 0x31, 0x47, 0xb4, 0xba, 0xd7, 0xe9, 0xb7, 0xa4, 0x88, 0x88, 0x88, 0x88,
    0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88,
    0x88, 0xc8, 0x9f, 0x3c,
];

/// `vec![0u8; 1024 * 1024]` as raw DEFLATE, from Python: `zlib.compressobj(9, zlib.DEFLATED, -15)`. It is exactly the
/// ratio floor in `zip/reader.rs`, and it compresses 1015:1, so the test below pins that the floor itself is not checked.
const ZEROS_AT_FLOOR_DEFLATED: &[u8] = &[
    0xed, 0xc1, 0x31, 0x01, 0x00, 0x00, 0x00, 0xc2, 0xa0, 0xf5, 0x4f, 0x6d, 0x08, 0x5f, 0xa0, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x03,
];

const STORED: u16 = 0;
const DEFLATE: u16 = 8;
const FLAG_ENCRYPTED: u16 = 0x0001;

/// The end record's length, which is also where the end record starts counting back from.
const END_RECORD_LENGTH: usize = 22;
/// The ZIP64 locator's length, placed right before the end record.
const ZIP64_LOCATOR_LENGTH: usize = 20;
const LOCAL_HEADER_LENGTH: u64 = 30;

/// One entry to write: the bytes stored in the archive and the sizes and checksum the headers declare. A test
/// changes one of them to make a bad archive.
#[derive(Clone)]
struct Part {
    name: String,
    method: u16,
    flags: u16,
    data: Vec<u8>,
    declared_uncompressed: u64,
    declared_crc: u32,
}

fn stored(name: &str, bytes: &[u8]) -> Part {
    Part {
        name: name.to_string(),
        method: STORED,
        flags: 0,
        data: bytes.to_vec(),
        declared_uncompressed: bytes.len() as u64,
        declared_crc: crc32(bytes),
    }
}

fn deflated(name: &str) -> Part {
    Part {
        name: name.to_string(),
        method: DEFLATE,
        flags: 0,
        data: DEFLATED_HELLO.to_vec(),
        declared_uncompressed: HELLO.len() as u64,
        declared_crc: crc32(HELLO),
    }
}

/// A deflate entry whose stored bytes are `compressed` and whose declared content is `content`.
fn deflated_content(name: &str, compressed: &[u8], content: &[u8]) -> Part {
    Part {
        name: name.to_string(),
        method: DEFLATE,
        flags: 0,
        data: compressed.to_vec(),
        declared_uncompressed: content.len() as u64,
        declared_crc: crc32(content),
    }
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn local_header(part: &Part) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0403_4b50);
    put16(&mut out, 20);
    put16(&mut out, part.flags);
    put16(&mut out, part.method);
    put32(&mut out, 0);
    put32(&mut out, part.declared_crc);
    put32(&mut out, part.data.len() as u32);
    put32(&mut out, part.declared_uncompressed as u32);
    put16(&mut out, part.name.len() as u16);
    put16(&mut out, 0);
    out.extend_from_slice(part.name.as_bytes());
    out
}

/// The central record of one entry, which points at `offset` and declares `compressed` bytes. With `zip64`, the
/// three 32-bit fields hold the sentinel and their values follow in a ZIP64 extra field, the way a writer does for an
/// archive it can't describe in 32 bits.
fn central_header(part: &Part, offset: u64, compressed: u64, zip64: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut extra = Vec::new();
    if zip64 {
        put16(&mut extra, 0x0001);
        put16(&mut extra, 24);
        put64(&mut extra, part.declared_uncompressed);
        put64(&mut extra, compressed);
        put64(&mut extra, offset);
    }
    put32(&mut out, 0x0201_4b50);
    put16(&mut out, 20);
    put16(&mut out, 20);
    put16(&mut out, part.flags);
    put16(&mut out, part.method);
    put32(&mut out, 0);
    put32(&mut out, part.declared_crc);
    put32(&mut out, if zip64 { 0xFFFF_FFFF } else { compressed as u32 });
    put32(
        &mut out,
        if zip64 {
            0xFFFF_FFFF
        } else {
            part.declared_uncompressed as u32
        },
    );
    put16(&mut out, part.name.len() as u16);
    put16(&mut out, extra.len() as u16);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, if zip64 { 0xFFFF_FFFF } else { offset as u32 });
    out.extend_from_slice(part.name.as_bytes());
    out.extend_from_slice(&extra);
    out
}

/// The end record. `disk` is the number of this disk, which is 0 for every archive a single file holds.
fn end_record(disk: u16, entry_count: u16, directory_size: u32, directory_offset: u32) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, disk);
    put16(&mut out, 0);
    put16(&mut out, entry_count);
    put16(&mut out, entry_count);
    put32(&mut out, directory_size);
    put32(&mut out, directory_offset);
    put16(&mut out, 0);
    out
}

fn zip64_end_record(entry_count: u64, directory_size: u64, directory_offset: u64) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0606_4b50);
    put64(&mut out, 44);
    put16(&mut out, 45);
    put16(&mut out, 45);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put64(&mut out, entry_count);
    put64(&mut out, entry_count);
    put64(&mut out, directory_size);
    put64(&mut out, directory_offset);
    out
}

fn zip64_locator(record_offset: u64) -> Vec<u8> {
    let mut out = Vec::new();
    put32(&mut out, 0x0706_4b50);
    put32(&mut out, 0);
    put64(&mut out, record_offset);
    put32(&mut out, 1);
    out
}

/// The local headers and data of every part, in order, and the offset each one starts at.
fn write_locals(parts: &[Part]) -> (Vec<u8>, Vec<u64>) {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for part in parts {
        offsets.push(out.len() as u64);
        out.extend(local_header(part));
        out.extend_from_slice(&part.data);
    }
    (out, offsets)
}

/// Writes an archive with the given entries, each one's central record pointing where its bytes are, with the
/// compressed size its data really has. With `zip64`, the end records are the ZIP64 ones, with the sentinels in the
/// 16-bit and 32-bit end record fields.
fn build_zip(parts: &[Part], zip64: bool) -> Vec<u8> {
    let (locals, offsets) = write_locals(parts);
    let records: Vec<(u64, u64)> = parts
        .iter()
        .zip(offsets)
        .map(|(part, offset)| (offset, part.data.len() as u64))
        .collect();
    finish_zip(parts, locals, &records, zip64)
}

/// Writes an archive whose central records are the given `(offset, compressed size)` pairs, one per part, whatever
/// the local headers say. A test uses it to point records at the wrong bytes.
fn build_zip_with_records(parts: &[Part], records: &[(u64, u64)], zip64: bool) -> Vec<u8> {
    let (locals, _) = write_locals(parts);
    finish_zip(parts, locals, records, zip64)
}

fn finish_zip(parts: &[Part], mut out: Vec<u8>, records: &[(u64, u64)], zip64: bool) -> Vec<u8> {
    let directory_offset = out.len() as u64;
    for (part, (offset, compressed)) in parts.iter().zip(records) {
        out.extend(central_header(part, *offset, *compressed, zip64));
    }
    let directory_size = out.len() as u64 - directory_offset;
    if zip64 {
        let record_offset = out.len() as u64;
        out.extend(zip64_end_record(parts.len() as u64, directory_size, directory_offset));
        out.extend(zip64_locator(record_offset));
        out.extend(end_record(0, 0xFFFF, 0xFFFF_FFFF, 0xFFFF_FFFF));
    } else {
        out.extend(end_record(
            0,
            parts.len() as u16,
            directory_size as u32,
            directory_offset as u32,
        ));
    }
    out
}

/// The error the archive is refused with when it is listed.
fn list_error(bytes: &[u8], limits: &ImportLimits) -> FormatsError {
    match list_zip_entries(&SliceSource { bytes }, limits) {
        Ok(_) => panic!("the archive should have been refused"),
        Err(error) => error,
    }
}

/// The code the archive is refused with when it is listed.
fn list_code(bytes: &[u8], limits: &ImportLimits) -> &'static str {
    list_error(bytes, limits).code()
}

/// Lists the archive and copies the named entry, returning the bytes or the code it was refused with.
fn copy_named(bytes: &[u8], name: &str) -> Result<Vec<u8>, &'static str> {
    let source = SliceSource { bytes };
    let limits = ImportLimits::default();
    let entries = list_zip_entries(&source, &limits).map_err(|error| error.code())?;
    let entry = entries.iter().find(|entry| entry.path == name).unwrap().clone();
    let mut sink = VecSink { bytes: Vec::new() };
    copy_zip_entry(&source, &entry, &mut sink, &limits).map_err(|error| error.code())?;
    Ok(sink.bytes)
}

/// The offset of the end record, which is the last 22 bytes of an archive with no comment.
fn end_record_at(bytes: &[u8]) -> usize {
    bytes.len() - END_RECORD_LENGTH
}

/// The directory's offset, read from a non-ZIP64 archive's end record.
fn directory_offset_of(bytes: &[u8]) -> u64 {
    let at = end_record_at(bytes) + 16;
    u64::from(u32::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
    ]))
}

#[test]
fn stored_and_deflate_entries_read() {
    let bytes = build_zip(&[stored("note.txt", b"plain bytes"), deflated("words.txt")], false);
    let entries = list_zip_entries(&SliceSource { bytes: &bytes }, &ImportLimits::default()).unwrap();
    let names: Vec<(&str, ZipMethod)> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.method))
        .collect();
    assert_eq!(
        names,
        vec![("note.txt", ZipMethod::Stored), ("words.txt", ZipMethod::Deflate)]
    );
    assert_eq!(copy_named(&bytes, "note.txt").unwrap(), b"plain bytes");
    assert_eq!(copy_named(&bytes, "words.txt").unwrap(), HELLO);
}

#[test]
fn a_zip64_archive_reads_the_same_entries() {
    let bytes = build_zip(&[stored("a.bin", b"zip64 stored"), deflated("b.txt")], true);
    assert_eq!(copy_named(&bytes, "a.bin").unwrap(), b"zip64 stored");
    assert_eq!(copy_named(&bytes, "b.txt").unwrap(), HELLO);
}

#[test]
fn zip_slip_is_refused_when_the_archive_lists() {
    // The second entry is valid and the first escapes: the archive is refused before either entry is read.
    for escaping in ["../evil.txt", "/etc/evil", "a\\..\\evil", "dir/../../evil"] {
        let bytes = build_zip(&[stored(escaping, b"x"), deflated("ok.txt")], false);
        assert_eq!(list_code(&bytes, &ImportLimits::default()), "RRS-E5001", "{escaping}");
    }
}

#[test]
fn a_deflate_bomb_is_refused_when_listed_before_it_is_inflated() {
    // The declared size is 10 MB for 10 compressed bytes: a ratio of 10 MB / 10 is far past the cap. Inflating it would
    // fail as a size mismatch (RRS-E5007), so RRS-E5002 proves the ratio is checked first.
    let mut bomb = deflated("bomb.txt");
    bomb.declared_uncompressed = 10_000_000;
    let bytes = build_zip(&[bomb], false);
    assert_eq!(list_code(&bytes, &ImportLimits::default()), "RRS-E5002");
}

#[test]
fn a_copy_re_checks_the_ratio_of_an_entry_built_by_hand() {
    // A caller can change a listed entry's sizes. The copy refuses the ratio before the stream is inflated, so the sink
    // stays empty.
    let bytes = build_zip(&[deflated("words.txt")], false);
    let source = SliceSource { bytes: &bytes };
    let limits = ImportLimits::default();
    let mut entry = list_zip_entries(&source, &limits).unwrap()[0].clone();
    entry.uncompressed_bytes = 10_000_000;
    let mut sink = VecSink { bytes: Vec::new() };
    let error = copy_zip_entry(&source, &entry, &mut sink, &limits).unwrap_err();
    assert_eq!(error.code(), "RRS-E5002");
    assert!(sink.bytes.is_empty());
}

#[test]
fn a_small_repetitive_deflate_entry_over_the_ratio_cap_reads() {
    // 13,000 bytes in 61 compressed bytes is 213:1, past the 200:1 cap. Under the 1 MiB floor the cap doesn't apply, so
    // the archive lists and the entry copies out whole.
    let content = b"{\"ok\": true}\n".repeat(1000);
    assert!(
        content.len() as u64
            > ImportLimits::default().max_zip_compression_ratio * REPETITIVE_JSON_DEFLATED.len() as u64
    );
    let bytes = build_zip(
        &[deflated_content("rows.json", REPETITIVE_JSON_DEFLATED, &content)],
        false,
    );
    let entries = list_zip_entries(&SliceSource { bytes: &bytes }, &ImportLimits::default()).unwrap();
    assert_eq!(entries[0].uncompressed_bytes, content.len() as u64);
    assert_eq!(copy_named(&bytes, "rows.json").unwrap(), content);
}

#[test]
fn the_same_ratio_above_the_floor_is_refused_when_listed() {
    // Declared 1 MiB plus one byte at 213:1, so the compressed size is 4,924 bytes. The listing refuses on the declared
    // sizes before it reads the entry's data, so the data's bytes don't matter here.
    let declared: u64 = 1024 * 1024 + 1;
    let compressed_bytes = (declared / 213 + 1) as usize;
    let mut part = deflated_content("rows.json", &vec![0u8; compressed_bytes], HELLO);
    part.declared_uncompressed = declared;
    let bytes = build_zip(&[part], false);
    assert_eq!(list_code(&bytes, &ImportLimits::default()), "RRS-E5002");
}

#[test]
fn a_deflate_entry_of_exactly_the_floor_is_not_ratio_checked() {
    // The floor is inclusive: an entry declaring exactly 1 MiB at 1015:1 is past the cap but not checked, so it lists
    // and reads back whole. The test above refuses the same ratio one byte higher.
    let content = vec![0u8; 1024 * 1024];
    let bytes = build_zip(
        &[deflated_content("zeros.bin", ZEROS_AT_FLOOR_DEFLATED, &content)],
        false,
    );
    let entries = list_zip_entries(&SliceSource { bytes: &bytes }, &ImportLimits::default()).unwrap();
    assert_eq!(entries[0].uncompressed_bytes, content.len() as u64);
    assert_eq!(copy_named(&bytes, "zeros.bin").unwrap(), content);
}

#[test]
fn entries_that_share_archive_bytes_are_refused_when_listed() {
    // 300 central records, all pointing at the first entry's local header and its 11 bytes. Each name is distinct, so
    // the only thing wrong is the shared bytes. A per-entry check passes every one of them; the spans can't.
    let parts: Vec<Part> = (0..300)
        .map(|index| stored(&format!("f{index}.txt"), b"hello world"))
        .collect();
    let records = vec![(0u64, 11u64); parts.len()];
    let bytes = build_zip_with_records(&parts, &records, false);
    let error = list_error(&bytes, &ImportLimits::default());
    assert_eq!(error.code(), "RRS-E5007");
    assert!(
        matches!(&error, FormatsError::ZipMalformed { reason } if reason.contains("share bytes")),
        "{error:?}"
    );
}

#[test]
fn an_entry_whose_data_runs_into_the_central_directory_is_refused_when_listed() {
    // The deflate entry declares 4 bytes more than it has, so its data ends 4 bytes into the central directory.
    let parts = [deflated("words.txt")];
    let normal = build_zip(&parts, false);
    let directory_offset = directory_offset_of(&normal);
    let data_offset = LOCAL_HEADER_LENGTH + "words.txt".len() as u64;
    let compressed = directory_offset - data_offset + 4;
    let bytes = build_zip_with_records(&parts, &[(0, compressed)], false);
    assert_eq!(list_code(&bytes, &ImportLimits::default()), "RRS-E5007");
}

#[test]
fn too_many_entries_and_too_many_bytes_are_refused_when_listed() {
    let bytes = build_zip(&[stored("a", b"aa"), stored("b", b"bb")], false);
    let few_entries = ImportLimits {
        max_zip_entries: 1,
        ..ImportLimits::default()
    };
    assert_eq!(list_code(&bytes, &few_entries), "RRS-E5002");
    let few_bytes = ImportLimits {
        max_zip_total_bytes: 3,
        ..ImportLimits::default()
    };
    assert_eq!(list_code(&bytes, &few_bytes), "RRS-E5002");
}

#[test]
fn a_truncated_central_directory_is_refused() {
    let bytes = build_zip(&[stored("a.txt", b"hello"), stored("b.txt", b"world")], false);
    // Cutting the end record off leaves no end record at the end of the file.
    assert_eq!(
        list_code(&bytes[..bytes.len() - 10], &ImportLimits::default()),
        "RRS-E5007"
    );
    // A directory size that runs past the end record: the central directory is cut short.
    let mut overlong = bytes.clone();
    let end_at = overlong.len() - 22;
    overlong[end_at + 12..end_at + 16].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(list_code(&overlong, &ImportLimits::default()), "RRS-E5007");
    // A directory that starts past the end of the file.
    let mut moved = bytes;
    let end_at = moved.len() - 22;
    moved[end_at + 16..end_at + 20].copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
    assert_eq!(list_code(&moved, &ImportLimits::default()), "RRS-E5007");
}

#[test]
fn a_wrong_checksum_is_refused_when_the_entry_is_copied() {
    let mut part = stored("a.txt", b"hello");
    part.declared_crc ^= 1;
    let bytes = build_zip(&[part], false);
    assert_eq!(copy_named(&bytes, "a.txt").unwrap_err(), "RRS-E5007");
}

#[test]
fn a_deflate_entry_that_declares_too_little_is_refused_without_extra_output() {
    let mut part = deflated("words.txt");
    part.declared_uncompressed = 5;
    let bytes = build_zip(&[part], false);
    let source = SliceSource { bytes: &bytes };
    let limits = ImportLimits::default();
    let entries = list_zip_entries(&source, &limits).unwrap();
    let mut sink = VecSink { bytes: Vec::new() };
    let error = copy_zip_entry(&source, &entries[0], &mut sink, &limits).unwrap_err();
    assert_eq!(error.code(), "RRS-E5002");
    assert!(
        sink.bytes.len() <= 5,
        "{} bytes went past the declared 5",
        sink.bytes.len()
    );
}

#[test]
fn a_deflate_entry_that_declares_too_much_is_refused_when_copied() {
    // The stream ends at 23 bytes; the entry declares 30. The copy refuses the short output (RRS-E5007).
    let mut part = deflated("words.txt");
    part.declared_uncompressed = 30;
    let bytes = build_zip(&[part], false);
    assert_eq!(copy_named(&bytes, "words.txt").unwrap_err(), "RRS-E5007");
}

#[test]
fn bytes_after_the_deflate_stream_are_refused_when_copied() {
    let mut part = deflated("words.txt");
    part.data.push(0);
    let bytes = build_zip(&[part], false);
    assert_eq!(copy_named(&bytes, "words.txt").unwrap_err(), "RRS-E5007");
}

#[test]
fn encryption_and_other_methods_are_unsupported() {
    let mut encrypted = stored("secret.txt", b"x");
    encrypted.flags = FLAG_ENCRYPTED;
    assert_eq!(
        list_code(&build_zip(&[encrypted], false), &ImportLimits::default()),
        "RRS-E5008"
    );
    let mut bzip2 = stored("packed.txt", b"x");
    bzip2.method = 12;
    assert_eq!(
        list_code(&build_zip(&[bzip2], false), &ImportLimits::default()),
        "RRS-E5008"
    );
}

#[test]
fn an_archive_on_several_disks_is_unsupported() {
    let bytes = build_zip(&[stored("a.txt", b"hello")], false);
    let mut second_disk = bytes;
    let end_at = second_disk.len() - 22;
    second_disk[end_at + 4..end_at + 6].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(list_code(&second_disk, &ImportLimits::default()), "RRS-E5008");
}

#[test]
fn a_zip64_locator_that_counts_several_disks_is_unsupported() {
    let bytes = build_zip(&[stored("a.txt", b"hello")], true);
    let mut two_disks = bytes;
    // The locator sits right before the end record; its total disk count is the last four bytes of it.
    let locator_total_at = two_disks.len() - END_RECORD_LENGTH - ZIP64_LOCATOR_LENGTH + 16;
    two_disks[locator_total_at..locator_total_at + 4].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(list_code(&two_disks, &ImportLimits::default()), "RRS-E5008");
}

#[test]
fn two_entries_with_one_name_are_malformed() {
    let bytes = build_zip(&[stored("same.txt", b"one"), stored("same.txt", b"two")], false);
    assert_eq!(list_code(&bytes, &ImportLimits::default()), "RRS-E5007");
}

#[test]
fn a_local_header_with_the_wrong_signature_is_refused_when_listed() {
    let bytes = build_zip(&[stored("a.txt", b"hello")], false);
    let mut broken = bytes;
    broken[0] = 0;
    assert_eq!(list_code(&broken, &ImportLimits::default()), "RRS-E5007");
}

#[test]
fn a_local_header_that_disagrees_with_its_central_record_is_refused_when_listed() {
    let bytes = build_zip(&[stored("a.txt", b"hello")], false);
    // The local method field is at offset 8: deflate where the central record says stored.
    let mut other_method = bytes.clone();
    other_method[8] = 8;
    assert_eq!(list_code(&other_method, &ImportLimits::default()), "RRS-E5007");
    // The local flags are at offset 6: the encryption bit set where the central record has none.
    let mut encrypted = bytes;
    encrypted[6] |= 0x01;
    assert_eq!(list_code(&encrypted, &ImportLimits::default()), "RRS-E5007");
}

#[test]
fn read_zip_entry_returns_a_document_and_refuses_one_over_the_file_cap() {
    let bytes = build_zip(&[stored("doc.json", b"{}")], false);
    let source = SliceSource { bytes: &bytes };
    let entries: Vec<ZipEntry> = list_zip_entries(&source, &ImportLimits::default()).unwrap();
    assert_eq!(
        read_zip_entry(&source, &entries[0], &ImportLimits::default()).unwrap(),
        b"{}"
    );
    let small_cap = ImportLimits {
        max_file_bytes: 1,
        ..ImportLimits::default()
    };
    let error = read_zip_entry(&source, &entries[0], &small_cap).unwrap_err();
    assert_eq!(error.code(), "RRS-E5003");
}
