//! The daemon's own durable records (contracts 7.1, layout 13.1): `command.finished`, `node.state_changed`,
//! `pin.changed`, `run.tags_changed` and the rest no worker writes. `record_daemon_event` appends each to
//! `<data>/daemon/events.log` and fsyncs, indexes it with `ApplyDaemonEvents` (its rows and `log_cursors` in one
//! transaction), then publishes it on the bus. A rebuild of `rastai.db` replays the same log, so nothing the daemon
//! decided is lost with the index.
//!
//! - **One at a time.** The log's cursor is the last indexed sequence and only moves forward, so records must be
//!   indexed in log order: an append and its index write happen under one lock.
//! - **A failed index write is retried with the next record.** The record is already in the log; indexing a later
//!   one alone would move the cursor past it, so it waits in `unindexed`, goes with the next batch and is published
//!   with it. Startup's replay catches anything a crash left behind.
//! - **Segments.** The live file is `events.log`; at `DAEMON_LOG_SEGMENT_BYTES` it is renamed `events-<n>.log` and a
//!   new one started. Each segment has its own sequence and its own cursor, named `daemon:<n>` from the day it is
//!   started, so the name survives the rename: the live file is segment (rolled segments + 1). The store lists and
//!   names them (`rastai_store::event_log::daemon_log`), the same way for this writer and the offline rebuild. A live
//!   file that can't be appended to (a corrupt frame, a sequence gap) is rolled the same way, so the damage stays in
//!   a closed segment and new records still have a log.

use std::path::Path;

use rastai_core::paths::{DataPaths, daemon_events_log_path, daemon_events_segment_path};
use rastai_core::time::now_unix_ms;
use rastai_diag::logger::log_warn;
use rastai_proto::common::v1::EventClass;
use rastai_proto::events::v1::EventRecord;
use rastai_store::error::StoreError;
use rastai_store::event_log::daemon_log::{DaemonLogSegment, daemon_log_name, daemon_log_segments};
use rastai_store::event_log::writer::{
    EventLogWriter, SyncPolicy, append_event, create_event_log, open_event_log_for_append, truncated_tail_bytes,
};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;

use crate::events::bus::{bus_event_from_record, publish};
use crate::server_state::Server;

/// A segment rolls once it reaches this size (contracts 7.1: 256 MiB).
pub const DAEMON_LOG_SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
/// The bus source of every record the daemon makes itself (contracts 7.2).
pub const DAEMON_SOURCE: &str = "daemon";
const FIRST_SEGMENT_NUMBER: u64 = 1;

/// The open live segment, its number, and the records appended to it but not indexed yet. Behind
/// `Server.daemon_log`, a tokio mutex held across the index write.
#[derive(Debug, Default)]
pub struct DaemonLog {
    writer: Option<EventLogWriter>,
    segment_number: u64,
    unindexed: Vec<EventRecord>,
}

/// A daemon log nothing has been written to yet in this process; the first record opens its live segment.
pub fn new_daemon_log() -> DaemonLog {
    DaemonLog::default()
}

/// Appends `record` to the daemon log, indexes it, then publishes it (layout 13.1). The record's sequence, format
/// version and class are set here; a record without a time gets the current one. An error means the record isn't
/// indexed yet: when it says the append failed, the record is lost, and otherwise it is in the log and is indexed
/// with the next record or at the next start.
pub async fn record_daemon_event(server: &Server, mut record: EventRecord) -> Result<(), StoreError> {
    record.event_class = EventClass::Durable as i32;
    if record.emitted_unix_ms == 0 {
        record.emitted_unix_ms = now_unix_ms();
    }
    let mut log = server.daemon_log.lock().await;
    append_to_daemon_log(&server.paths, &mut log, &mut record)?;
    log.unindexed.push(record.clone());
    let write = IndexWrite::ApplyDaemonEvents {
        log_name: daemon_log_name(log.segment_number),
        records: log.unindexed.clone(),
    };
    write_index(&server.store.index, write).await?;
    // Every record of the batch that just committed is published, the ones an earlier failed write left behind
    // too, while the lock is held, so the bus has the daemon's records in log order.
    let committed = std::mem::take(&mut log.unindexed);
    for committed_record in &committed {
        publish(&server.bus, bus_event_from_record(committed_record, DAEMON_SOURCE));
    }
    Ok(())
}

/// Opens the live segment if it isn't open yet, rolls it when it is full, and appends with an fsync.
fn append_to_daemon_log(paths: &DataPaths, log: &mut DaemonLog, record: &mut EventRecord) -> Result<(), StoreError> {
    // A full segment rolls only once everything in it is indexed, so `unindexed` always belongs to one segment.
    if log.writer.is_some() && log.unindexed.is_empty() && live_segment_is_full(paths) {
        log.writer = None;
        roll_live_segment(paths, log.segment_number)?;
    }
    if log.writer.is_none() {
        open_live_segment(paths, log)?;
    }
    let Some(writer) = log.writer.as_mut() else {
        return Err(StoreError::WriterStopped);
    };
    append_event(writer, record, SyncPolicy::Now)?;
    Ok(())
}

/// Opens `events.log` to append to it, or creates it. One that can't be appended to is rolled aside first.
fn open_live_segment(paths: &DataPaths, log: &mut DaemonLog) -> Result<(), StoreError> {
    let live = daemon_log_segments(paths).pop().unwrap_or(DaemonLogSegment {
        segment_number: FIRST_SEGMENT_NUMBER,
        log_name: daemon_log_name(FIRST_SEGMENT_NUMBER),
        path: daemon_events_log_path(paths),
    });
    let mut segment_number = live.segment_number;
    if !live.path.exists() {
        create_daemon_folder(&live.path)?;
        log.writer = Some(create_event_log(&live.path)?);
        log.segment_number = segment_number;
        return Ok(());
    }
    let writer = match open_event_log_for_append(&live.path) {
        Ok(writer) => writer,
        Err(damage) => {
            log_warn(
                "the daemon log can't be appended to; it is closed as a segment and a new one started",
                &[("code", damage.code()), ("path", &live.path.display().to_string())],
            );
            roll_live_segment(paths, segment_number)?;
            segment_number += 1;
            create_event_log(&live.path)?
        }
    };
    if truncated_tail_bytes(&writer) > 0 {
        log_warn(
            "a torn record at the end of the daemon log was cut off",
            &[("code", "RRS-S2001"), ("path", &live.path.display().to_string())],
        );
    }
    log.writer = Some(writer);
    log.segment_number = segment_number;
    Ok(())
}

/// Renames the live `events.log` to `events-<segment_number>.log`; the next record starts a new live file.
fn roll_live_segment(paths: &DataPaths, segment_number: u64) -> Result<(), StoreError> {
    let live = daemon_events_log_path(paths);
    let rolled = daemon_events_segment_path(paths, segment_number);
    std::fs::rename(&live, &rolled).map_err(|error| StoreError::EventLogWriteFailed {
        path: live,
        reason: error.to_string(),
    })
}

fn live_segment_is_full(paths: &DataPaths) -> bool {
    match std::fs::metadata(daemon_events_log_path(paths)) {
        Ok(metadata) => metadata.len() >= DAEMON_LOG_SEGMENT_BYTES,
        Err(_) => false,
    }
}

fn create_daemon_folder(live_path: &Path) -> Result<(), StoreError> {
    let Some(folder) = live_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(folder).map_err(|error| StoreError::EventLogWriteFailed {
        path: folder.to_path_buf(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_paths(test_name: &str) -> DataPaths {
        let root = std::env::temp_dir().join(format!("rsv-unit-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        }
    }

    #[test]
    fn a_live_segment_that_cant_be_appended_to_is_rolled_aside() {
        let paths = test_paths("roll-damaged");
        let live = daemon_events_log_path(&paths);
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        // Not an event log at all: corrupt at offset 0.
        std::fs::write(
            &live,
            b"not an event log, and long enough to be more than a torn header",
        )
        .unwrap();
        let mut log = new_daemon_log();
        let mut record = EventRecord::default();
        append_to_daemon_log(&paths, &mut log, &mut record).unwrap();
        assert_eq!(record.sequence, 1);
        assert_eq!(log.segment_number, 2);
        assert!(daemon_events_segment_path(&paths, 1).exists());
        let names: Vec<String> = daemon_log_segments(&paths)
            .into_iter()
            .map(|segment| segment.log_name)
            .collect();
        assert_eq!(names, ["daemon:1", "daemon:2"]);
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }
}
