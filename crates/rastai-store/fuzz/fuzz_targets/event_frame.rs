//! Fuzz target for `read_next_frame` (contracts 12.5): an event log's bytes come from a node, a crashed writer or a
//! damaged disk, so arbitrary bytes must never panic the reader, and it must never read past a frame:
//!
//! - every record moves the reader exactly one frame, the length the frame's own header declares, and that frame's
//!   body has the CRC-32 the header says, with sequences 1, 2, 3 ...;
//! - every other outcome leaves the reader where the whole frames end: `EndOfFile` at the end of the file,
//!   `Incomplete` and `Corrupt` at the offset they report.
//!
//! Then the writer gets the same bytes: whenever `open_event_log_for_append` accepts them, it has cut off any torn
//! tail, so the file now reads as whole frames to the end, the same records as before (a torn tail is truncated,
//! never trusted).

#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;
use rastai_formats::crc32::crc32;
use rastai_store::event_log::frame::{EVENT_LOG_HEADER, FRAME_HEADER_BYTES, decode_frame_header};
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame, reader_offset};
use rastai_store::event_log::writer::open_event_log_for_append;

/// What one read of the whole file saw.
struct ReadSummary {
    record_count: u64,
    outcome: &'static str,
}

fuzz_target!(|bytes: &[u8]| {
    // The reader reads files, so the input becomes one, a file per fuzzing process, removed after each input so a
    // run leaves nothing in the temp folder.
    let path = std::env::temp_dir().join(format!("rastai-store-fuzz-event-frame-{}.log", std::process::id()));
    if std::fs::write(&path, bytes).is_ok() {
        check_reader_then_writer(&path, bytes);
    }
    let _ = std::fs::remove_file(&path);
});

/// Reads `path` (whose bytes are `bytes`) checking every frame, then opens it for append and, if the writer accepts
/// it, checks the cut file reads as whole frames to the end with the same records.
fn check_reader_then_writer(path: &Path, bytes: &[u8]) {
    let Some(before) = read_and_check_every_frame(path, bytes) else {
        return;
    };
    if open_event_log_for_append(path).is_err() {
        return;
    }
    let Ok(cut_bytes) = std::fs::read(path) else {
        return;
    };
    let after = read_and_check_every_frame(path, &cut_bytes).expect("a log the writer accepted reads again");
    assert_eq!(
        after.outcome, "end of file",
        "the writer left a bad tail; before: {}",
        before.outcome
    );
    assert_eq!(after.record_count, before.record_count);
}

/// Reads `path` (whose bytes are `bytes`) to the first non-record, checking each step. `None` when the read stops
/// with an error (a sequence gap, or the file system).
fn read_and_check_every_frame(path: &Path, bytes: &[u8]) -> Option<ReadSummary> {
    let mut reader = open_event_log(path).ok()?;
    let mut record_count = 0;
    loop {
        let frame_start = reader_offset(&reader).max(EVENT_LOG_HEADER.len() as u64);
        let outcome = read_next_frame(&mut reader).ok()?;
        let offset = reader_offset(&reader);
        assert!(
            offset <= bytes.len() as u64,
            "the reader moved past the end of the file"
        );
        match outcome {
            FrameRead::Record(record) => {
                record_count += 1;
                assert_eq!(record.sequence, record_count);
                check_one_frame_was_read(bytes, frame_start, offset);
            }
            FrameRead::EndOfFile => {
                assert_eq!(offset, bytes.len() as u64);
                return Some(summary(record_count, "end of file"));
            }
            FrameRead::Incomplete { valid_bytes } => {
                assert_eq!(valid_bytes, offset);
                return Some(summary(record_count, "incomplete"));
            }
            FrameRead::Corrupt { offset: corrupt_offset } => {
                assert_eq!(corrupt_offset, offset);
                return Some(summary(record_count, "corrupt"));
            }
        }
    }
}

/// The reader moved from `frame_start` to `offset`: exactly the frame the header at `frame_start` declares, whose
/// body has the declared checksum.
fn check_one_frame_was_read(bytes: &[u8], frame_start: u64, offset: u64) {
    let start = frame_start as usize;
    let header_end = start + FRAME_HEADER_BYTES;
    let header_bytes: [u8; FRAME_HEADER_BYTES] = bytes[start..header_end].try_into().expect("8 bytes");
    let header = decode_frame_header(&header_bytes);
    let frame_end = header_end + header.body_length as usize;
    assert_eq!(
        offset, frame_end as u64,
        "the reader moved by something other than one frame"
    );
    assert_eq!(crc32(&bytes[header_end..frame_end]), header.checksum);
}

fn summary(record_count: u64, outcome: &'static str) -> ReadSummary {
    ReadSummary { record_count, outcome }
}
