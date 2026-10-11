//! The bytes of an event log (contracts 7.1). A file is the 8-byte header `RRSEVLG1` followed by frames, and a frame
//! is:
//!
//! ```text
//! body_length: u32 little-endian | checksum: u32 little-endian | body: body_length bytes
//! ```
//!
//! The body is one protobuf `rastai.events.v1.EventRecord`, and the checksum is CRC-32/IEEE of the body through
//! `rastai_formats::crc32`, the same as Python's `zlib.crc32`, so the Python worker's frames and these are the
//! same bytes. This is the one file that encodes or decodes a frame body (prost's wrapper here, layout 6.5).
//!
//! It also walks the top-level fields of a body that may be cut off (`body_starts_as_record`, `field_end_at`), which
//! prost can't: the reader uses the walk to tell a frame a record merely quotes from a real later frame.

use prost::Message;
use rastai_formats::crc32::crc32;
use rastai_proto::events::v1::EventRecord;

/// The first 8 bytes of every event log.
pub const EVENT_LOG_HEADER: &[u8; 8] = b"RRSEVLG1";
/// `body_length` and `checksum`, the two little-endian u32s in front of every body.
pub const FRAME_HEADER_BYTES: usize = 8;
/// The record layout this code reads and writes (`EventRecord.format_version`). A reader refuses any other version
/// instead of guessing at it.
pub const EVENT_RECORD_FORMAT_VERSION: u32 = 1;
/// Every log's first record has sequence 1, and each next one adds 1 (contracts 7.1). Used by the reader and the
/// writer only, so it isn't part of the crate's public names.
pub(crate) const FIRST_SEQUENCE: u64 = 1;

/// The protobuf wire types (the low 3 bits of a field key) that `EventRecord` and its messages use. The group types,
/// 3 and 4, are never written by our encoders, so the walk stops at them.
const WIRE_TYPE_VARINT: u64 = 0;
const WIRE_TYPE_FIXED64: u64 = 1;
const WIRE_TYPE_LENGTH_DELIMITED: u64 = 2;
const WIRE_TYPE_FIXED32: u64 = 5;
const WIRE_TYPE_BITS: u32 = 3;
const FIXED64_BYTES: usize = 8;
const FIXED32_BYTES: usize = 4;
/// A u64 varint takes at most 10 bytes, 7 bits each.
const MAX_VARINT_BYTES: usize = 10;
/// `EventRecord`'s first two field numbers. Every encoder writes fields in number order (contracts 4.3), and both
/// are always written, because a record's version (1) and sequence (1 or more) are never 0.
const FORMAT_VERSION_FIELD: u64 = 1;
const SEQUENCE_FIELD: u64 = 2;

/// The two numbers in front of a frame's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub body_length: u32,
    pub checksum: u32,
}

/// One whole frame for `record`: header, then the encoded body. `None` when the body doesn't fit the u32 length
/// field. That never happens in practice (large data lives in the blob store and a record carries only its hash),
/// but a wrong length would make every later frame unreadable, so it is refused rather than written.
pub fn encode_frame(record: &EventRecord) -> Option<Vec<u8>> {
    let body = record.encode_to_vec();
    let body_length = u32::try_from(body.len()).ok()?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + body.len());
    frame.extend_from_slice(&body_length.to_le_bytes());
    frame.extend_from_slice(&crc32(&body).to_le_bytes());
    frame.extend_from_slice(&body);
    Some(frame)
}

/// Reads the 8 bytes in front of a body. It can't fail: any 8 bytes are some length and some checksum, and the
/// reader checks both against the file.
pub fn decode_frame_header(bytes: &[u8; FRAME_HEADER_BYTES]) -> FrameHeader {
    FrameHeader {
        body_length: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        checksum: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    }
}

/// True when `body` is what `header` says it is: the declared length and a matching CRC-32.
pub fn body_matches_header(header: &FrameHeader, body: &[u8]) -> bool {
    body.len() as u64 == u64::from(header.body_length) && crc32(body) == header.checksum
}

/// The record in a body whose checksum already matched, or `None` when it isn't a record this code can read: bytes
/// that aren't an `EventRecord`, or a `format_version` other than 1.
pub fn decode_frame_body(body: &[u8]) -> Option<EventRecord> {
    let record = EventRecord::decode(body).ok()?;
    if record.format_version != EVENT_RECORD_FORMAT_VERSION {
        return None;
    }
    Some(record)
}

/// True when `bytes` begin the way the body of record `sequence` does: its `format_version` field holding 1, then its
/// `sequence` field holding `sequence`. The rest of the body may be missing (a torn frame).
pub(crate) fn body_starts_as_record(bytes: &[u8], sequence: u64) -> bool {
    let Some(version) = read_wire_field(bytes, 0) else {
        return false;
    };
    let Some(sequence_field) = read_wire_field(bytes, version.end) else {
        return false;
    };
    let version_matches = version.number == FORMAT_VERSION_FIELD
        && version.wire_type == WIRE_TYPE_VARINT
        && version.value == u64::from(EVENT_RECORD_FORMAT_VERSION);
    let sequence_matches = sequence_field.number == SEQUENCE_FIELD
        && sequence_field.wire_type == WIRE_TYPE_VARINT
        && sequence_field.value == sequence;
    version_matches && sequence_matches
}

/// Where the whole top-level protobuf field that starts at `start` ends, or `None` when the bytes there aren't a
/// field or the field runs past the end of `bytes`. A record's text sits inside its fields, never between them.
pub(crate) fn field_end_at(bytes: &[u8], start: usize) -> Option<usize> {
    Some(read_wire_field(bytes, start)?.end)
}

/// One protobuf field as the walk sees it: `value` is the number of a varint field, or the length of a
/// length-delimited one.
struct WireField {
    number: u64,
    wire_type: u64,
    value: u64,
    end: usize,
}

fn read_wire_field(bytes: &[u8], start: usize) -> Option<WireField> {
    let (key, after_key) = read_varint(bytes, start)?;
    let number = key >> WIRE_TYPE_BITS;
    let wire_type = key & ((1 << WIRE_TYPE_BITS) - 1);
    if number == 0 {
        return None;
    }
    let (value, end) = match wire_type {
        WIRE_TYPE_VARINT => read_varint(bytes, after_key)?,
        WIRE_TYPE_FIXED64 => (0, after_key.checked_add(FIXED64_BYTES)?),
        WIRE_TYPE_FIXED32 => (0, after_key.checked_add(FIXED32_BYTES)?),
        WIRE_TYPE_LENGTH_DELIMITED => {
            let (length, after_length) = read_varint(bytes, after_key)?;
            (length, after_length.checked_add(usize::try_from(length).ok()?)?)
        }
        _ => return None,
    };
    if end > bytes.len() {
        return None;
    }
    Some(WireField {
        number,
        wire_type,
        value,
        end,
    })
}

/// The varint at `start` and where it ends; `None` when the bytes end first or it is longer than a u64's 10 bytes.
fn read_varint(bytes: &[u8], start: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for index in 0..MAX_VARINT_BYTES {
        let byte = *bytes.get(start.checked_add(index)?)?;
        value |= u64::from(byte & 0x7F) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, start + index + 1));
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sample_record() -> EventRecord {
        EventRecord {
            format_version: EVENT_RECORD_FORMAT_VERSION,
            sequence: 7,
            run_id: "run_01JA2B3C4D5E6F7G8H9J0K1M2S".to_string(),
            job_id: "job_01JA2B3C4D5E6F7G8H9J0K1M2Q".to_string(),
            emitted_unix_ms: 1_791_700_000_123,
            event_type: "run.state_changed".to_string(),
            ..EventRecord::default()
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let record = sample_record();
        let frame = encode_frame(&record).unwrap();
        let header = decode_frame_header(frame[..FRAME_HEADER_BYTES].try_into().unwrap());
        let body = &frame[FRAME_HEADER_BYTES..];
        assert_eq!(header.body_length as usize, body.len());
        assert!(body_matches_header(&header, body));
        assert_eq!(decode_frame_body(body), Some(record));
    }

    #[test]
    fn the_header_is_little_endian() {
        let header = decode_frame_header(&[0x10, 0, 0, 0, 0x26, 0x39, 0xF4, 0xCB]);
        assert_eq!(header.body_length, 16);
        assert_eq!(header.checksum, 0xCBF4_3926);
    }

    #[test]
    fn a_flipped_body_byte_fails_the_checksum() {
        let frame = encode_frame(&sample_record()).unwrap();
        let header = decode_frame_header(frame[..FRAME_HEADER_BYTES].try_into().unwrap());
        let mut body = frame[FRAME_HEADER_BYTES..].to_vec();
        body[3] ^= 0x01;
        assert!(!body_matches_header(&header, &body));
    }

    #[test]
    fn a_body_and_its_torn_prefixes_start_as_their_record() {
        let body = sample_record().encode_to_vec();
        assert!(body_starts_as_record(&body, 7));
        assert!(body_starts_as_record(&body[..6], 7));
        assert!(!body_starts_as_record(&body, 8));
        assert!(!body_starts_as_record(&body[..1], 7));
        assert!(!body_starts_as_record(&[0xFF; 16], 7));
    }

    #[test]
    fn the_field_walk_steps_over_whole_fields_only() {
        let record = sample_record();
        let body = record.encode_to_vec();
        let mut position = 0;
        let mut field_count = 0;
        while let Some(end) = field_end_at(&body, position) {
            position = end;
            field_count += 1;
        }
        // format_version, sequence, run_id, job_id, emitted_unix_ms, event_type: the walk ends exactly at the end.
        assert_eq!(position, body.len());
        assert_eq!(field_count, 6);
        // A field cut short, and a key with field number 0, aren't fields.
        assert_eq!(field_end_at(&body[..body.len() - 1], 0), Some(2));
        let run_id_start = 4;
        assert_eq!(field_end_at(&body[..run_id_start + 3], run_id_start), None);
        assert_eq!(field_end_at(&[0x00, 0x01], 0), None);
        assert_eq!(field_end_at(&[0xFF; 11], 0), None);
    }

    #[test]
    fn another_format_version_is_refused() {
        let mut record = sample_record();
        record.format_version = 2;
        assert_eq!(decode_frame_body(&record.encode_to_vec()), None);
        // An empty body decodes as an all-default record, version 0: refused too.
        assert_eq!(decode_frame_body(&[]), None);
    }
}
