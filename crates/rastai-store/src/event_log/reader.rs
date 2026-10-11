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
//! leave a zero-filled block) or, for a checksum mismatch, when nothing but zeros follows its declared end (the frame
//! ends at the end of the file, or a power cut kept its header but zeroed its body and the block after it). A bad
//! frame with real bytes after it is corruption (`RRS-S2002`).
//!
//! A frame whose declared end lies past the end of the file is usually the tail: the writer is still writing it, or
//! died while writing it. Bit rot in its `body_length` looks the same, though, and would make a torn-tail cut drop
//! every whole frame after it. The writer makes one write per frame, so a crash tears at most the last frame: when a
//! whole, CRC-valid, version-1 frame with a later sequence lies in the `LATER_FRAME_SEARCH_BYTES` after the bad
//! frame's header, the length was damaged and the frame is corrupt. The same search runs before a checksum mismatch
//! with only zeros after it is called the tail, so a length damaged to end inside a zero-filled tail can't hide the
//! frames inside its span.
//!
//! Where the search looks: when the bytes after the header start as the record this frame should hold (version 1,
//! the expected sequence), a later frame can only start where one of that record's top-level fields ends. A frame
//! inside a field is the record's own content, a text that quotes one, and proves nothing: otherwise a torn record
//! quoting a frame would read as corruption and stop the run from resuming. When the bytes don't start as that
//! record (its header and body are both damaged), every byte offset is a candidate. Either way the search checks at
//! most `LATER_FRAME_CRC_BUDGET_BYTES` of candidate bodies; once that runs out it gives up and the frame counts as
//! the torn tail, so crafted or unlucky bytes can't make one read slow. A candidate is only evidence: the reader
//! never returns it as a record.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rastai_proto::events::v1::EventRecord;

use crate::error::{StoreError, event_log_read_error};
use crate::event_log::frame::{
    EVENT_LOG_HEADER, FIRST_SEQUENCE, FRAME_HEADER_BYTES, FrameHeader, body_matches_header, body_starts_as_record,
    decode_frame_body, decode_frame_header, field_end_at,
};

/// How much of the file the zero-tail check reads at a time.
const ZERO_SCAN_CHUNK_BYTES: usize = 64 * 1024;
/// How far past a bad frame's header the reader looks for a whole later frame (see the top of this file).
/// A record carries hashes, not data, so a real frame is a few KiB: 1 MiB holds the damaged frame's true body and
/// the frame after it many times over, and it bounds the memory a damaged length can cost. A later frame that
/// doesn't lie wholly inside it isn't seen, and the overrunning frame is then cut off as the torn tail.
const LATER_FRAME_SEARCH_BYTES: usize = 1024 * 1024;
/// How many body bytes one search may run through CRC-32 in all. The window alone doesn't bound the time: every
/// byte offset is a candidate, and each candidate whose length fits costs a CRC over its body, so bytes crafted to
/// fit at every offset would cost about 1 MiB times 1 MiB (minutes per read, repeated on every tailer poll). With
/// this budget the work is linear, milliseconds at most. A real search spends a few KiB of it, because random
/// lengths rarely fit; when it runs out the bad frame counts as the torn tail.
const LATER_FRAME_CRC_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// What trying one candidate offset in the search found.
enum Candidate {
    LaterFrame,
    Nothing,
    BudgetSpent,
}

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
    let file = File::open(path).map_err(|error| event_log_read_error(path, &error))?;
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
        return torn_unless_a_later_frame_follows(reader, frame_start, file_length);
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
    if rest_of_file_is_zero(reader, frame_start, file_length)? {
        return Ok(FrameRead::Incomplete {
            valid_bytes: frame_start,
        });
    }
    // A matching checksum means these are the bytes the writer wrote, so a crash didn't make them.
    if !checksum_matches && rest_of_file_is_zero(reader, frame_end, file_length)? {
        return torn_unless_a_later_frame_follows(reader, frame_start, file_length);
    }
    Ok(FrameRead::Corrupt { offset: frame_start })
}

/// A bad frame with nothing but its own bytes and zeros left in the file: the torn (or still unfinished) tail,
/// unless a whole later frame follows its header, which proves its length is damaged (see the top of this file).
fn torn_unless_a_later_frame_follows(
    reader: &mut EventLogReader,
    frame_start: u64,
    file_length: u64,
) -> Result<FrameRead, StoreError> {
    let search_start = frame_start + FRAME_HEADER_BYTES as u64;
    if whole_later_frame_follows(reader, search_start, file_length)? {
        return Ok(FrameRead::Corrupt { offset: frame_start });
    }
    Ok(FrameRead::Incomplete {
        valid_bytes: frame_start,
    })
}

/// True when a whole later frame lies inside the `LATER_FRAME_SEARCH_BYTES` from `search_start`, the first byte
/// after the bad frame's header. False as well when the candidates use up `LATER_FRAME_CRC_BUDGET_BYTES` before one
/// is found.
fn whole_later_frame_follows(
    reader: &mut EventLogReader,
    search_start: u64,
    file_length: u64,
) -> Result<bool, StoreError> {
    let bytes_left = file_length.saturating_sub(search_start);
    let window_length = bytes_left.min(LATER_FRAME_SEARCH_BYTES as u64) as usize;
    let mut window = vec![0u8; window_length];
    let bytes_read = read_at(reader, search_start, &mut window)?;
    window.truncate(bytes_read);

    if body_starts_as_record(&window, reader.next_sequence) {
        return Ok(later_frame_at_a_field_end(&window, reader.next_sequence));
    }
    Ok(later_frame_at_any_offset(&window, reader.next_sequence))
}

/// The search when `window` starts as the bad frame's own record: a later frame can only start where one of the
/// record's top-level fields ends, because anything inside a field is the record's own content. Past the record's
/// true end the walk reads the next frame's bytes as fields, but by then the true end has been tried.
fn later_frame_at_a_field_end(window: &[u8], next_sequence: u64) -> bool {
    let mut crc_budget_left = LATER_FRAME_CRC_BUDGET_BYTES;
    let mut field_end = 0;
    while let Some(next_field_end) = field_end_at(window, field_end) {
        field_end = next_field_end;
        match try_candidate(&window[field_end..], next_sequence, &mut crc_budget_left) {
            Candidate::LaterFrame => return true,
            Candidate::BudgetSpent => return false,
            Candidate::Nothing => {}
        }
    }
    false
}

/// The search when `window` doesn't start as the bad frame's own record (its body is damaged too): every byte offset
/// is tried, because the damaged length says nothing about where the next frame really starts.
fn later_frame_at_any_offset(window: &[u8], next_sequence: u64) -> bool {
    let mut crc_budget_left = LATER_FRAME_CRC_BUDGET_BYTES;
    for candidate_start in 0..window.len() {
        match try_candidate(&window[candidate_start..], next_sequence, &mut crc_budget_left) {
            Candidate::LaterFrame => return true,
            Candidate::BudgetSpent => return false,
            Candidate::Nothing => {}
        }
    }
    false
}

/// Whether `bytes` start with a whole later frame, paying for its CRC-32 out of `crc_budget_left`. A candidate whose
/// declared body doesn't fit in `bytes` costs nothing.
fn try_candidate(bytes: &[u8], next_sequence: u64, crc_budget_left: &mut usize) -> Candidate {
    if bytes.len() < FRAME_HEADER_BYTES {
        return Candidate::Nothing;
    }
    let Some(body_length) = body_length_if_it_fits(bytes) else {
        return Candidate::Nothing;
    };
    if body_length > *crc_budget_left {
        return Candidate::BudgetSpent;
    }
    *crc_budget_left -= body_length;
    if is_whole_later_frame(bytes, body_length, next_sequence) {
        return Candidate::LaterFrame;
    }
    Candidate::Nothing
}

/// The body length the frame header at the start of `bytes` declares, when that body lies wholly inside `bytes`;
/// `None` when it doesn't (then the candidate costs no CRC).
fn body_length_if_it_fits(bytes: &[u8]) -> Option<usize> {
    let header = frame_header_at(bytes);
    let body_end = FRAME_HEADER_BYTES as u64 + u64::from(header.body_length);
    if body_end > bytes.len() as u64 {
        return None;
    }
    Some(header.body_length as usize)
}

/// True when `bytes` start with a whole frame of `body_length` (which `body_length_if_it_fits` found fits): a body
/// with the header's CRC-32, holding a version-1 record whose sequence is `next_sequence` or later. Only a later
/// sequence shows the writer got past this frame; a copy of a frame the reader has already passed shows nothing.
fn is_whole_later_frame(bytes: &[u8], body_length: usize, next_sequence: u64) -> bool {
    let header = frame_header_at(bytes);
    let body = &bytes[FRAME_HEADER_BYTES..FRAME_HEADER_BYTES + body_length];
    if !body_matches_header(&header, body) {
        return false;
    }
    match decode_frame_body(body) {
        Some(record) => record.sequence >= next_sequence,
        None => false,
    }
}

/// The frame header in the first `FRAME_HEADER_BYTES` of `bytes` (the caller makes sure there are that many).
fn frame_header_at(bytes: &[u8]) -> FrameHeader {
    let mut header_bytes = [0u8; FRAME_HEADER_BYTES];
    header_bytes.copy_from_slice(&bytes[..FRAME_HEADER_BYTES]);
    decode_frame_header(&header_bytes)
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
        .map_err(|error| event_log_read_error(&reader.path, &error))?;
    Ok(metadata.len())
}

/// Fills as much of `buffer` as the file holds from `offset` on and returns how many bytes that was. Fewer than
/// `buffer.len()` means the file ended (a plain `read` may also stop early for other reasons, so it loops).
fn read_at(reader: &mut EventLogReader, offset: u64, buffer: &mut [u8]) -> Result<usize, StoreError> {
    reader
        .file
        .seek(SeekFrom::Start(offset))
        .map_err(|error| event_log_read_error(&reader.path, &error))?;
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(event_log_read_error(&reader.path, &error)),
        }
    }
    Ok(filled)
}
