//! One tail per open run (layout 13.1, flows/platform.md 6.1): `NodeService.WatchRunEvents` after the run's ack,
//! then, batch by batch, the daemon mirror for a remote run, the index, and the bus.
//!
//! - **The ack is the cursor.** `runs.acked_sequence` commits in the same transaction as the rows it covers
//!   (`ApplyRunEvents`), and every (re)connect watches after it, so a crash anywhere re-sends records the upserts
//!   absorb and nothing is indexed twice.
//! - **Batches** are up to `INDEX_BATCH_RECORDS` records or `INDEX_BATCH_MS` after the first, one transaction each.
//! - **Remote runs** get a mirror, `<data>/runs/<run_id>/events.log` on this machine, written and synced before the
//!   index (the embedded node shares the run's own file, so it needs none). After a restart the tail indexes the
//!   mirror's records after the ack first, then watches after the mirror's last record.
//! - **The node's report** is the record with sequence 0 (contracts 4.4): the stream's last, about a damaged or
//!   unreadable log (`RRS-S2002`, `RRS-S2003`, `RRS-S0006`). It is published, never mirrored, indexed or
//!   acknowledged, and the tail stops: that run's index stays incomplete until the log is repaired.
//! - **Run logs are untrusted.** A record the run's log may not carry is refused by the store (`RRS-S2004`): the
//!   records before it are indexed, the refusal is published and the tail stops there. After a restart it stops at
//!   the same record again. Only that run's indexing stops.
//! - **A full disk** (`RRS-S3002`) is retried every `INDEX_DISK_FULL_RETRY_SECONDS`; the records wait in the logs.
//! - **A broken stream** is watched again after `NODE_RECONNECT_BACKOFF_SECONDS`, 1 doubling to 30: the tail owns
//!   its own reconnection. The tail ends when the node ends the stream (contracts 4.4: the run's writer is gone and
//!   its last whole frame was sent) or the daemon stops.
//! - **One tail per run.** A tail holds its run's place in `Server::tailed_run_ids` until it ends, so
//!   `start_run_tails`, called again after the node link reconnects, skips the runs whose tail is still watching:
//!   two tails of one run would index and publish its records twice and write one mirror from two writers.
//!
//! The node is a `RunEventSource`. The daemon's node links are one (nodes/node_links.rs, `NodeConnection`); the tests
//! use a fake node.

use std::collections::HashSet;
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;

use rastai_core::paths::{run_dir, run_events_log_path};
use rastai_core::states::{is_terminal_eval_run_state, parse_eval_run_state, parse_run_state};
use rastai_core::time::now_unix_ms;
use rastai_core::wire::diagnostic_to_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_diag::logger::log_warn;
use rastai_proto::common::v1::{EventClass, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::{BusEvent, DiagnosticReported, EventPayload, EventRecord};
use rastai_store::error::StoreError;
use rastai_store::event_log::frame::encode_frame;
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
use rastai_store::event_log::writer::{
    EventLogWriter, append_raw_frame, create_event_log, open_event_log_for_append, sync_event_log,
};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{RunFilter, get_run, list_runs};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::error::store_error_diagnostic;
use crate::events::bus::{bus_event_from_record, publish};
use crate::events::daemon_events::DAEMON_SOURCE;
use crate::server_state::Server;
use crate::shutdown::wait_until_stopping;

/// Records per index transaction, and how long a batch waits for more after its first (contracts 15.10).
pub const INDEX_BATCH_RECORDS: usize = 256;
pub const INDEX_BATCH_MS: u64 = 100;
/// How often a write that hit a full disk is tried again (contracts 15.10).
pub const INDEX_DISK_FULL_RETRY_SECONDS: u64 = 10;
/// The wait before watching again after a broken stream: 1 s, doubling to 30 s (contracts 15.10).
const NODE_RECONNECT_FIRST_BACKOFF_SECONDS: u64 = 1;
const NODE_RECONNECT_MAX_BACKOFF_SECONDS: u64 = 30;
/// No log record has sequence 0: a record with it is the node's report about the log (contracts 4.4).
const REPORT_SEQUENCE: u64 = 0;
/// The training-run states that end a run (contracts 9.5). `completed` is the only terminal one, since a resume
/// reopens the other two, but a resume starts a new tail, so none of the three needs one.
const RUN_STATES_THAT_END_A_RUN: &[RunState] = &[RunState::Completed, RunState::Failed, RunState::Cancelled];
const JOB_SOURCE_PREFIX: &str = "job:";
const NODE_SOURCE_PREFIX: &str = "node:";
const DIAGNOSTIC_REPORTED_EVENT: &str = "diagnostic.reported";
const CODE_DISK_FULL: &str = "RRS-S3002";
const CODE_EVENT_LOG_CORRUPT: &str = "RRS-S2002";
const CODE_RUN_LOG_RECORD_REFUSED: &str = "RRS-S2004";

/// The node a run's records come from. The node links implement it over `NodeService.WatchRunEvents`.
/// A source is one long-lived object per node that survives reconnects, never one per connection: a tail keeps its
/// claim and re-watches through the source it started with, so a source made per connection would leave the run
/// skipped by `start_run_tails` and the old tail retrying a dead link.
pub trait RunEventSource: Send + Sync {
    /// The node's id, as `runs.node_id` holds it.
    fn node_id(&self) -> &str;
    /// True for a node on another machine: its runs get a daemon mirror. The embedded node shares `<data>`.
    fn is_remote(&self) -> bool;
    /// Starts watching the run's log after `after_sequence`. The channel carries its records in order and closes
    /// when the node ends the stream; an `Err` means the stream broke, and the tail watches again later.
    fn watch_run_events(&self, run_id: &str, after_sequence: u64) -> mpsc::Receiver<Result<EventRecord, Diagnostic>>;
}

/// How one watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailEnd {
    /// The node ended the stream: the run's writer is gone and every record is indexed.
    Finished,
    /// Damage, a refused record, a run without a row, or the daemon stopping: tailing this run is over.
    Stopped,
    /// The stream or a write broke; the tail watches again after a pause.
    Broken { indexed_any: bool },
}

/// How a batch of records ended.
#[derive(Debug, Clone, PartialEq)]
enum BatchEnd {
    /// Full, or its time was up: more may follow.
    More,
    Ended,
    Broken,
    ShuttingDown,
    /// The node's report (sequence 0), the stream's last record. Boxed: a record is large and a report rare.
    Report(Box<EventRecord>),
}

#[derive(Debug, Clone, PartialEq)]
struct Batch {
    records: Vec<EventRecord>,
    end: BatchEnd,
}

/// What one watch of one run knows.
struct RunTail<'a> {
    server: &'a Server,
    run_id: &'a str,
    node_id: String,
    /// The run's ack: every record up to it is indexed.
    acked_sequence: u64,
    /// The last record received; for a remote run, the last one in the mirror.
    last_sequence: u64,
    /// The daemon mirror, for a remote run.
    mirror: Option<EventLogWriter>,
    indexed_any: bool,
    shutdown: watch::Receiver<bool>,
}

/// A run's place in `Server::tailed_run_ids` while its tail runs. Dropping it, however the tail ends (a panic too),
/// frees the run for a new tail.
struct TailClaim {
    server: Arc<Server>,
    run_id: String,
}

impl Drop for TailClaim {
    fn drop(&mut self) {
        tailed_run_ids(&self.server).remove(&self.run_id);
    }
}

/// Starts a tail for every open run on `node` that has none yet (after a connect or a reconnect) and returns each
/// new tail's run id and task. This is the one place that tails a node's open runs; the node links settle each
/// run once its task ends (nodes/reconcile.rs).
pub fn start_run_tails(server: &Arc<Server>, node: &Arc<dyn RunEventSource>) -> Vec<(String, JoinHandle<()>)> {
    let run_ids = match open_run_ids(server, node.node_id()) {
        Ok(run_ids) => run_ids,
        Err(error) => {
            log_warn(
                "the open runs can't be listed; no run is tailed",
                &[("node_id", node.node_id()), ("code", error.code())],
            );
            return Vec::new();
        }
    };
    let mut tails = Vec::new();
    for run_id in run_ids {
        if let Some(task) = spawn_run_tail(server, node, &run_id) {
            tails.push((run_id, task));
        }
    }
    tails
}

/// Starts one run's tail on a task of its own and returns the task, or None when the run already has a tail. The
/// run is claimed before this returns, so a call right after it already sees the run as tailed (the node links
/// settle a run's end only once its tail is over, nodes/reconcile.rs). The tail runs until the node ends the run's
/// stream, its log is found damaged, or the daemon stops; a broken stream is watched again after a pause that grows
/// while nothing gets indexed.
pub fn spawn_run_tail(server: &Arc<Server>, node: &Arc<dyn RunEventSource>, run_id: &str) -> Option<JoinHandle<()>> {
    let claim = claim_run(server, run_id)?;
    Some(tokio::spawn(tail_claimed_run(claim, node.clone())))
}

/// The run's claim, or None when another tail holds it.
fn claim_run(server: &Arc<Server>, run_id: &str) -> Option<TailClaim> {
    if !tailed_run_ids(server).insert(run_id.to_string()) {
        return None;
    }
    Some(TailClaim {
        server: server.clone(),
        run_id: run_id.to_string(),
    })
}

fn tailed_run_ids(server: &Server) -> MutexGuard<'_, HashSet<String>> {
    // A panic while the lock was held leaves a plain set that is still whole, so the poison flag is ignored.
    server.tailed_run_ids.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The tail's loop: watches, and after a broken stream waits and watches again. The claim is released on return.
async fn tail_claimed_run(claim: TailClaim, node: Arc<dyn RunEventSource>) {
    let server = &claim.server;
    let run_id = &claim.run_id;
    let mut backoff_seconds = NODE_RECONNECT_FIRST_BACKOFF_SECONDS;
    loop {
        match tail_once(server, node.as_ref(), run_id).await {
            TailEnd::Finished | TailEnd::Stopped => return,
            TailEnd::Broken { indexed_any: true } => backoff_seconds = NODE_RECONNECT_FIRST_BACKOFF_SECONDS,
            TailEnd::Broken { indexed_any: false } => {}
        }
        let mut shutdown = server.shutdown.subscribe();
        if !pause_unless_stopping(&mut shutdown, Duration::from_secs(backoff_seconds)).await {
            return;
        }
        backoff_seconds = (backoff_seconds * 2).min(NODE_RECONNECT_MAX_BACKOFF_SECONDS);
    }
}

/// One watch: the ack, the mirror's catch-up for a remote run, then batches until the stream ends.
async fn tail_once(server: &Server, node: &dyn RunEventSource, run_id: &str) -> TailEnd {
    let acked_sequence = match read_acked_sequence(server, run_id) {
        Ok(Some(acked_sequence)) => acked_sequence,
        Ok(None) => {
            log_warn("a run without a runs row can't be tailed", &[("run_id", run_id)]);
            return TailEnd::Stopped;
        }
        Err(error) => {
            log_warn(
                "a run's ack can't be read",
                &[("run_id", run_id), ("code", error.code())],
            );
            return TailEnd::Broken { indexed_any: false };
        }
    };
    let mut tail = RunTail {
        server,
        run_id,
        node_id: node.node_id().to_string(),
        acked_sequence,
        last_sequence: acked_sequence,
        mirror: None,
        indexed_any: false,
        shutdown: server.shutdown.subscribe(),
    };
    if node.is_remote()
        && let Err(end) = catch_up_from_mirror(&mut tail).await
    {
        return end;
    }
    let mut records = node.watch_run_events(run_id, tail.last_sequence);
    loop {
        let batch = collect_batch(&mut records, &mut tail.shutdown).await;
        if let Some(end) = handle_records(&mut tail, batch.records).await {
            return end;
        }
        match batch.end {
            BatchEnd::More => {}
            BatchEnd::Ended => return TailEnd::Finished,
            BatchEnd::ShuttingDown => return TailEnd::Stopped,
            BatchEnd::Broken => {
                return TailEnd::Broken {
                    indexed_any: tail.indexed_any,
                };
            }
            BatchEnd::Report(report) => {
                publish_node_report(&tail, &report);
                return TailEnd::Stopped;
            }
        }
    }
}

/// Waits for the first record, then takes more until the batch is full or `INDEX_BATCH_MS` has passed.
async fn collect_batch(
    records: &mut mpsc::Receiver<Result<EventRecord, Diagnostic>>,
    shutdown: &mut watch::Receiver<bool>,
) -> Batch {
    let first = tokio::select! {
        item = records.recv() => item,
        () = wait_until_stopping(shutdown) => return batch_of(Vec::new(), BatchEnd::ShuttingDown),
    };
    let mut batch = Vec::new();
    if let Some(end) = add_to_batch(&mut batch, first) {
        return batch_of(batch, end);
    }
    let deadline = Instant::now() + Duration::from_millis(INDEX_BATCH_MS);
    while batch.len() < INDEX_BATCH_RECORDS {
        let Ok(item) = timeout_at(deadline, records.recv()).await else {
            break;
        };
        if let Some(end) = add_to_batch(&mut batch, item) {
            return batch_of(batch, end);
        }
    }
    batch_of(batch, BatchEnd::More)
}

/// Adds one item of the stream to the batch, or says how the batch ends with it.
fn add_to_batch(batch: &mut Vec<EventRecord>, item: Option<Result<EventRecord, Diagnostic>>) -> Option<BatchEnd> {
    match item {
        None => Some(BatchEnd::Ended),
        Some(Err(problem)) => {
            log_warn(
                "a run's event stream broke; it is watched again",
                &[("code", &problem.code), ("message", &problem.message)],
            );
            Some(BatchEnd::Broken)
        }
        Some(Ok(record)) if record.sequence == REPORT_SEQUENCE => Some(BatchEnd::Report(Box::new(record))),
        Some(Ok(record)) => {
            batch.push(record);
            None
        }
    }
}

fn batch_of(records: Vec<EventRecord>, end: BatchEnd) -> Batch {
    Batch { records, end }
}

/// Mirrors (for a remote run), indexes and publishes one batch. `None` to go on; otherwise how the watch ends.
async fn handle_records(tail: &mut RunTail<'_>, received: Vec<EventRecord>) -> Option<TailEnd> {
    let (records, out_of_sequence) = new_records_in_sequence(tail.last_sequence, received);
    let broken = TailEnd::Broken {
        indexed_any: tail.indexed_any,
    };
    if records.is_empty() && out_of_sequence {
        return Some(broken);
    }
    if records.is_empty() {
        return None;
    }
    if let Some(mirror) = tail.mirror.as_mut()
        && let Err(error) = append_to_mirror(mirror, &records)
    {
        log_warn(
            "a remote run's mirror can't be written",
            &[("run_id", tail.run_id), ("code", error.code())],
        );
        return Some(broken);
    }
    tail.last_sequence = last_sequence_of(&records, tail.last_sequence);
    if let Some(end) = index_and_publish(tail, &records).await {
        return Some(end);
    }
    if out_of_sequence {
        log_warn(
            "a run's records arrived out of sequence; it is watched again",
            &[("run_id", tail.run_id)],
        );
        return Some(TailEnd::Broken { indexed_any: true });
    }
    None
}

/// The records after `last_sequence` that follow it one by one. A record that skips ahead (a sequence gap on the
/// wire) and everything after it are left out, and the bool says so; a record already received is dropped quietly.
fn new_records_in_sequence(last_sequence: u64, received: Vec<EventRecord>) -> (Vec<EventRecord>, bool) {
    let mut expected = last_sequence + 1;
    let mut records = Vec::new();
    for record in received {
        if record.sequence < expected {
            continue;
        }
        if record.sequence > expected {
            return (records, true);
        }
        expected += 1;
        records.push(record);
    }
    (records, false)
}

fn last_sequence_of(records: &[EventRecord], otherwise: u64) -> u64 {
    match records.last() {
        Some(record) => record.sequence,
        None => otherwise,
    }
}

/// Appends the batch's records to the mirror, then one fsync: the mirror is ahead of the index, never behind it. The
/// frames are encoded again from the decoded records, so they hold the same records as the node's log but not
/// always the same bytes: prost drops a field this build doesn't know.
fn append_to_mirror(mirror: &mut EventLogWriter, records: &[EventRecord]) -> Result<(), StoreError> {
    for record in records {
        let Some(frame) = encode_frame(record) else {
            return Err(StoreError::DatabaseWriteFailed {
                reason: "a record too large for a frame".to_string(),
            });
        };
        append_raw_frame(mirror, &frame)?;
    }
    sync_event_log(mirror)
}

/// Indexes the records with the run's ack in one transaction, then publishes those the index didn't have.
async fn index_and_publish(tail: &mut RunTail<'_>, records: &[EventRecord]) -> Option<TailEnd> {
    let mut disk_full_reported = false;
    loop {
        let write = IndexWrite::ApplyRunEvents {
            run_id: tail.run_id.to_string(),
            records: records.to_vec(),
        };
        match write_index(&tail.server.store.index, write).await {
            Ok(()) => {
                publish_indexed(tail, records);
                return None;
            }
            Err(StoreError::DiskFull { reason }) => {
                if !disk_full_reported {
                    report_problem(tail, &diagnostic(CODE_DISK_FULL, &[("reason", &reason)]));
                    disk_full_reported = true;
                }
                let retry = Duration::from_secs(INDEX_DISK_FULL_RETRY_SECONDS);
                if !pause_unless_stopping(&mut tail.shutdown, retry).await {
                    return Some(TailEnd::Stopped);
                }
            }
            Err(StoreError::RunLogRecordRefused { sequence, reason, .. }) => {
                index_before_refused(tail, records, sequence).await;
                let sequence_text = sequence.to_string();
                let values = [
                    ("run_id", tail.run_id),
                    ("sequence", sequence_text.as_str()),
                    ("reason", &reason),
                ];
                report_problem(tail, &diagnostic(CODE_RUN_LOG_RECORD_REFUSED, &values));
                return Some(TailEnd::Stopped);
            }
            Err(error) => {
                log_warn(
                    "a run's records can't be indexed",
                    &[("run_id", tail.run_id), ("code", error.code())],
                );
                return Some(TailEnd::Broken {
                    indexed_any: tail.indexed_any,
                });
            }
        }
    }
}

/// After a refusal: the records before the refused one are indexed alone, so the ack stops right before it.
async fn index_before_refused(tail: &mut RunTail<'_>, records: &[EventRecord], refused_sequence: u64) {
    let before: Vec<EventRecord> = records
        .iter()
        .filter(|record| record.sequence < refused_sequence)
        .cloned()
        .collect();
    if before.is_empty() {
        return;
    }
    let write = IndexWrite::ApplyRunEvents {
        run_id: tail.run_id.to_string(),
        records: before.clone(),
    };
    match write_index(&tail.server.store.index, write).await {
        Ok(()) => publish_indexed(tail, &before),
        Err(error) => log_warn(
            "the records before a refused one can't be indexed",
            &[("run_id", tail.run_id), ("code", error.code())],
        ),
    }
}

/// Publishes the newly indexed records and moves the tail's copy of the ack. Records the index already had (a remote
/// mirror rebuilt behind its ack) were published when they were first indexed.
fn publish_indexed(tail: &mut RunTail<'_>, records: &[EventRecord]) {
    for record in records {
        if record.sequence > tail.acked_sequence {
            let source = record_source(record, &tail.node_id);
            publish(&tail.server.bus, bus_event_from_record(record, &source));
        }
    }
    tail.acked_sequence = tail.acked_sequence.max(last_sequence_of(records, 0));
    tail.indexed_any = true;
}

/// `job:<job_id>` for a record a job wrote, else `node:<node_id>` (contracts 7.2).
fn record_source(record: &EventRecord, node_id: &str) -> String {
    if record.job_id.is_empty() {
        return format!("{NODE_SOURCE_PREFIX}{node_id}");
    }
    format!("{JOB_SOURCE_PREFIX}{}", record.job_id)
}

/// A remote run after a restart: index the mirror's records after the ack, then open it to append. Records the
/// node sends from then on follow the mirror's last one.
async fn catch_up_from_mirror(tail: &mut RunTail<'_>) -> Result<(), TailEnd> {
    let path = run_events_log_path(&tail.server.paths, tail.run_id);
    tail.last_sequence = 0;
    if !path.exists() {
        tail.mirror = Some(create_mirror(tail)?);
        return Ok(());
    }
    let mut reader = open_event_log(&path).map_err(|error| mirror_unusable(tail, &error))?;
    let mut batch = Vec::new();
    loop {
        match read_next_frame(&mut reader) {
            Ok(FrameRead::Record(record)) => {
                tail.last_sequence = record.sequence;
                batch.push(record);
            }
            Ok(FrameRead::EndOfFile) | Ok(FrameRead::Incomplete { .. }) => break,
            Ok(FrameRead::Corrupt { offset }) => {
                let path_text = path.display().to_string();
                let offset_text = offset.to_string();
                let damage = [("path", path_text.as_str()), ("offset", offset_text.as_str())];
                report_problem(tail, &diagnostic(CODE_EVENT_LOG_CORRUPT, &damage));
                return Err(TailEnd::Stopped);
            }
            Err(error) => return Err(mirror_unusable(tail, &error)),
        }
        if batch.len() == INDEX_BATCH_RECORDS {
            index_mirror_batch(tail, std::mem::take(&mut batch)).await?;
        }
    }
    index_mirror_batch(tail, batch).await?;
    let writer = open_event_log_for_append(&path).map_err(|error| mirror_unusable(tail, &error))?;
    tail.mirror = Some(writer);
    Ok(())
}

/// Mirror records at or below the ack are already indexed; the rest go through the normal path.
async fn index_mirror_batch(tail: &mut RunTail<'_>, batch: Vec<EventRecord>) -> Result<(), TailEnd> {
    let acked_sequence = tail.acked_sequence;
    let records: Vec<EventRecord> = batch
        .into_iter()
        .filter(|record| record.sequence > acked_sequence)
        .collect();
    if records.is_empty() {
        return Ok(());
    }
    match index_and_publish(tail, &records).await {
        Some(end) => Err(end),
        None => Ok(()),
    }
}

fn create_mirror(tail: &RunTail<'_>) -> Result<EventLogWriter, TailEnd> {
    let folder = run_dir(&tail.server.paths, tail.run_id);
    if let Err(error) = std::fs::create_dir_all(&folder) {
        let failure = StoreError::EventLogWriteFailed {
            path: folder,
            reason: error.to_string(),
        };
        return Err(mirror_unusable(tail, &failure));
    }
    let path = run_events_log_path(&tail.server.paths, tail.run_id);
    create_event_log(&path).map_err(|error| mirror_unusable(tail, &error))
}

/// A mirror that can't be read or written: published and logged, and the tail stops, since a mirror out of step
/// would hide records from the next restart.
fn mirror_unusable(tail: &RunTail<'_>, error: &StoreError) -> TailEnd {
    report_problem(tail, &store_error_diagnostic(error));
    TailEnd::Stopped
}

/// The node's report (sequence 0) as it is: the UI shows it on the run.
fn publish_node_report(tail: &RunTail<'_>, report: &EventRecord) {
    let code = report_code(report);
    log_warn(
        "a run's log is damaged; its index stops here",
        &[("run_id", tail.run_id), ("code", &code)],
    );
    let source = format!("{NODE_SOURCE_PREFIX}{}", tail.node_id);
    publish(&tail.server.bus, bus_event_from_record(report, &source));
}

fn report_code(report: &EventRecord) -> String {
    let payload = report.payload.as_ref().and_then(|payload| payload.payload.as_ref());
    match payload {
        Some(Payload::DiagnosticReported(reported)) => match &reported.diagnostic {
            Some(diagnostic) => diagnostic.code.clone(),
            None => String::new(),
        },
        _ => String::new(),
    }
}

/// Logs a problem with the run's indexing and publishes it as `diagnostic.reported` about the run.
fn report_problem(tail: &RunTail<'_>, problem: &Diagnostic) {
    log_warn(
        "a run's indexing hit a problem",
        &[
            ("run_id", tail.run_id),
            ("code", &problem.code),
            ("message", &problem.message),
        ],
    );
    let reported = DiagnosticReported {
        diagnostic: Some(diagnostic_to_proto(problem)),
        subject_id: tail.run_id.to_string(),
    };
    let event = BusEvent {
        bus_sequence: 0,
        event_type: DIAGNOSTIC_REPORTED_EVENT.to_string(),
        event_class: EventClass::Durable as i32,
        subject_id: tail.run_id.to_string(),
        source: DAEMON_SOURCE.to_string(),
        emitted_unix_ms: now_unix_ms(),
        payload: Some(EventPayload {
            payload: Some(Payload::DiagnosticReported(reported)),
        }),
    };
    publish(&tail.server.bus, event);
}

/// Waits `pause`, or less if the daemon starts stopping. True when the pause ran out and the daemon still runs.
async fn pause_unless_stopping(shutdown: &mut watch::Receiver<bool>, pause: Duration) -> bool {
    if *shutdown.borrow() {
        return false;
    }
    tokio::select! {
        _ = tokio::time::sleep(pause) => true,
        () = wait_until_stopping(shutdown) => false,
    }
}

fn read_acked_sequence(server: &Server, run_id: &str) -> Result<Option<u64>, StoreError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    let run = get_run(&reader, run_id)?;
    Ok(run.map(|row| row.acked_sequence))
}

/// The runs on `node_id` whose state doesn't end the run: their logs may still grow.
pub fn open_run_ids(server: &Server, node_id: &str) -> Result<Vec<String>, StoreError> {
    let reader = open_index_reader(&server.store.index_readers)?;
    let mut run_ids = Vec::new();
    for run in list_runs(&reader, &RunFilter::default())? {
        if run.node_id == node_id && run_is_open(&run.state) {
            run_ids.push(run.run_id);
        }
    }
    Ok(run_ids)
}

/// A training-style run is open until completed, failed or cancelled; an eval run until its terminal state.
fn run_is_open(state: &str) -> bool {
    if let Some(run_state) = parse_run_state(state) {
        return !RUN_STATES_THAT_END_A_RUN.contains(&run_state);
    }
    if let Some(eval_state) = parse_eval_run_state(state) {
        return !is_terminal_eval_run_state(eval_state);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sequence: u64) -> EventRecord {
        EventRecord {
            sequence,
            ..EventRecord::default()
        }
    }

    fn sequences(records: &[EventRecord]) -> Vec<u64> {
        records.iter().map(|record| record.sequence).collect()
    }

    #[test]
    fn records_already_received_are_dropped_and_a_gap_stops_the_batch() {
        let (records, gap) = new_records_in_sequence(2, vec![record(1), record(2), record(3), record(4)]);
        assert_eq!(sequences(&records), [3, 4]);
        assert!(!gap);
        let (records, gap) = new_records_in_sequence(2, vec![record(3), record(5), record(6)]);
        assert_eq!(sequences(&records), [3]);
        assert!(gap);
        let (records, gap) = new_records_in_sequence(0, Vec::new());
        assert!(records.is_empty());
        assert!(!gap);
    }

    #[test]
    fn a_batch_ends_at_the_report_the_stream_end_or_a_break() {
        let mut batch = Vec::new();
        assert_eq!(add_to_batch(&mut batch, Some(Ok(record(1)))), None);
        assert_eq!(
            add_to_batch(&mut batch, Some(Ok(record(0)))),
            Some(BatchEnd::Report(Box::new(record(0))))
        );
        assert_eq!(add_to_batch(&mut batch, None), Some(BatchEnd::Ended));
        let broken = Some(Err(diagnostic("RRS-N2008", &[("timeout_ms", "1")])));
        assert_eq!(add_to_batch(&mut batch, broken), Some(BatchEnd::Broken));
        assert_eq!(sequences(&batch), [1]);
    }

    #[test]
    fn records_name_the_job_that_wrote_them_or_the_node() {
        let mut from_job = record(1);
        from_job.job_id = "job_a".to_string();
        assert_eq!(record_source(&from_job, "node_local"), "job:job_a");
        assert_eq!(record_source(&record(1), "node_local"), "node:node_local");
    }

    #[test]
    fn only_runs_that_may_still_grow_are_open() {
        for open in [
            "created",
            "queued",
            "preparing",
            "running",
            "paused",
            "checkpointing",
            "preempted",
        ] {
            assert!(run_is_open(open), "{open}");
        }
        for ended in ["completed", "failed", "cancelled"] {
            assert!(!run_is_open(ended), "{ended}");
        }
    }
}
