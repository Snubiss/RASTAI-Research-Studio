//! Reading an event log frame by frame (contracts 7.1, flows/platform.md 6.1 and 6.2). The node's tailer, the
//! daemon's indexer, the rebuild and the writer's own open-for-append all read through `read_next_frame`, so there is
//! one parser of these bytes.
//!
//! Each call answers one of four things: a record, the end of the file, an incomplete tail (stop and, if the writer
//! is still alive, try again later) or a corrupt frame (stop for good). A reader never changes the file: only the
//! writer truncates a torn tail (writer.rs).
//!
//! How a bad frame is told apart from a torn tail (contracts 7.1): a crash can only damage the end of the file, so a
//! frame that fails its checks counts as the torn tail when nothing but zeros follows its start (a power cut can
//! leave a zero-filled block) or, for a checksum mismatch, when it ends exactly at the end of the file. A bad frame
//! with real bytes after it is corruption (`RRS-S2002`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rastai_proto::events::v1::EventRecord;

use crate::error::{StoreError, event_log_io_error};
use crate::event_log::frame::{
    EVENT_LOG_HEADER, FRAME_HEADER_BYTES, body_matches_header, decode_frame_body, decode_frame_header,
};

/// Every log's first record has sequence 1 (contracts 7.1).
const FIRST_SEQUENCE: u64 = 1;
/// How much of the file the zero-tail check reads at a time.
const ZERO_SCAN_CHUNK_BYTES: usize = 64 * 1024;

/// What one `read_next_frame` call found.
// The record is held by value, as the layout's signature has it (rust-control-plane.md 6.2): one FrameRead exists
// per call and the caller moves the record straight out, so a Box would only add an allocation per record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum FrameRead {
    /// The next record, in sequence.
    Record(EventRecord),
    /// Every byte of the file has been read as whole frames.
    EndOfFile,
    /// The file ends inside a frame, or with a frame that a crash tore. `valid_bytes` is where the whole frames end.
    /// A tailer whose writer is still alive reads again later; once the writer is gone it is a torn tail.
    Incomplete { valid_bytes: u64 },
    /// A bad frame starts at `offset` with more of the file after it (`RRS-S2002`). Offset 0 means the file isn't
    /// an event log at all.
    Corrupt { offset: u64 },
}

/// An open event log and how far into it the reader has got.
#[derive(Debug)]
pub struct EventLogReader {
    file: File,
    path: PathBuf,
    /// Just past the last whole frame read; 0 until the file header has been checked.
    offset: u64,
    next_sequence: u64,
}

/// Opens a log for reading. Nothing is read yet: the first `read_next_frame` checks the header, so a tailer may open
/// a log whose writer hasn't written its header yet.
pub fn open_event_log(path: &Path) -> Result<EventLogReader, StoreError> {
    let file = File::open(path).map_err(|error| event_log_io_error(path, &error))?;
    Ok(EventLogReader {
        file,
        path: path.to_path_buf(),
        offset: 0,
        next_sequence: FIRST_SEQUENCE,
    })
}

/// The byte offset just past the last whole frame read: where the next frame starts.
pub fn reader_offset(reader: &EventLogReader) -> u64 {
    reader.offset
}

/// Reads the next frame. A record whose sequence isn't the previous one plus 1 is `RRS-S2003`
/// (`StoreError::EventSequenceGap`); the reader stays in front of it. After anything but a record, the reader also
/// stays where it was, so a tailer can call again once the writer has written more.
pub fn read_next_frame(reader: &mut EventLogReader) -> Result<FrameRead, StoreError> {
    if reader.offset == 0 {
        let header_problem = check_file_header(reader)?;
        if let Some(stop) = header_problem {
            return Ok(stop);
        }
    }
    let file_length = current_file_length(reader)?;
    let frame_start = reader.offset;

    let mut header_bytes = [0u8; FRAME_HEADER_BYTES];
    let header_bytes_read = read_at(reader, frame_start, &mut header_bytes)?;
    if header_bytes_read == 0 {
        return Ok(FrameRead::EndOfFile);
    }
    if header_bytes_read < FRAME_HEADER_BYTES {
        return Ok(FrameRead::Incomplete {
            valid_bytes: frame_start,
        });
    }
    let header = decode_frame_header(&header_bytes);
    let frame_end = frame_start + FRAME_HEADER_BYTES as u64 + u64::from(header.body_length);
    // Checked before allocating, so a damaged length can't ask for 4 GiB of memory.
    if frame_end > file_length {
        return Ok(FrameRead::Incomplete {
            valid_bytes: frame_start,
        });
    }

    let mut body = vec![0u8; header.body_length as usize];
    let body_bytes_read = read_at(reader, frame_start + FRAME_HEADER_BYTES as u64, &mut body)?;
    if body_bytes_read < body.len() {
        return Ok(FrameRead::Incomplete {
            valid_bytes: frame_start,
        });
    }
    let checksum_matches = body_matches_header(&header, &body);
    let record = if checksum_matches {
        decode_frame_body(&body)
    } else {
        None
    };
    let Some(record) = record else {
        return classify_bad_frame(reader, frame_start, frame_end, file_length, checksum_matches);
    };
    accept_record(reader, record, frame_end)
}

/// Moves past a good frame, after checking that its sequence follows the previous one.
fn accept_record(reader: &mut EventLogReader, record: EventRecord, frame_end: u64) -> Result<FrameRead, StoreError> {
    if record.sequence != reader.next_sequence {
        return Err(StoreError::EventSequenceGap {
            path: reader.path.clone(),
            expected_sequence: reader.next_sequence,
            found_sequence: record.sequence,
        });
    }
    reader.offset = frame_end;
    reader.next_sequence += 1;
    Ok(FrameRead::Record(record))
}

/// Checks the 8-byte file header and moves past it. `None` when it is there; otherwise what to report: a header the
/// writer hasn't finished (a prefix of `RRSEVLG1`, or zeros) is incomplete, anything else isn't an event log.
fn check_file_header(reader: &mut EventLogReader) -> Result<Option<FrameRead>, StoreError> {
    let mut header = [0u8; EVENT_LOG_HEADER.len()];
    let bytes_read = read_at(reader, 0, &mut header)?;
    if header[..bytes_read] != EVENT_LOG_HEADER[..bytes_read] {
        let file_length = current_file_length(reader)?;
        if rest_of_file_is_zero(reader, 0, file_length)? {
            return Ok(Some(FrameRead::Incomplete { valid_bytes: 0 }));
        }
        return Ok(Some(FrameRead::Corrupt { offset: 0 }));
    }
    if bytes_read < EVENT_LOG_HEADER.len() {
        return Ok(Some(FrameRead::Incomplete { valid_bytes: 0 }));
    }
    reader.offset = EVENT_LOG_HEADER.len() as u64;
    Ok(None)
}

/// A frame that failed its checksum, or whose checksum matched but whose body isn't a version-1 `EventRecord`:
/// the torn tail, or corruption (see the top of this file).
fn classify_bad_frame(
    reader: &mut EventLogReader,
    frame_start: u64,
    frame_end: u64,
    file_length: u64,
    checksum_matches: bool,
) -> Result<FrameRead, StoreError> {
    let torn_at_the_end = !checksum_matches && frame_end == file_length;
    if torn_at_the_end || rest_of_file_is_zero(reader, frame_start, file_length)? {
        return Ok(FrameRead::Incomplete {
            valid_bytes: frame_start,
        });
    }
    Ok(FrameRead::Corrupt { offset: frame_start })
}

/// True when every byte from `start` to `file_length` is zero.
fn rest_of_file_is_zero(reader: &mut EventLogReader, start: u64, file_length: u64) -> Result<bool, StoreError> {
    let mut chunk = vec![0u8; ZERO_SCAN_CHUNK_BYTES];
    let mut position = start;
    while position < file_length {
        let bytes_read = read_at(reader, position, &mut chunk)?;
        if bytes_read == 0 {
            break;
        }
        if chunk[..bytes_read].iter().any(|byte| *byte != 0) {
            return Ok(false);
        }
        position += bytes_read as u64;
    }
    Ok(true)
}

fn current_file_length(reader: &EventLogReader) -> Result<u64, StoreError> {
    let metadata = reader
        .file
        .metadata()
        .map_err(|error| event_log_io_error(&reader.path, &error))?;
    Ok(metadata.len())
}

/// Fills as much of `buffer` as the file holds from `offset` on and returns how many bytes that was. Fewer than
/// `buffer.len()` means the file ended (a plain `read` may also stop early for other reasons, so it loops).
fn read_at(reader: &mut EventLogReader, offset: u64, buffer: &mut [u8]) -> Result<usize, StoreError> {
    reader
        .file
        .seek(SeekFrom::Start(offset))
        .map_err(|error| event_log_io_error(&reader.path, &error))?;
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(event_log_io_error(&reader.path, &error)),
        }
    }
    Ok(filled)
}
