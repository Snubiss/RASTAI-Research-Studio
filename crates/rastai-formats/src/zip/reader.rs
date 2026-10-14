//! The zip container, read in-house (layout 4.5, ADR-0037 decision 1): the end record (ZIP64 too), the central
//! directory, every `ImportLimits` cap, the zip-slip check and the check that no two entries share bytes of the
//! archive. `list_zip_entries` runs all of it before any entry's bytes are read. Only DEFLATE is borrowed, in
//! `inflate.rs`. The stored-only writer is `writer.rs` (M2-20).
//!
//! The archive is read through a `ByteSource`, so the same code reads a zip inside a file or a buffer. Archives with
//! several disks, encryption or a method other than stored and deflate are refused with `RRS-E5008`: guessing at them
//! is not safe. A zip that is cut short or whose records disagree is `RRS-E5007`.

use std::collections::HashSet;

use crate::byte_source::{ByteSink, ByteSource, VecSink};
use crate::crc32::update_crc32;
use crate::error::FormatsError;
use crate::limits::ImportLimits;
use crate::zip::inflate::{inflate_chunk, new_inflate_stream};

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_RECORD_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_END_RECORD_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const ZIP64_EXTRA_ID: u16 = 0x0001;

const LOCAL_HEADER_LENGTH: u64 = 30;
const CENTRAL_HEADER_LENGTH: u64 = 46;
const END_RECORD_LENGTH: u64 = 22;
const ZIP64_LOCATOR_LENGTH: u64 = 20;
const ZIP64_END_RECORD_LENGTH: u64 = 56;
/// The end record's comment is the last thing before the end of the file, at most this long, so the search for the
/// record reads no more than this and the record's own length.
const MAX_COMMENT_LENGTH: u64 = 65_535;

/// A field whose real value is in the ZIP64 extra field when it holds these.
const SIXTEEN_BIT_SENTINEL: u16 = 0xFFFF;
const THIRTY_TWO_BIT_SENTINEL: u32 = 0xFFFF_FFFF;

const FLAG_ENCRYPTED: u16 = 0x0001;
const FLAG_STRONG_ENCRYPTION: u16 = 0x0040;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// Bytes copied at a time from a stored entry, so memory doesn't grow with the entry.
const COPY_CHUNK_BYTES: usize = 32 * 1024;
/// Compressed bytes read from the archive at a time for a deflate entry.
const INPUT_CHUNK_BYTES: usize = 64 * 1024;

/// The name of the cap in `RRS-E5002` for the ratio of one entry.
const RATIO_CAP: &str = "compression ratio";
/// The name of the cap in `RRS-E5002` for the total uncompressed size of the archive.
const TOTAL_CAP: &str = "total uncompressed size";
/// The name of the cap in `RRS-E5002` for the number of entries.
const ENTRY_COUNT_CAP: &str = "entry count";

const REASON_NO_END_RECORD: &str = "no zip end record reaches the end of the file";
const REASON_CUT_SHORT: &str = "a zip record runs past the end of the file or its directory";
const REASON_CENTRAL_SIGNATURE: &str = "a central directory record has the wrong signature";
const REASON_LOCAL_SIGNATURE: &str = "a local file header has the wrong signature";
const REASON_LOCAL_DISAGREES: &str =
    "a local file header disagrees with its central record on the method or encryption";
const REASON_LOCATOR: &str = "ZIP64 fields are used without their locator";
const REASON_ZIP64_MISSING: &str = "a ZIP64 value is needed but its extra field is missing";
const REASON_EXTRA_CUT_SHORT: &str = "an extra field runs past its record";
const REASON_DIRECTORY_SHORT: &str = "the central directory is too short for its entry count";
const REASON_DIRECTORY_PAST_END: &str = "the central directory runs past the end record";
const REASON_DUPLICATE_NAME: &str = "two entries share a name";
const REASON_NAME_NOT_UTF8: &str = "an entry name is not UTF-8";
const REASON_NAME_EMPTY: &str = "an entry name is empty or holds a NUL byte";
const REASON_STORED_SIZES: &str = "a stored entry's compressed and uncompressed sizes differ";
const REASON_DATA_PAST_END: &str = "an entry's data runs past the end of the file";
const REASON_SHARED_BYTES: &str =
    "two entries share bytes of the archive, or an entry's data runs into the central directory";
const REASON_CHECKSUM: &str = "an entry's bytes don't match its checksum or declared size";
const REASON_SHORT_OUTPUT: &str = "the deflate stream ends before the bytes its entry declares";
const REASON_TRAILING_BYTES: &str = "bytes follow the deflate stream inside its entry";
const REASON_TRAILING_RECORDS: &str = "bytes follow the last entry in the central directory";
const REASON_MULTI_DISK: &str = "the archive spans several disks";
const REASON_ENCRYPTED: &str = "an entry is encrypted";
const REASON_METHOD: &str = "an entry uses a compression method other than stored or deflate";
const REASON_OTHER_DISK: &str = "an entry is on another disk";

/// How an entry's bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipMethod {
    Stored,
    Deflate,
}

/// One entry as the central directory describes it. The sizes and the checksum are what the directory says;
/// `copy_zip_entry` checks the bytes it reads against them. `data_offset` is where the entry's compressed bytes start
/// in the archive, read from its local header when the directory is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub path: String,
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
    pub method: ZipMethod,
    pub data_offset: u64,
    pub crc32: u32,
}

/// Where the central directory is, how many entries it holds, and where it must end: the end record's position, or
/// the ZIP64 end record's when there is one.
struct Directory {
    entry_count: u64,
    size: u64,
    offset: u64,
    must_end_by: u64,
}

/// The sizes and position of one entry. Each one is the 32-bit field, or its ZIP64 value when the field holds the
/// sentinel.
struct Sizes {
    uncompressed: u64,
    compressed: u64,
    offset: u64,
    disk: u64,
}

/// Which fields of an entry hold their value in the ZIP64 extra field.
#[derive(Clone, Copy)]
struct Wanted {
    uncompressed: bool,
    compressed: bool,
    offset: bool,
    disk: bool,
}

impl Wanted {
    fn any(&self) -> bool {
        self.uncompressed || self.compressed || self.offset || self.disk
    }
}

/// The bytes one entry takes up in the archive, from its local header to the end of its compressed data.
struct ByteSpan {
    start: u64,
    end: u64,
}

/// One central directory record, as far as listing needs it. The entry's data offset is not in it: that comes from
/// the local header the record points at.
struct CentralRecord {
    path: String,
    method: ZipMethod,
    compressed_bytes: u64,
    uncompressed_bytes: u64,
    crc32: u32,
    local_header_offset: u64,
}

/// Passes bytes on to the caller's sink while it counts them and computes their checksum, so an entry is checked
/// against the central directory in the same pass that copies it.
struct CheckedSink<'a> {
    inner: &'a mut dyn ByteSink,
    crc: u32,
    written: u64,
}

impl ByteSink for CheckedSink<'_> {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), FormatsError> {
        self.crc = update_crc32(self.crc, bytes);
        self.written += bytes.len() as u64;
        self.inner.write_bytes(bytes)
    }
}

/// Lists an archive's entries: reads the end record and the central directory, and checks every cap of `limits`, the
/// zip-slip rule, the duplicate names and that no two entries share bytes of the archive. Each entry's data offset is
/// read from its local header here, so a refused archive never has an entry's bytes read.
pub fn list_zip_entries(source: &dyn ByteSource, limits: &ImportLimits) -> Result<Vec<ZipEntry>, FormatsError> {
    let directory = find_directory(source)?;
    check_entry_count(directory.entry_count, limits)?;
    check_directory_fits(&directory)?;
    let (entries, spans) = read_entries(source, &directory, limits)?;
    check_no_shared_bytes(spans, directory.offset)?;
    Ok(entries)
}

/// Copies one entry's bytes into `sink`, decompressing a deflate entry, and checks them against the entry's
/// checksum and declared size. The ratio cap is checked again here, so an entry a caller built by hand can't make
/// this inflate past it. A failed entry may already have written some bytes to the sink; callers discard that output.
pub fn copy_zip_entry(
    source: &dyn ByteSource,
    entry: &ZipEntry,
    sink: &mut dyn ByteSink,
    limits: &ImportLimits,
) -> Result<(), FormatsError> {
    data_end(source, entry.data_offset, entry.compressed_bytes)?;
    let mut checked = CheckedSink {
        inner: sink,
        crc: 0,
        written: 0,
    };
    match entry.method {
        ZipMethod::Stored => copy_stored(source, entry.data_offset, entry.compressed_bytes, &mut checked)?,
        ZipMethod::Deflate => {
            check_ratio(entry.compressed_bytes, entry.uncompressed_bytes, limits)?;
            copy_deflate(source, entry, &mut checked)?;
        }
    }
    if checked.crc != entry.crc32 || checked.written != entry.uncompressed_bytes {
        return Err(malformed(REASON_CHECKSUM));
    }
    Ok(())
}

/// Reads one entry's bytes whole, for a document. An entry over the file-size cap is refused with `RRS-E5003` before
/// any of its bytes are read. Larger entries are copied with `copy_zip_entry`.
pub fn read_zip_entry(
    source: &dyn ByteSource,
    entry: &ZipEntry,
    limits: &ImportLimits,
) -> Result<Vec<u8>, FormatsError> {
    if entry.uncompressed_bytes > limits.max_file_bytes {
        return Err(FormatsError::FileTooLarge {
            size_bytes: entry.uncompressed_bytes,
            limit_bytes: limits.max_file_bytes,
        });
    }
    let mut sink = VecSink { bytes: Vec::new() };
    copy_zip_entry(source, entry, &mut sink, limits)?;
    Ok(sink.bytes)
}

fn malformed(reason: &'static str) -> FormatsError {
    FormatsError::ZipMalformed { reason }
}

fn unsupported(reason: &'static str) -> FormatsError {
    FormatsError::ZipUnsupported { reason }
}

// Every caller passes a buffer of its record's fixed length, so the offsets below are inside it.
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(raw)
}

/// `length` bytes at `offset`, when they lie entirely before `limit`. Anything else is a cut-short record.
fn read_range(source: &dyn ByteSource, offset: u64, length: u64, limit: u64) -> Result<Vec<u8>, FormatsError> {
    let fits = offset.checked_add(length).is_some_and(|end| end <= limit);
    if !fits {
        return Err(malformed(REASON_CUT_SHORT));
    }
    let Ok(size) = usize::try_from(length) else {
        return Err(malformed(REASON_CUT_SHORT));
    };
    let mut bytes = vec![0u8; size];
    source.read_at(offset, &mut bytes)?;
    Ok(bytes)
}

fn find_directory(source: &dyn ByteSource) -> Result<Directory, FormatsError> {
    let end_record_at = find_end_record(source)?;
    let record = read_range(source, end_record_at, END_RECORD_LENGTH, source.length())?;
    if !needs_zip64(&record) {
        check_single_disk(u16_at(&record, 4), u16_at(&record, 6))?;
        return Ok(Directory {
            entry_count: u64::from(u16_at(&record, 10)),
            size: u64::from(u32_at(&record, 12)),
            offset: u64::from(u32_at(&record, 16)),
            must_end_by: end_record_at,
        });
    }
    read_zip64_directory(source, end_record_at)
}

/// The end record is the last 22 bytes plus a comment of up to 65535 bytes, and nothing follows it. The search takes
/// the last signature whose comment length reaches exactly to the end of the file.
fn find_end_record(source: &dyn ByteSource) -> Result<u64, FormatsError> {
    let length = source.length();
    if length < END_RECORD_LENGTH {
        return Err(malformed(REASON_NO_END_RECORD));
    }
    let window_length = length.min(END_RECORD_LENGTH + MAX_COMMENT_LENGTH);
    let window_start = length - window_length;
    let window = read_range(source, window_start, window_length, length)?;
    let record_size = END_RECORD_LENGTH as usize;
    for at in (0..=window.len() - record_size).rev() {
        let comment_length = usize::from(u16_at(&window, at + 20));
        if u32_at(&window, at) == END_RECORD_SIGNATURE && at + record_size + comment_length == window.len() {
            return Ok(window_start + at as u64);
        }
    }
    Err(malformed(REASON_NO_END_RECORD))
}

fn needs_zip64(record: &[u8]) -> bool {
    u16_at(record, 4) == SIXTEEN_BIT_SENTINEL
        || u16_at(record, 6) == SIXTEEN_BIT_SENTINEL
        || u16_at(record, 10) == SIXTEEN_BIT_SENTINEL
        || u32_at(record, 12) == THIRTY_TWO_BIT_SENTINEL
        || u32_at(record, 16) == THIRTY_TWO_BIT_SENTINEL
}

/// Several disks are refused: the entries of a split archive can't be read from one file.
fn check_single_disk(disk: u16, directory_disk: u16) -> Result<(), FormatsError> {
    if disk != 0 || directory_disk != 0 {
        return Err(unsupported(REASON_MULTI_DISK));
    }
    Ok(())
}

/// The ZIP64 locator sits right before the end record and points at the ZIP64 end record, which holds the real
/// entry count, directory size and directory offset.
fn read_zip64_directory(source: &dyn ByteSource, end_record_at: u64) -> Result<Directory, FormatsError> {
    if end_record_at < ZIP64_LOCATOR_LENGTH {
        return Err(malformed(REASON_LOCATOR));
    }
    let locator_at = end_record_at - ZIP64_LOCATOR_LENGTH;
    let locator = read_range(source, locator_at, ZIP64_LOCATOR_LENGTH, source.length())?;
    if u32_at(&locator, 0) != ZIP64_LOCATOR_SIGNATURE {
        return Err(malformed(REASON_LOCATOR));
    }
    // The locator's total disk count is 1 for one disk. Some writers leave it 0; only a count above 1 says the
    // archive spans disks.
    if u32_at(&locator, 16) > 1 {
        return Err(unsupported(REASON_MULTI_DISK));
    }
    let record_at = u64_at(&locator, 8);
    let record = read_range(source, record_at, ZIP64_END_RECORD_LENGTH, locator_at)?;
    if u32_at(&record, 0) != ZIP64_END_RECORD_SIGNATURE {
        return Err(malformed(REASON_LOCATOR));
    }
    if u32_at(&record, 16) != 0 || u32_at(&record, 20) != 0 {
        return Err(unsupported(REASON_MULTI_DISK));
    }
    Ok(Directory {
        entry_count: u64_at(&record, 32),
        size: u64_at(&record, 40),
        offset: u64_at(&record, 48),
        must_end_by: record_at,
    })
}

/// Refuses an archive that declares more entries than the cap, before any record is read.
fn check_entry_count(entry_count: u64, limits: &ImportLimits) -> Result<(), FormatsError> {
    if entry_count > limits.max_zip_entries {
        return Err(FormatsError::ZipLimit {
            what: ENTRY_COUNT_CAP,
            size: entry_count,
            limit: limits.max_zip_entries,
        });
    }
    Ok(())
}

/// The directory must lie before its end, and its entry count must fit in its size, so a short directory is refused
/// before any of its records is read.
fn check_directory_fits(directory: &Directory) -> Result<(), FormatsError> {
    let Some(end) = directory.offset.checked_add(directory.size) else {
        return Err(malformed(REASON_DIRECTORY_PAST_END));
    };
    if end > directory.must_end_by {
        return Err(malformed(REASON_DIRECTORY_PAST_END));
    }
    let Some(minimum_size) = directory.entry_count.checked_mul(CENTRAL_HEADER_LENGTH) else {
        return Err(malformed(REASON_DIRECTORY_SHORT));
    };
    if minimum_size > directory.size {
        return Err(malformed(REASON_DIRECTORY_SHORT));
    }
    Ok(())
}

/// Reads every central record, checks each one, and reads each entry's data offset from its local header. Returns the
/// entries in directory order, and the span of archive bytes each entry's header and data take up.
fn read_entries(
    source: &dyn ByteSource,
    directory: &Directory,
    limits: &ImportLimits,
) -> Result<(Vec<ZipEntry>, Vec<ByteSpan>), FormatsError> {
    let directory_end = directory.offset + directory.size;
    let mut entries = Vec::new();
    let mut spans = Vec::new();
    let mut names = HashSet::new();
    let mut total_bytes: u64 = 0;
    let mut position = directory.offset;
    for _ in 0..directory.entry_count {
        let (record, record_length) = read_central_record(source, position, directory_end)?;
        if !names.insert(record.path.clone()) {
            return Err(malformed(REASON_DUPLICATE_NAME));
        }
        check_total(record.uncompressed_bytes, limits, &mut total_bytes)?;
        check_method_sizes(&record, limits)?;
        let data_offset = local_data_offset(source, &record)?;
        let data_end_at = data_end(source, data_offset, record.compressed_bytes)?;
        spans.push(ByteSpan {
            start: record.local_header_offset,
            end: data_end_at,
        });
        entries.push(ZipEntry {
            path: record.path,
            compressed_bytes: record.compressed_bytes,
            uncompressed_bytes: record.uncompressed_bytes,
            method: record.method,
            data_offset,
            crc32: record.crc32,
        });
        position += record_length;
    }
    if position != directory_end {
        return Err(malformed(REASON_TRAILING_RECORDS));
    }
    Ok((entries, spans))
}

/// Entries may not share bytes of the archive, and no entry's data may run into the central directory. A bomb can
/// point many records at one compressed stream, and a per-entry ratio check can't see that; checking the spans
/// together can. Sorting by start and comparing neighbours finds every overlap.
fn check_no_shared_bytes(mut spans: Vec<ByteSpan>, directory_offset: u64) -> Result<(), FormatsError> {
    spans.sort_unstable_by_key(|span| span.start);
    let mut previous_end: u64 = 0;
    for span in spans {
        if span.start < previous_end {
            return Err(malformed(REASON_SHARED_BYTES));
        }
        previous_end = span.end;
    }
    if previous_end > directory_offset {
        return Err(malformed(REASON_SHARED_BYTES));
    }
    Ok(())
}

/// One central directory record, read and checked. Returns the record and the length of its bytes.
fn read_central_record(
    source: &dyn ByteSource,
    at: u64,
    must_end_by: u64,
) -> Result<(CentralRecord, u64), FormatsError> {
    let header = read_range(source, at, CENTRAL_HEADER_LENGTH, must_end_by)?;
    if u32_at(&header, 0) != CENTRAL_HEADER_SIGNATURE {
        return Err(malformed(REASON_CENTRAL_SIGNATURE));
    }
    let name_length = usize::from(u16_at(&header, 28));
    let extra_length = usize::from(u16_at(&header, 30));
    let comment_length = usize::from(u16_at(&header, 32));
    let variable_length = (name_length + extra_length + comment_length) as u64;
    let variable = read_range(source, at + CENTRAL_HEADER_LENGTH, variable_length, must_end_by)?;
    let flags = u16_at(&header, 8);
    if flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION) != 0 {
        return Err(unsupported(REASON_ENCRYPTED));
    }
    let method = method_of(u16_at(&header, 10))?;
    let extra = &variable[name_length..name_length + extra_length];
    let sizes = central_sizes(&header, extra)?;
    if sizes.disk != 0 {
        return Err(unsupported(REASON_OTHER_DISK));
    }
    let record = CentralRecord {
        path: entry_name(&variable[..name_length])?,
        method,
        compressed_bytes: sizes.compressed,
        uncompressed_bytes: sizes.uncompressed,
        crc32: u32_at(&header, 16),
        local_header_offset: sizes.offset,
    };
    Ok((record, CENTRAL_HEADER_LENGTH + variable_length))
}

fn method_of(code: u16) -> Result<ZipMethod, FormatsError> {
    match code {
        METHOD_STORED => Ok(ZipMethod::Stored),
        METHOD_DEFLATE => Ok(ZipMethod::Deflate),
        _ => Err(unsupported(REASON_METHOD)),
    }
}

/// The entry's sizes and offset from its central record: the 32-bit fields, unless one holds its sentinel, in which
/// case the value comes from the ZIP64 extra field.
fn central_sizes(header: &[u8], extra: &[u8]) -> Result<Sizes, FormatsError> {
    let plain = Sizes {
        uncompressed: u64::from(u32_at(header, 24)),
        compressed: u64::from(u32_at(header, 20)),
        offset: u64::from(u32_at(header, 42)),
        disk: u64::from(u16_at(header, 34)),
    };
    let wanted = Wanted {
        uncompressed: u32_at(header, 24) == THIRTY_TWO_BIT_SENTINEL,
        compressed: u32_at(header, 20) == THIRTY_TWO_BIT_SENTINEL,
        offset: u32_at(header, 42) == THIRTY_TWO_BIT_SENTINEL,
        disk: u16_at(header, 34) == SIXTEEN_BIT_SENTINEL,
    };
    if !wanted.any() {
        return Ok(plain);
    }
    read_zip64_extra(extra, plain, wanted)
}

/// Reads the fields the entry marks as ZIP64, in the order the specification gives them: uncompressed size,
/// compressed size, offset, disk.
fn read_zip64_extra(extra: &[u8], plain: Sizes, wanted: Wanted) -> Result<Sizes, FormatsError> {
    let Some(data) = find_zip64_field(extra)? else {
        return Err(malformed(REASON_ZIP64_MISSING));
    };
    let mut sizes = plain;
    let mut cursor = 0;
    if wanted.uncompressed {
        sizes.uncompressed = take_u64(data, &mut cursor)?;
    }
    if wanted.compressed {
        sizes.compressed = take_u64(data, &mut cursor)?;
    }
    if wanted.offset {
        sizes.offset = take_u64(data, &mut cursor)?;
    }
    if wanted.disk {
        sizes.disk = u64::from(take_u32(data, &mut cursor)?);
    }
    Ok(sizes)
}

/// The data of the ZIP64 extra field, if the extra field list has one.
fn find_zip64_field(extra: &[u8]) -> Result<Option<&[u8]>, FormatsError> {
    let mut cursor = 0;
    while cursor < extra.len() {
        if cursor + 4 > extra.len() {
            return Err(malformed(REASON_EXTRA_CUT_SHORT));
        }
        let id = u16_at(extra, cursor);
        let start = cursor + 4;
        let end = start + usize::from(u16_at(extra, cursor + 2));
        if end > extra.len() {
            return Err(malformed(REASON_EXTRA_CUT_SHORT));
        }
        if id == ZIP64_EXTRA_ID {
            return Ok(Some(&extra[start..end]));
        }
        cursor = end;
    }
    Ok(None)
}

fn take_u64(data: &[u8], cursor: &mut usize) -> Result<u64, FormatsError> {
    let end = *cursor + 8;
    if end > data.len() {
        return Err(malformed(REASON_ZIP64_MISSING));
    }
    let value = u64_at(data, *cursor);
    *cursor = end;
    Ok(value)
}

fn take_u32(data: &[u8], cursor: &mut usize) -> Result<u32, FormatsError> {
    let end = *cursor + 4;
    if end > data.len() {
        return Err(malformed(REASON_ZIP64_MISSING));
    }
    let value = u32_at(data, *cursor);
    *cursor = end;
    Ok(value)
}

/// The name, checked for zip slip before the archive opens. A name must be UTF-8 and nonempty, and it may not reach
/// outside the archive's folder.
fn entry_name(bytes: &[u8]) -> Result<String, FormatsError> {
    let name = String::from_utf8(bytes.to_vec()).map_err(|_| malformed(REASON_NAME_NOT_UTF8))?;
    check_entry_name(&name)?;
    Ok(name)
}

/// Zip slip: a name is refused when it starts with `/`, contains a backslash (a separator on Windows), has a drive
/// letter's colon in its first part, or has a `..` part. A name that stays inside the folder is still refused when
/// it uses `..`, because readers disagree about what such a name lands on.
fn check_entry_name(name: &str) -> Result<(), FormatsError> {
    if name.is_empty() || name.contains('\0') {
        return Err(malformed(REASON_NAME_EMPTY));
    }
    let escapes = name.starts_with('/')
        || name.contains('\\')
        || name.split('/').next().is_some_and(|first| first.contains(':'))
        || name.split('/').any(|part| part == "..");
    if escapes {
        return Err(FormatsError::ZipEntryEscapes { name: name.to_string() });
    }
    Ok(())
}

/// The running total of uncompressed bytes, checked against the cap after each entry.
fn check_total(uncompressed_bytes: u64, limits: &ImportLimits, total_bytes: &mut u64) -> Result<(), FormatsError> {
    *total_bytes = total_bytes.saturating_add(uncompressed_bytes);
    if *total_bytes > limits.max_zip_total_bytes {
        return Err(FormatsError::ZipLimit {
            what: TOTAL_CAP,
            size: *total_bytes,
            limit: limits.max_zip_total_bytes,
        });
    }
    Ok(())
}

/// A stored entry has the same size both ways. A deflate entry's ratio is checked against the cap.
fn check_method_sizes(record: &CentralRecord, limits: &ImportLimits) -> Result<(), FormatsError> {
    match record.method {
        ZipMethod::Stored => {
            if record.compressed_bytes != record.uncompressed_bytes {
                return Err(malformed(REASON_STORED_SIZES));
            }
            Ok(())
        }
        ZipMethod::Deflate => check_ratio(record.compressed_bytes, record.uncompressed_bytes, limits),
    }
}

/// A deflate entry may not expand past the ratio cap (zip bomb). The cap is checked on the declared sizes, before any
/// of the entry's bytes are read.
fn check_ratio(compressed_bytes: u64, uncompressed_bytes: u64, limits: &ImportLimits) -> Result<(), FormatsError> {
    let allowed = compressed_bytes.saturating_mul(limits.max_zip_compression_ratio);
    if uncompressed_bytes > allowed {
        return Err(FormatsError::ZipLimit {
            what: RATIO_CAP,
            size: uncompressed_bytes,
            limit: allowed,
        });
    }
    Ok(())
}

/// Where an entry's compressed bytes start: after its local header, whose name and extra field lengths may differ
/// from the central record's. The local header must also agree on the method and the encryption bit. The central
/// record is what gets read, so a mismatch would be invisible otherwise, and stricter readers refuse it.
fn local_data_offset(source: &dyn ByteSource, record: &CentralRecord) -> Result<u64, FormatsError> {
    let local_header_offset = record.local_header_offset;
    let header = read_range(source, local_header_offset, LOCAL_HEADER_LENGTH, source.length())?;
    if u32_at(&header, 0) != LOCAL_HEADER_SIGNATURE {
        return Err(malformed(REASON_LOCAL_SIGNATURE));
    }
    let local_method = method_of(u16_at(&header, 8)).ok();
    let local_encrypted = u16_at(&header, 6) & FLAG_ENCRYPTED != 0;
    if local_method != Some(record.method) || local_encrypted {
        return Err(malformed(REASON_LOCAL_DISAGREES));
    }
    let variable_length = u64::from(u16_at(&header, 26)) + u64::from(u16_at(&header, 28));
    Ok(local_header_offset + LOCAL_HEADER_LENGTH + variable_length)
}

/// The end of an entry's compressed bytes, refused when they run past the end of the archive.
fn data_end(source: &dyn ByteSource, data_offset: u64, compressed_bytes: u64) -> Result<u64, FormatsError> {
    match data_offset.checked_add(compressed_bytes) {
        Some(end) if end <= source.length() => Ok(end),
        _ => Err(malformed(REASON_DATA_PAST_END)),
    }
}

/// Copies `size` stored bytes starting at `data_offset`, in chunks.
fn copy_stored(
    source: &dyn ByteSource,
    data_offset: u64,
    size: u64,
    sink: &mut dyn ByteSink,
) -> Result<(), FormatsError> {
    let mut buffer = vec![0u8; COPY_CHUNK_BYTES];
    let mut copied: u64 = 0;
    while copied < size {
        let chunk = (size - copied).min(COPY_CHUNK_BYTES as u64) as usize;
        source.read_at(data_offset + copied, &mut buffer[..chunk])?;
        sink.write_bytes(&buffer[..chunk])?;
        copied += chunk as u64;
    }
    Ok(())
}

/// Inflates a deflate entry's compressed bytes into `sink`, reading them from the archive a chunk at a time. The
/// stream must produce exactly the declared size and end where the entry's compressed bytes end.
fn copy_deflate(source: &dyn ByteSource, entry: &ZipEntry, sink: &mut dyn ByteSink) -> Result<(), FormatsError> {
    let mut stream = new_inflate_stream(entry.uncompressed_bytes);
    let mut input = EntryInput::new(source, entry);
    let mut written: u64 = 0;
    loop {
        input.refill_if_used_up()?;
        let progress = inflate_chunk(&mut stream, input.unused(), sink)?;
        input.consume(progress.consumed);
        written += progress.written as u64;
        if progress.finished {
            break;
        }
    }
    if written != entry.uncompressed_bytes {
        return Err(malformed(REASON_SHORT_OUTPUT));
    }
    if !input.is_all_used() {
        return Err(malformed(REASON_TRAILING_BYTES));
    }
    Ok(())
}

/// The compressed bytes of one deflate entry, read from the archive a chunk at a time. `unused` is what the stream
/// has not taken yet.
struct EntryInput<'a> {
    source: &'a dyn ByteSource,
    data_offset: u64,
    compressed_bytes: u64,
    /// Compressed bytes read from the archive so far.
    read_so_far: u64,
    chunk: Vec<u8>,
    /// The next unused byte of `chunk`.
    chunk_at: usize,
    /// One past the last byte of `chunk` that holds data.
    chunk_end: usize,
}

impl<'a> EntryInput<'a> {
    fn new(source: &'a dyn ByteSource, entry: &ZipEntry) -> Self {
        let chunk_length = entry.compressed_bytes.min(INPUT_CHUNK_BYTES as u64) as usize;
        EntryInput {
            source,
            data_offset: entry.data_offset,
            compressed_bytes: entry.compressed_bytes,
            read_so_far: 0,
            chunk: vec![0u8; chunk_length],
            chunk_at: 0,
            chunk_end: 0,
        }
    }

    /// Reads the next chunk from the archive once the last one is used up.
    fn refill_if_used_up(&mut self) -> Result<(), FormatsError> {
        if self.chunk_at < self.chunk_end || self.read_so_far == self.compressed_bytes {
            return Ok(());
        }
        let length = (self.compressed_bytes - self.read_so_far).min(self.chunk.len() as u64) as usize;
        self.source
            .read_at(self.data_offset + self.read_so_far, &mut self.chunk[..length])?;
        self.chunk_at = 0;
        self.chunk_end = length;
        self.read_so_far += length as u64;
        Ok(())
    }

    fn unused(&self) -> &[u8] {
        &self.chunk[self.chunk_at..self.chunk_end]
    }

    fn consume(&mut self, count: usize) {
        self.chunk_at += count;
    }

    /// Every compressed byte of the entry has been read from the archive and taken by the stream.
    fn is_all_used(&self) -> bool {
        self.chunk_at == self.chunk_end && self.read_so_far == self.compressed_bytes
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_allowed() {
        for name in [
            "model.json",
            "weights/layer.safetensors",
            "dir/",
            "./a.txt",
            "a..b/c",
            "..hidden",
        ] {
            check_entry_name(name).unwrap();
        }
    }

    #[test]
    fn names_that_can_escape_are_refused_as_zip_slip() {
        for name in [
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "a\\..\\b",
            "C:evil",
            "dir/..",
            "..\\x",
        ] {
            let error = check_entry_name(name).unwrap_err();
            assert_eq!(error.code(), "RRS-E5001", "{name}");
        }
    }

    #[test]
    fn empty_names_and_nul_bytes_are_malformed() {
        assert_eq!(check_entry_name("").unwrap_err().code(), "RRS-E5007");
        assert_eq!(check_entry_name("a\0b").unwrap_err().code(), "RRS-E5007");
    }

    fn spans(pairs: &[(u64, u64)]) -> Vec<ByteSpan> {
        pairs.iter().map(|&(start, end)| ByteSpan { start, end }).collect()
    }

    #[test]
    fn spans_that_touch_are_fine_and_spans_that_share_bytes_are_not() {
        check_no_shared_bytes(spans(&[(0, 10), (10, 20), (20, 30)]), 30).unwrap();
        assert_eq!(
            check_no_shared_bytes(spans(&[(0, 10), (5, 20)]), 30)
                .unwrap_err()
                .code(),
            "RRS-E5007"
        );
        assert_eq!(
            check_no_shared_bytes(spans(&[(0, 10), (0, 10)]), 30)
                .unwrap_err()
                .code(),
            "RRS-E5007"
        );
    }

    #[test]
    fn a_span_that_runs_into_the_central_directory_is_refused() {
        assert_eq!(
            check_no_shared_bytes(spans(&[(0, 31)]), 30).unwrap_err().code(),
            "RRS-E5007"
        );
    }
}
