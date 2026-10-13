//! At startup, before the listener opens: index every daemon-log record after its segment's cursor
//! (`log_cursors.acked_sequence`, layout 13.1, flows/platform.md 6.4). A crash between a record's append and its
//! index commit leaves it only in the log; this puts its rows back, so the first request already sees them. A
//! replayed `command.finished` writes its outcome over the journal row, so a command whose record reached the log
//! keeps its real outcome, before or after the unfinished rows are closed with `RRS-C0005`.
//!
//! Records are read with the store's reader and indexed in batches of `INDEX_BATCH_RECORDS`, each batch with its
//! cursor in one transaction. Reading stops at a torn tail (the writer died mid-append; the next append cuts it off)
//! and at damage (`RRS-S2002`, `RRS-S2003`), which is logged: what came before it is indexed, and the daemon starts.
//! Nothing is published: no client is connected yet, and these records are older than any client's stream.

use rastai_diag::logger::log_warn;
use rastai_proto::events::v1::EventRecord;
use rastai_store::event_log::daemon_log::{DaemonLogSegment, daemon_log_segments};
use rastai_store::event_log::reader::{EventLogReader, FrameRead, open_event_log, read_next_frame};
use rastai_store::index::log_cursors::get_log_cursor;
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;

use crate::error::ServerError;
use crate::server_state::Server;

/// Records per index transaction (contracts 15.10, the same as a run tail's batches).
const INDEX_BATCH_RECORDS: usize = 256;
const CODE_EVENT_LOG_CORRUPT: &str = "RRS-S2002";

/// Indexes the records of every daemon-log segment that are after its cursor. A failed index write stops the
/// start (a full disk is `RRS-S3002`); a damaged segment doesn't.
pub async fn replay_daemon_log(server: &Server) -> Result<(), ServerError> {
    for segment in daemon_log_segments(&server.paths) {
        if segment.path.exists() {
            replay_segment(server, &segment).await?;
        }
    }
    Ok(())
}

async fn replay_segment(server: &Server, segment: &DaemonLogSegment) -> Result<(), ServerError> {
    let acked_sequence = read_cursor(server, &segment.log_name)?;
    let path = segment.path.display().to_string();
    let mut reader = match open_event_log(&segment.path) {
        Ok(reader) => reader,
        Err(error) => {
            log_warn(
                "a daemon log segment can't be read",
                &[("code", error.code()), ("path", &path)],
            );
            return Ok(());
        }
    };
    let mut batch: Vec<EventRecord> = Vec::new();
    while let Some(record) = next_record(&mut reader, &path) {
        if record.sequence <= acked_sequence {
            continue;
        }
        batch.push(record);
        if batch.len() == INDEX_BATCH_RECORDS {
            index_batch(server, &segment.log_name, std::mem::take(&mut batch)).await?;
        }
    }
    if !batch.is_empty() {
        index_batch(server, &segment.log_name, batch).await?;
    }
    Ok(())
}

fn read_cursor(server: &Server, log_name: &str) -> Result<u64, ServerError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    Ok(get_log_cursor(&reader, log_name)?.unwrap_or(0))
}

/// The segment's next whole record, or `None` at its end, a torn tail or damage (which is logged).
fn next_record(reader: &mut EventLogReader, path: &str) -> Option<EventRecord> {
    match read_next_frame(reader) {
        Ok(FrameRead::Record(record)) => Some(record),
        Ok(FrameRead::EndOfFile) | Ok(FrameRead::Incomplete { .. }) => None,
        Ok(FrameRead::Corrupt { offset }) => {
            let offset = offset.to_string();
            log_warn(
                "the daemon log is damaged; the records before the damage were indexed",
                &[("code", CODE_EVENT_LOG_CORRUPT), ("path", path), ("offset", &offset)],
            );
            None
        }
        Err(error) => {
            log_warn(
                "the daemon log can't be read further; the records before were indexed",
                &[("code", error.code()), ("path", path)],
            );
            None
        }
    }
}

async fn index_batch(server: &Server, log_name: &str, records: Vec<EventRecord>) -> Result<(), ServerError> {
    let write = IndexWrite::ApplyDaemonEvents {
        log_name: log_name.to_string(),
        records,
    };
    write_index(&server.store.index, write).await?;
    Ok(())
}
