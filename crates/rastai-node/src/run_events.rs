//! `WatchRunEvents`: a run's `events.log`, tailed for the daemon (layout: rust-node-gateway-proto.md section 7.5;
//! platform flows 6.1 and 6.2; contracts 7.1).
//!
//! The job is the log's only writer; the node only reads it, through rastai-store's reader, and never changes it. The
//! daemon asks for the records after the last sequence it indexed (`after_sequence`, its acknowledgement); the node
//! keeps no acknowledgement of its own, so a daemon that died before indexing asks again and gets the same records.
//!
//! **The loop.** Read the frames after `after_sequence` and send each record. At the end of what is written, wait
//! `TAIL_POLL_MS` and read again while the run's writer is alive; once it is gone, the stream ends. Whether the writer
//! is alive is read before each read, never after: a writer seen dead had already written its last frame, so that
//! read gets it.
//!
//! **A torn tail** (a frame that isn't whole yet) is the writer mid-write while it lives, so the tail waits for the
//! rest. Once the writer is dead the frame will never be finished: the stream ends before it (`RRS-S2001`, logged),
//! and the file is left alone; a relaunched worker cuts the torn frame itself before it appends.
//!
//! **Damage** stops the stream: a bad frame with more of the file after it (`RRS-S2002`), a sequence gap
//! (`RRS-S2003`) or a log that can't be read (`RRS-S0006`). The records before it are sent, then the node's report:
//! a `diagnostic.reported` record with sequence 0, which no log record has, so the daemon can't take it for one or
//! acknowledge it (contracts 4.4). Nothing after the damage is guessed at. A `run_id` that isn't a run id is refused
//! the same way (`RRS-C0002`), before anything is read.
//!
//! **Positions.** `RunEventPositions` keeps, per run, where the frame of every `POSITION_MARK_EVERY`th sequence starts
//! and where the last read ended, in memory. A watch opens the log at the nearest position at or before the first
//! record it needs and reads on from there, so a daemon that reconnects deep into a long run doesn't make the node read
//! the whole log again. Frames between that position and `after_sequence` go through the same reader (checksum and
//! sequence checked) and are dropped. After a node restart the table is empty and the first watch reads from the
//! file header.
//!
//! The reads are blocking file work, so they run on tokio's blocking threads, a batch at a time; the waits don't hold a
//! thread at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::paths::run_events_log_path;
use rastai_core::time::now_unix_ms;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::log_diagnostic;
use rastai_proto::common::v1::EventClass;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{DiagnosticReported, EventPayload, EventRecord};
use rastai_store::error::StoreError;
use rastai_store::event_log::frame::EVENT_RECORD_FORMAT_VERSION;
use rastai_store::event_log::reader::{
    EventLogReader, FrameRead, open_event_log, open_event_log_at, read_next_frame, reader_offset,
};
use tokio::sync::mpsc;

use crate::jobs::supervisor::run_has_live_writer;
use crate::node::NodeState;

/// How often a caught-up tail looks for new frames while the writer lives (contracts 15.10). A poll of a few files
/// costs nothing and needs no file-watch dependency.
pub const TAIL_POLL_MS: u64 = 50;
/// Every this many sequences, the position of the next frame is kept for later watches (layout 7.5).
const POSITION_MARK_EVERY: u64 = 1000;
/// The most frames one blocking read takes before its records are sent, so a long log streams as it is read.
const FRAMES_PER_READ: usize = 256;
/// The sequence of the node's own report at the end of a damaged stream: no log record has it (logs start at 1).
pub const NODE_REPORT_SEQUENCE: u64 = 0;
const DIAGNOSTIC_REPORTED_EVENT_TYPE: &str = "diagnostic.reported";

/// Where in a run's log the frame with sequence `next_sequence` starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogPosition {
    next_sequence: u64,
    offset: u64,
}

/// The node's read positions in each run's log, in memory only (layout 7.5). A run's positions stay until the node
/// restarts: a few dozen bytes per run, but a node that lives through very many runs keeps them all. Forgetting a
/// finished run's positions belongs with reattach (M1-45), which knows when a run is done for good.
#[derive(Debug, Default)]
pub struct RunEventPositions {
    /// Per run, the position after every `POSITION_MARK_EVERY`th record, oldest first.
    marks: BTreeMap<String, Vec<LogPosition>>,
    /// Per run, where the last read ended.
    last_read: BTreeMap<String, LogPosition>,
}

/// One watch's place in its run's log, carried from one blocking read to the next.
struct RunTail {
    run_id: String,
    log_path: PathBuf,
    after_sequence: u64,
    /// None until the log exists and has been opened.
    reader: Option<EventLogReader>,
}

/// What one blocking read found: the records to send, then what to do next.
struct TailRead {
    records: Vec<EventRecord>,
    next: NextStep,
}

enum NextStep {
    /// More frames may follow at once.
    ReadAgain,
    /// Everything written so far was read and the writer lives: wait, then read again.
    Wait,
    /// The writer is gone and its last frame was read: the stream ends.
    Finished,
    /// The log is damaged here: the stream ends with this report.
    Damaged(Diagnostic),
}

/// Streams the records of `run_id`'s log after `after_sequence` to `sender` until the writer is gone and its last
/// frame was sent, the log turns out damaged (then the node's report is the last record), or the daemon closes the
/// stream. Never an error: what goes wrong in the log is the report.
pub async fn watch_run_events(
    state: &Arc<NodeState>,
    run_id: &str,
    after_sequence: u64,
    sender: mpsc::Sender<EventRecord>,
) {
    let log_path = run_events_log_path(&state.paths, run_id);
    if !id_has_prefix(run_id, IdPrefix::Run) {
        // The id becomes a folder name, so it must be a real id (contracts 8.6), never a path. A wrong id is the
        // daemon's request refused, not a log that failed to read.
        let _ = sender.send(node_report(run_id, run_id_refused(run_id))).await;
        return;
    }
    let mut tail = RunTail {
        run_id: run_id.to_string(),
        log_path,
        after_sequence,
        reader: None,
    };
    loop {
        let Some((returned_tail, read)) = read_in_background(state, tail).await else {
            return;
        };
        tail = returned_tail;
        for record in read.records {
            if sender.send(record).await.is_err() {
                return;
            }
        }
        match read.next {
            NextStep::ReadAgain => {}
            NextStep::Wait => {
                if !wait_unless_closed(&sender).await {
                    return;
                }
            }
            NextStep::Finished => return,
            NextStep::Damaged(found) => {
                log_diagnostic(&found, &[("run_id", run_id)]);
                let _ = sender.send(node_report(run_id, found)).await;
                return;
            }
        }
    }
}

/// The position to start a watch at: the nearest one kept for the run at or before the frame of
/// `after_sequence + 1`, or None to start at the file header.
fn nearest_position(positions: &RunEventPositions, run_id: &str, after_sequence: u64) -> Option<LogPosition> {
    let first_needed = after_sequence.saturating_add(1);
    let mut best: Option<LogPosition> = None;
    if let Some(marks) = positions.marks.get(run_id) {
        for mark in marks {
            best = nearer_fitting_position(best, *mark, first_needed);
        }
    }
    if let Some(last_read) = positions.last_read.get(run_id) {
        best = nearer_fitting_position(best, *last_read, first_needed);
    }
    best
}

/// `candidate` when it starts at or before the frame of `first_needed` and later than `best`, else `best`.
fn nearer_fitting_position(
    best: Option<LogPosition>,
    candidate: LogPosition,
    first_needed: u64,
) -> Option<LogPosition> {
    let fits = candidate.next_sequence <= first_needed;
    let nearer = match best {
        Some(found) => candidate.next_sequence > found.next_sequence,
        None => true,
    };
    if fits && nearer {
        return Some(candidate);
    }
    best
}

/// Records that the frame of `position.next_sequence` starts at `position.offset`: the run's last read, and a mark
/// when the record before it is a `POSITION_MARK_EVERY`th one.
fn remember_position(positions: &mut RunEventPositions, run_id: &str, position: LogPosition) {
    let record_sequence = position.next_sequence.saturating_sub(1);
    if record_sequence > 0 && record_sequence.is_multiple_of(POSITION_MARK_EVERY) {
        let marks = positions.marks.entry(run_id.to_string()).or_default();
        let is_new = marks
            .last()
            .is_none_or(|last| last.next_sequence < position.next_sequence);
        if is_new {
            marks.push(position);
        }
    }
    let last_read = positions.last_read.entry(run_id.to_string()).or_insert(position);
    if position.next_sequence >= last_read.next_sequence {
        *last_read = position;
    }
}

/// Forgets a run's positions, for a log that is no longer the file they were read from.
fn forget_positions(state: &NodeState, run_id: &str) {
    let mut positions = lock_positions(state);
    positions.marks.remove(run_id);
    positions.last_read.remove(run_id);
}

/// One `read_frames` on a blocking thread. None when that thread failed, which ends the watch.
async fn read_in_background(state: &Arc<NodeState>, mut tail: RunTail) -> Option<(RunTail, TailRead)> {
    let state = Arc::clone(state);
    let joined = tokio::task::spawn_blocking(move || {
        let read = read_frames(&state, &mut tail);
        (tail, read)
    })
    .await;
    joined.ok()
}

/// Waits `TAIL_POLL_MS`; false when the daemon closed the stream meanwhile.
async fn wait_unless_closed(sender: &mpsc::Sender<EventRecord>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(TAIL_POLL_MS)) => true,
        _ = sender.closed() => false,
    }
}

/// Reads up to `FRAMES_PER_READ` frames and decides what comes next. Runs on a blocking thread.
fn read_frames(state: &NodeState, tail: &mut RunTail) -> TailRead {
    let writer_alive = run_has_live_writer(state, &tail.run_id);
    let mut read = TailRead {
        records: Vec::new(),
        next: NextStep::ReadAgain,
    };
    if tail.reader.is_none() {
        match open_tail_reader(state, tail) {
            Ok(Some(reader)) => tail.reader = Some(reader),
            Ok(None) => {
                read.next = caught_up(writer_alive);
                return read;
            }
            Err(error) => {
                read.next = NextStep::Damaged(read_error_diagnostic(&error));
                return read;
            }
        }
    }
    let Some(reader) = tail.reader.as_mut() else {
        return read;
    };
    for _ in 0..FRAMES_PER_READ {
        match read_next_frame(reader) {
            Ok(FrameRead::Record(record)) => {
                let position = LogPosition {
                    next_sequence: record.sequence + 1,
                    offset: reader_offset(reader),
                };
                keep_position(state, &tail.run_id, position);
                if record.sequence > tail.after_sequence {
                    read.records.push(record);
                }
            }
            Ok(FrameRead::EndOfFile) => {
                read.next = caught_up(writer_alive);
                break;
            }
            Ok(FrameRead::Incomplete { .. }) => {
                read.next = caught_up(writer_alive);
                if !writer_alive {
                    log_diagnostic(&diagnostic("RRS-S2001", &[("path", &path_text(&tail.log_path))]), &[]);
                }
                break;
            }
            Ok(FrameRead::Corrupt { offset }) => {
                read.next = NextStep::Damaged(corrupt_log(&tail.log_path, offset));
                break;
            }
            Err(error) => {
                read.next = NextStep::Damaged(read_error_diagnostic(&error));
                break;
            }
        }
    }
    read
}

/// `remember_position` in the node's table.
fn keep_position(state: &NodeState, run_id: &str, position: LogPosition) {
    let mut positions = lock_positions(state);
    remember_position(&mut positions, run_id, position);
}

/// Opens the run's log at the nearest kept position. None while the log doesn't exist yet. A position past the end of
/// the file (the log was replaced since) is forgotten, and the read starts at the header.
fn open_tail_reader(state: &NodeState, tail: &RunTail) -> Result<Option<EventLogReader>, StoreError> {
    let metadata = match std::fs::metadata(&tail.log_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(StoreError::EventLogReadFailed {
                path: tail.log_path.clone(),
                reason: error.to_string(),
            });
        }
    };
    let start = nearest_position(&lock_positions(state), &tail.run_id, tail.after_sequence);
    let opened = match start {
        Some(position) if position.offset <= metadata.len() => {
            open_event_log_at(&tail.log_path, position.offset, position.next_sequence)
        }
        Some(_) => {
            forget_positions(state, &tail.run_id);
            open_event_log(&tail.log_path)
        }
        None => open_event_log(&tail.log_path),
    };
    opened.map(Some)
}

/// What to do at the end of what is written: wait while the writer lives, else finish.
fn caught_up(writer_alive: bool) -> NextStep {
    if writer_alive {
        NextStep::Wait
    } else {
        NextStep::Finished
    }
}

/// The node's report at the end of a damaged stream (see the top of this file).
fn node_report(run_id: &str, found: Diagnostic) -> EventRecord {
    let reported = DiagnosticReported {
        diagnostic: Some(diagnostic_to_proto(&found)),
        subject_id: run_id.to_string(),
    };
    EventRecord {
        format_version: EVENT_RECORD_FORMAT_VERSION,
        sequence: NODE_REPORT_SEQUENCE,
        run_id: run_id.to_string(),
        job_id: String::new(),
        emitted_unix_ms: now_unix_ms(),
        event_type: DIAGNOSTIC_REPORTED_EVENT_TYPE.to_string(),
        event_class: EventClass::Durable as i32,
        payload: Some(EventPayload {
            payload: Some(Payload::DiagnosticReported(reported)),
        }),
        blobs: Vec::new(),
    }
}

/// `RRS-S2002`: a bad frame at `offset` with more of the file after it.
fn corrupt_log(log_path: &Path, offset: u64) -> Diagnostic {
    let offset = offset.to_string();
    diagnostic("RRS-S2002", &[("path", &path_text(log_path)), ("offset", &offset)])
}

/// The reader's errors: a sequence gap (`RRS-S2003`); anything else is a log that couldn't be read (`RRS-S0006`).
fn read_error_diagnostic(error: &StoreError) -> Diagnostic {
    match error {
        StoreError::EventSequenceGap {
            path,
            expected_sequence,
            found_sequence,
        } => {
            let expected = expected_sequence.to_string();
            let found = found_sequence.to_string();
            diagnostic(
                "RRS-S2003",
                &[
                    ("path", &path_text(path)),
                    ("expected_sequence", &expected),
                    ("found_sequence", &found),
                ],
            )
        }
        StoreError::EventLogReadFailed { path, reason } => {
            read_failed(&format!("the event log {} can't be read: {reason}", path.display()))
        }
        other => read_failed(&format!("the event log can't be read: {other:?}")),
    }
}

fn read_failed(reason: &str) -> Diagnostic {
    diagnostic("RRS-S0006", &[("reason", reason)])
}

/// `RRS-C0002`: the daemon asked for a run id that isn't one.
fn run_id_refused(run_id: &str) -> Diagnostic {
    let found = format!("run_id {run_id:?}");
    diagnostic("RRS-C0002", &[("found", &found), ("expected", "a run_ id")])
}

fn lock_positions(state: &NodeState) -> MutexGuard<'_, RunEventPositions> {
    // A panic while the lock was held leaves plain data that is still whole, so the poison flag is ignored.
    state.run_events.lock().unwrap_or_else(PoisonError::into_inner)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(next_sequence: u64, offset: u64) -> LogPosition {
        LogPosition { next_sequence, offset }
    }

    /// The positions a read of `record_count` records leaves, with frames of `frame_bytes` after the 8-byte header.
    fn positions_after_reading(record_count: u64, frame_bytes: u64) -> RunEventPositions {
        let mut positions = RunEventPositions::default();
        for sequence in 1..=record_count {
            remember_position(
                &mut positions,
                "run_a",
                position(sequence + 1, 8 + sequence * frame_bytes),
            );
        }
        positions
    }

    #[test]
    fn a_mark_is_kept_every_thousandth_record_and_the_last_read_always() {
        let positions = positions_after_reading(2500, 10);
        let marks = &positions.marks["run_a"];
        assert_eq!(marks, &vec![position(1001, 8 + 10_000), position(2001, 8 + 20_000)]);
        assert_eq!(positions.last_read["run_a"], position(2501, 8 + 25_000));
    }

    #[test]
    fn a_watch_starts_at_the_nearest_position_at_or_before_what_it_needs() {
        let positions = positions_after_reading(2500, 10);
        // Before the first mark: the header.
        assert_eq!(nearest_position(&positions, "run_a", 0), None);
        assert_eq!(nearest_position(&positions, "run_a", 999), None);
        // Record 1001 is needed after 1000, and the first mark is exactly its frame.
        assert_eq!(
            nearest_position(&positions, "run_a", 1000),
            Some(position(1001, 10_008))
        );
        assert_eq!(
            nearest_position(&positions, "run_a", 1999),
            Some(position(1001, 10_008))
        );
        assert_eq!(
            nearest_position(&positions, "run_a", 2400),
            Some(position(2001, 20_008))
        );
        // At or after the last read: the last read.
        assert_eq!(
            nearest_position(&positions, "run_a", 2500),
            Some(position(2501, 25_008))
        );
        assert_eq!(
            nearest_position(&positions, "run_a", u64::MAX),
            Some(position(2501, 25_008))
        );
        // Another run has nothing kept.
        assert_eq!(nearest_position(&positions, "run_b", 2000), None);
    }

    #[test]
    fn reading_an_earlier_part_again_keeps_the_marks_and_the_furthest_read() {
        let mut positions = positions_after_reading(2500, 10);
        remember_position(&mut positions, "run_a", position(1001, 10_008));
        remember_position(&mut positions, "run_a", position(5, 48));
        assert_eq!(positions.marks["run_a"].len(), 2);
        assert_eq!(positions.last_read["run_a"], position(2501, 25_008));
    }

    #[test]
    fn the_report_cannot_be_taken_for_a_log_record() {
        let report = node_report("run_a", corrupt_log(Path::new("/data/runs/run_a/events.log"), 77));
        assert_eq!(report.sequence, NODE_REPORT_SEQUENCE);
        assert_eq!(report.event_type, "diagnostic.reported");
        let Some(EventPayload {
            payload: Some(Payload::DiagnosticReported(reported)),
        }) = report.payload
        else {
            panic!("the report carries no DiagnosticReported");
        };
        assert_eq!(reported.subject_id, "run_a");
        let carried = reported.diagnostic.unwrap_or_default();
        assert_eq!(carried.code, "RRS-S2002");
        assert!(carried.message.contains("byte 77"), "{}", carried.message);
    }
}
