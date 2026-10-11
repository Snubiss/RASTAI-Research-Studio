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
    fn another_format_version_is_refused() {
        let mut record = sample_record();
        record.format_version = 2;
        assert_eq!(decode_frame_body(&record.encode_to_vec()), None);
        // An empty body decodes as an all-default record, version 0: refused too.
        assert_eq!(decode_frame_body(&[]), None);
    }
}
