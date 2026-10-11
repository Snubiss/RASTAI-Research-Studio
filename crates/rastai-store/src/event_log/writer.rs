//! Writing an event log (contracts 7.1, flows/platform.md 6.1 and 6.2). Each log has exactly one writer: the job,
//! or the daemon for its own log, its mirrors of remote runs and chain runs (layout 6.6). The store provides these
//! functions; it never decides who calls them.
//!
//! - `create_event_log` starts a new log: the header, fsynced, and the folder fsynced so the file itself survives a
//!   power cut.
//! - `open_event_log_for_append` reopens one after a crash. It reads every frame through the reader, cuts a torn
//!   tail off (`RRS-S2001`, info) and continues from the last good sequence plus 1. A corrupt frame or a sequence gap
//!   before the end is an error: appending after it would hide it.
//! - `append_event` writes one frame with a single write, so a crash leaves at most one torn frame. If the write
//!   itself fails (a full disk), the partial frame is cut off again before the error returns.
//! - fsync: `SyncPolicy::Now` after state changes and checkpoints; `SyncPolicy::Batched` for metrics, synced at
//!   least once a second while appends continue. A writer that may go quiet calls `sync_event_log` on its own timer.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rastai_proto::events::v1::EventRecord;

use crate::error::{StoreError, event_log_io_error};
use crate::event_log::frame::{EVENT_LOG_HEADER, EVENT_RECORD_FORMAT_VERSION, FIRST_SEQUENCE, encode_frame};
use crate::event_log::reader::{FrameRead, open_event_log, read_next_frame, reader_offset};

/// Batched appends (metrics) are fsynced at least this often (contracts 7.1: "at least once a second"). The same
/// name and value as the Python worker's writer (contracts 15.10).
const METRICS_FSYNC_INTERVAL_SECONDS: u64 = 1;
const FRAME_TOO_LARGE_REASON: &str = "the record is larger than a frame's 4 GiB length field allows";

/// When an append is made durable against a power cut. Every append survives the writer being killed either way:
/// the bytes are in the operating system's page cache as soon as the write returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// fsync before returning: state changes and checkpoints.
    Now,
    /// fsync when the last one was a second or more ago: metrics.
    Batched,
}

/// The one open writer of a log.
#[derive(Debug)]
pub struct EventLogWriter {
    file: File,
    path: PathBuf,
    /// Where the next frame goes: the end of the last whole frame.
    end_offset: u64,
    next_sequence: u64,
    /// Bytes written since the last fsync, and when that fsync was.
    has_unsynced_bytes: bool,
    last_sync: Instant,
    truncated_tail_bytes: u64,
}

/// Creates a new, empty log at `path`. Refuses a path that already exists, so a log is never overwritten.
pub fn create_event_log(path: &Path) -> Result<EventLogWriter, StoreError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| event_log_io_error(path, &error))?;
    file.write_all(EVENT_LOG_HEADER)
        .map_err(|error| event_log_io_error(path, &error))?;
    file.sync_all().map_err(|error| event_log_io_error(path, &error))?;
    sync_parent_folder(path)?;
    Ok(new_writer(file, path, EVENT_LOG_HEADER.len() as u64, FIRST_SEQUENCE, 0))
}

/// Opens an existing log to append to it, after a crash or a restart. A torn tail is truncated first (the caller
/// logs `RRS-S2001` when `truncated_tail_bytes` isn't 0). A corrupt frame (`RRS-S2002`) or a sequence gap
/// (`RRS-S2003`) anywhere is returned as an error and the file is left as it is.
pub fn open_event_log_for_append(path: &Path) -> Result<EventLogWriter, StoreError> {
    let (valid_bytes, last_sequence) = find_end_of_whole_frames(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| event_log_io_error(path, &error))?;
    let file_length = file.metadata().map_err(|error| event_log_io_error(path, &error))?.len();
    if valid_bytes < file_length {
        cut_file_to(&file, path, valid_bytes)?;
    }
    let mut end_offset = valid_bytes;
    if valid_bytes == 0 {
        // The writer died before its header was whole: start the file again.
        write_header_at_start(&mut file, path)?;
        end_offset = EVENT_LOG_HEADER.len() as u64;
    }
    let truncated_tail_bytes = file_length.saturating_sub(valid_bytes);
    Ok(new_writer(
        file,
        path,
        end_offset,
        last_sequence + 1,
        truncated_tail_bytes,
    ))
}

/// Appends `record` as the log's next frame and returns its sequence. The writer sets `sequence` and
/// `format_version` on the record, so the caller can't break the 1, 2, 3 ... rule.
pub fn append_event(
    writer: &mut EventLogWriter,
    record: &mut EventRecord,
    sync: SyncPolicy,
) -> Result<u64, StoreError> {
    record.sequence = writer.next_sequence;
    record.format_version = EVENT_RECORD_FORMAT_VERSION;
    let Some(frame) = encode_frame(record) else {
        return Err(StoreError::EventLogIoFailed {
            path: writer.path.clone(),
            reason: FRAME_TOO_LARGE_REASON.to_string(),
        });
    };
    write_frame_at_end(writer, &frame)?;
    writer.next_sequence += 1;
    if sync == SyncPolicy::Now || batched_sync_is_due(writer.last_sync, Instant::now()) {
        sync_event_log(writer)?;
    }
    Ok(record.sequence)
}

/// fsyncs whatever was appended since the last fsync.
pub fn sync_event_log(writer: &mut EventLogWriter) -> Result<(), StoreError> {
    if writer.has_unsynced_bytes {
        writer
            .file
            .sync_data()
            .map_err(|error| event_log_io_error(&writer.path, &error))?;
        writer.has_unsynced_bytes = false;
    }
    writer.last_sync = Instant::now();
    Ok(())
}

/// How many bytes of a torn tail `open_event_log_for_append` cut off; 0 for a writer that cut nothing.
pub fn truncated_tail_bytes(writer: &EventLogWriter) -> u64 {
    writer.truncated_tail_bytes
}

fn new_writer(
    file: File,
    path: &Path,
    end_offset: u64,
    next_sequence: u64,
    truncated_tail_bytes: u64,
) -> EventLogWriter {
    EventLogWriter {
        file,
        path: path.to_path_buf(),
        end_offset,
        next_sequence,
        has_unsynced_bytes: false,
        last_sync: Instant::now(),
        truncated_tail_bytes,
    }
}

/// Reads the whole log through the reader: where its whole frames end and the last sequence in it (0 when there
/// are no records).
fn find_end_of_whole_frames(path: &Path) -> Result<(u64, u64), StoreError> {
    let mut reader = open_event_log(path)?;
    let mut last_sequence = 0;
    loop {
        match read_next_frame(&mut reader)? {
            FrameRead::Record(record) => last_sequence = record.sequence,
            FrameRead::EndOfFile => return Ok((reader_offset(&reader), last_sequence)),
            FrameRead::Incomplete { valid_bytes } => return Ok((valid_bytes, last_sequence)),
            FrameRead::Corrupt { offset } => {
                return Err(StoreError::EventLogCorrupt {
                    path: path.to_path_buf(),
                    offset,
                });
            }
        }
    }
}

/// One write per frame, at the end of the last whole frame. On failure the file is cut back to that point, so a
/// failed append never leaves a torn frame for the next reader.
fn write_frame_at_end(writer: &mut EventLogWriter, frame: &[u8]) -> Result<(), StoreError> {
    let written = writer
        .file
        .seek(SeekFrom::Start(writer.end_offset))
        .and_then(|_| writer.file.write_all(frame));
    if let Err(error) = written {
        // Best effort: if even the cut fails, the next open_event_log_for_append cuts the torn frame instead.
        let _ = writer.file.set_len(writer.end_offset);
        return Err(event_log_io_error(&writer.path, &error));
    }
    writer.end_offset += frame.len() as u64;
    writer.has_unsynced_bytes = true;
    Ok(())
}

/// Truncates the file to `length` and fsyncs, so the cut is durable before anything new is appended after it.
fn cut_file_to(file: &File, path: &Path, length: u64) -> Result<(), StoreError> {
    file.set_len(length).map_err(|error| event_log_io_error(path, &error))?;
    file.sync_all().map_err(|error| event_log_io_error(path, &error))
}

fn write_header_at_start(file: &mut File, path: &Path) -> Result<(), StoreError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| event_log_io_error(path, &error))?;
    file.write_all(EVENT_LOG_HEADER)
        .map_err(|error| event_log_io_error(path, &error))?;
    file.sync_all().map_err(|error| event_log_io_error(path, &error))
}

/// For `SyncPolicy::Batched`: whether the last fsync is old enough that this append should fsync.
fn batched_sync_is_due(last_sync: Instant, now: Instant) -> bool {
    now.duration_since(last_sync) >= Duration::from_secs(METRICS_FSYNC_INTERVAL_SECONDS)
}

/// fsyncs the folder that holds a new log, so its directory entry survives a power cut too. Windows can't open a
/// folder as a file and makes the entry durable with the file, so there it is skipped.
#[cfg(unix)]
fn sync_parent_folder(path: &Path) -> Result<(), StoreError> {
    let Some(folder) = path.parent() else {
        return Ok(());
    };
    // A bare file name has an empty parent: the current folder.
    let folder = if folder.as_os_str().is_empty() {
        Path::new(".")
    } else {
        folder
    };
    let handle = File::open(folder).map_err(|error| event_log_io_error(folder, &error))?;
    handle.sync_all().map_err(|error| event_log_io_error(folder, &error))
}

#[cfg(not(unix))]
fn sync_parent_folder(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batched_sync_is_due_after_one_second() {
        let last_sync = Instant::now();
        assert!(!batched_sync_is_due(last_sync, last_sync));
        assert!(!batched_sync_is_due(last_sync, last_sync + Duration::from_millis(999)));
        assert!(batched_sync_is_due(last_sync, last_sync + Duration::from_secs(1)));
    }
}
