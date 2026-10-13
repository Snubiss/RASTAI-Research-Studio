//! Run-log indexing through a running daemon (layout 13.1 and 8.6, flows/platform.md 6.1 and 6.2): a fake node
//! streams a run's records and the ack commits with the rows; a restart re-indexes nothing twice; a remote run's
//! mirror is written before the index and a restart re-indexes from it once; a refused record and the node's report
//! stop the tail; a broken stream is watched again after the ack; a reconnect never starts a second tail of a run;
//! and the daemon log's records after the cursor are indexed before the listener opens.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rastai_core::paths::{DataPaths, daemon_events_log_path, run_events_log_path};
use rastai_core::principal::owner_principal;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, EventClass, JobState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    BusEvent, CheckpointSaved, CommandFinished, DiagnosticReported, EventPayload, EventRecord, JobStateChanged,
    RunMetrics, RunStateChanged,
};
use rastai_server::daemon::RunningDaemon;
use rastai_server::events::bus::subscribe;
use rastai_server::indexing::run_tail::{RunEventSource, spawn_run_tail, start_run_tails};
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
use rastai_store::event_log::writer::{SyncPolicy, append_event, create_event_log, open_event_log_for_append};
use rastai_store::index::journal::{JournalStartedRow, get_journal};
use rastai_store::index::log_cursors::get_log_cursor;
use rastai_store::index::reads::{list_table_columns, list_table_rows, open_index_reader};
use rastai_store::index::runs::{RunRow, get_run};
use rastai_store::index::writes::IndexWrite;
use rastai_store::writer_thread::write_index;
use tokio::sync::{broadcast, mpsc};

use common::{
    execute_request, ok_result, remove_test_folder, run_command, start_test_daemon, stop_test_daemon, test_paths,
};

const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2N";
const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2N";
const CHECKPOINT_ID: &str = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2N";
const EMBEDDED_NODE_ID: &str = "node_local";
const REMOTE_NODE_ID: &str = "node_01JA2B3C4D5E6F7G8H9J0K1M2N";
const CONTENT_HASH: &str = "blake3:0101010101010101010101010101010101010101010101010101010101010101";
const RECORD_TIME_UNIX_MS: i64 = 1_791_700_000_000;
/// The table the embedded node's heartbeats keep writing (every_stored_row), and the event they publish (published).
const NODES_TABLE: &str = "nodes";
const NODE_HEARTBEAT_EVENT: &str = "node.heartbeat";
/// How long a test waits for a tail running on its own task: up to 200 steps of 25 ms.
const WAIT_ATTEMPTS: usize = 200;
const WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(25);

/// A node that streams the records it was given. `ignore_after` makes it re-send everything whatever the daemon
/// asks for, like a node that lost its place; `break_after` ends the first watch with a broken stream after that
/// many records; `hold_open` keeps every stream open, as for a run still writing, until `open_streams` is cleared.
struct FakeNode {
    node_id: String,
    remote: bool,
    records: Vec<EventRecord>,
    ignore_after: bool,
    break_after: Option<usize>,
    hold_open: bool,
    open_streams: Mutex<Vec<mpsc::Sender<Result<EventRecord, Diagnostic>>>>,
    watches: Mutex<Vec<u64>>,
}

impl RunEventSource for FakeNode {
    fn node_id(&self) -> &str {
        &self.node_id
    }

    fn is_remote(&self) -> bool {
        self.remote
    }

    fn watch_run_events(&self, _run_id: &str, after_sequence: u64) -> mpsc::Receiver<Result<EventRecord, Diagnostic>> {
        let mut watches = self.watches.lock().unwrap();
        let first_watch = watches.is_empty();
        watches.push(after_sequence);
        let (sender, receiver) = mpsc::channel(self.records.len() + 1);
        let mut sent = 0;
        for record in &self.records {
            // The report (sequence 0) is always the stream's last record.
            let wanted = self.ignore_after || record.sequence > after_sequence || record.sequence == 0;
            if !wanted {
                continue;
            }
            if first_watch && self.break_after == Some(sent) {
                sender
                    .try_send(Err(diagnostic("RRS-N2008", &[("timeout_ms", "1")])))
                    .unwrap();
                return receiver;
            }
            sender.try_send(Ok(record.clone())).unwrap();
            sent += 1;
        }
        // Dropping the sender ends the stream: the run's writer is gone.
        if self.hold_open {
            self.open_streams.lock().unwrap().push(sender);
        }
        receiver
    }
}

fn fake_node(node_id: &str, remote: bool, records: Vec<EventRecord>) -> FakeNode {
    FakeNode {
        node_id: node_id.to_string(),
        remote,
        records,
        ignore_after: false,
        break_after: None,
        hold_open: false,
        open_streams: Mutex::new(Vec::new()),
        watches: Mutex::new(Vec::new()),
    }
}

fn record(sequence: u64, event_type: &str, payload: Payload) -> EventRecord {
    EventRecord {
        format_version: 1,
        sequence,
        run_id: RUN_ID.to_string(),
        job_id: JOB_ID.to_string(),
        emitted_unix_ms: RECORD_TIME_UNIX_MS + i64::try_from(sequence).unwrap(),
        event_type: event_type.to_string(),
        event_class: EventClass::Durable as i32,
        payload: Some(EventPayload { payload: Some(payload) }),
        blobs: Vec::new(),
    }
}

fn run_state(sequence: u64, state: RunState) -> EventRecord {
    let change = RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(state as i32)),
    };
    record(sequence, "run.state_changed", Payload::RunStateChanged(change))
}

fn metrics(sequence: u64, step: u64) -> EventRecord {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), 1.0 / step as f64);
    record(
        sequence,
        "run.metrics",
        Payload::RunMetrics(RunMetrics { step, values }),
    )
}

fn checkpoint(sequence: u64) -> EventRecord {
    let saved = CheckpointSaved {
        checkpoint_id: CHECKPOINT_ID.to_string(),
        step: 3,
        folder: "checkpoints/step-00000003".to_string(),
        is_best: true,
        ..CheckpointSaved::default()
    };
    record(sequence, "checkpoint.saved", Payload::CheckpointSaved(saved))
}

/// A whole short run: it starts, logs three steps, saves a checkpoint and completes.
fn whole_run() -> Vec<EventRecord> {
    vec![
        run_state(1, RunState::Running),
        metrics(2, 1),
        metrics(3, 2),
        metrics(4, 3),
        checkpoint(5),
        run_state(6, RunState::Completed),
    ]
}

/// A record only the daemon's log may carry (contracts 7.1): a run's log that holds one is refused with RRS-S2004.
fn job_state_record(sequence: u64) -> EventRecord {
    let change = JobStateChanged {
        job_id: JOB_ID.to_string(),
        from_state: JobState::Running as i32,
        to_state: JobState::Succeeded as i32,
        ..JobStateChanged::default()
    };
    record(sequence, "job.state_changed", Payload::JobStateChanged(change))
}

/// The node's report about a damaged log: sequence 0, never a log record (contracts 4.4).
fn node_report(code: &str) -> EventRecord {
    let reported = DiagnosticReported {
        diagnostic: Some(ProtoDiagnostic {
            code: code.to_string(),
            ..ProtoDiagnostic::default()
        }),
        subject_id: RUN_ID.to_string(),
    };
    record(0, "diagnostic.reported", Payload::DiagnosticReported(reported))
}

async fn add_run(running: &RunningDaemon, node_id: &str) {
    let row = RunRow {
        run_id: RUN_ID.to_string(),
        kind: "train".to_string(),
        state: "queued".to_string(),
        project_id: String::new(),
        node_id: node_id.to_string(),
        parent_run_id: String::new(),
        record_hash: CONTENT_HASH.to_string(),
        acked_sequence: 0,
        created_unix_ms: RECORD_TIME_UNIX_MS,
        finished_unix_ms: None,
        pinned: false,
        archive_state: "local".to_string(),
    };
    write_index(&running.server.store.index, IndexWrite::UpsertRun(row))
        .await
        .unwrap();
}

/// Tails the run until its tail ends.
async fn tail(running: &RunningDaemon, node: &Arc<FakeNode>) {
    let source: Arc<dyn RunEventSource> = node.clone();
    let task = spawn_run_tail(&running.server, &source, RUN_ID).unwrap();
    task.await.unwrap();
}

fn run_row(running: &RunningDaemon) -> RunRow {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_run(&reader, RUN_ID).unwrap().unwrap()
}

fn table_rows(running: &RunningDaemon, table_name: &str) -> Vec<Vec<String>> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    list_table_rows(&reader, table_name).unwrap()
}

/// Every row of every table of rastai.db, as text: what "nothing twice" compares. The `nodes` table is left out: the
/// embedded node's heartbeats move its row's `last_heartbeat_unix_ms` while the test runs.
fn every_stored_row(running: &RunningDaemon) -> Vec<String> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    let mut rows = Vec::new();
    for table in list_table_columns(&reader).unwrap() {
        if table.table_name == NODES_TABLE {
            continue;
        }
        for row in list_table_rows(&reader, &table.table_name).unwrap() {
            rows.push(format!("{}\t{}", table.table_name, row.join("\t")));
        }
    }
    rows
}

/// The events published so far, without waiting, but the embedded node's telemetry, which its heartbeats publish
/// whenever they come.
fn published(live: &mut broadcast::Receiver<BusEvent>) -> Vec<BusEvent> {
    let mut events = Vec::new();
    while let Ok(event) = live.try_recv() {
        if event.event_type != NODE_HEARTBEAT_EVENT {
            events.push(event);
        }
    }
    events
}

/// The sequences of the run's own records among the events, in order.
fn record_sequences(events: &[BusEvent], records: &[EventRecord]) -> Vec<u64> {
    let mut sequences = Vec::new();
    for event in events {
        for record in records {
            if event.payload == record.payload && event.event_type == record.event_type {
                sequences.push(record.sequence);
            }
        }
    }
    sequences
}

/// The sequences of the whole records in a log file.
fn log_sequences(path: &std::path::Path) -> Vec<u64> {
    let mut reader = open_event_log(path).unwrap();
    let mut sequences = Vec::new();
    while let Ok(FrameRead::Record(record)) = read_next_frame(&mut reader) {
        sequences.push(record.sequence);
    }
    sequences
}

fn reported_codes(events: &[BusEvent]) -> Vec<String> {
    let mut codes = Vec::new();
    for event in events {
        let payload = event.payload.as_ref().and_then(|payload| payload.payload.as_ref());
        if let Some(Payload::DiagnosticReported(reported)) = payload {
            codes.push(reported.diagnostic.as_ref().unwrap().code.clone());
        }
    }
    codes
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fake_node_streams_records_and_the_ack_commits_with_the_rows() {
    let paths = test_paths("tail-embedded");
    let running = start_test_daemon(&paths).await;
    add_run(&running, EMBEDDED_NODE_ID).await;
    let mut live = subscribe(&running.server.bus);
    let node = Arc::new(fake_node(EMBEDDED_NODE_ID, false, whole_run()));

    tail(&running, &node).await;

    assert_eq!(*node.watches.lock().unwrap(), [0]);
    let run = run_row(&running);
    assert_eq!(run.acked_sequence, 6);
    assert_eq!(run.state, "completed");
    assert_eq!(table_rows(&running, "run_metrics").len(), 3);
    assert_eq!(table_rows(&running, "checkpoints").len(), 1);
    let events = published(&mut live);
    assert_eq!(record_sequences(&events, &whole_run()), [1, 2, 3, 4, 5, 6]);
    for event in &events {
        assert_eq!(event.subject_id, RUN_ID);
        assert_eq!(event.source, format!("job:{JOB_ID}"));
    }
    // The embedded node shares the run's own file: the daemon writes no mirror of it.
    assert!(!run_events_log_path(&paths, RUN_ID).exists());
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_re_indexes_nothing_twice() {
    let paths = test_paths("tail-restart");
    let running = start_test_daemon(&paths).await;
    add_run(&running, EMBEDDED_NODE_ID).await;
    // The run stops before its end, as at a daemon crash: four records indexed, the run still open.
    let first = Arc::new(fake_node(EMBEDDED_NODE_ID, false, whole_run()[..4].to_vec()));
    tail(&running, &first).await;
    assert_eq!(run_row(&running).acked_sequence, 4);
    stop_test_daemon(running).await;

    let restarted = start_test_daemon(&paths).await;
    let rows_before = every_stored_row(&restarted);
    let mut live = subscribe(&restarted.server.bus);
    // A node that lost its place re-sends every record from the start; the tail asks after the ack all the same.
    let mut resending = fake_node(EMBEDDED_NODE_ID, false, whole_run()[..4].to_vec());
    resending.ignore_after = true;
    let resending = Arc::new(resending);
    tail(&restarted, &resending).await;
    assert_eq!(*resending.watches.lock().unwrap(), [4]);
    assert_eq!(every_stored_row(&restarted), rows_before);
    assert!(published(&mut live).is_empty());

    // Then the rest of the run arrives once, through start_run_tails, which tails the open run only.
    let rest = Arc::new(fake_node(EMBEDDED_NODE_ID, false, whole_run()));
    let source: Arc<dyn RunEventSource> = rest.clone();
    let tails = start_run_tails(&restarted.server, &source);
    assert_eq!(tails.len(), 1);
    assert_eq!(tails[0].0, RUN_ID);
    for (_run_id, task) in tails {
        task.await.unwrap();
    }
    assert_eq!(*rest.watches.lock().unwrap(), [4]);
    assert_eq!(run_row(&restarted).acked_sequence, 6);
    assert_eq!(table_rows(&restarted, "run_metrics").len(), 3);
    assert_eq!(record_sequences(&published(&mut live), &whole_run()), [5, 6]);
    // The run is completed now: there is nothing left to tail.
    assert!(start_run_tails(&restarted.server, &source).is_empty());
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_run_is_mirrored_before_it_is_indexed() {
    let paths = test_paths("tail-mirror-first");
    let running = start_test_daemon(&paths).await;
    add_run(&running, REMOTE_NODE_ID).await;
    let mut live = subscribe(&running.server.bus);
    // The fourth record is one a run's log may not carry: the index refuses it, but the mirror already has it.
    let mut records = whole_run()[..3].to_vec();
    records.push(job_state_record(4));
    records.push(metrics(5, 3));
    let node = Arc::new(fake_node(REMOTE_NODE_ID, true, records.clone()));

    tail(&running, &node).await;

    assert_eq!(log_sequences(&run_events_log_path(&paths, RUN_ID)), [1, 2, 3, 4, 5]);
    assert_eq!(run_row(&running).acked_sequence, 3);
    assert_eq!(table_rows(&running, "run_metrics").len(), 2);
    let events = published(&mut live);
    assert_eq!(record_sequences(&events, &records), [1, 2, 3]);
    assert_eq!(reported_codes(&events), ["RRS-S2004"]);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_re_indexes_a_remote_run_from_its_mirror_once() {
    let paths = test_paths("tail-mirror-restart");
    let running = start_test_daemon(&paths).await;
    add_run(&running, REMOTE_NODE_ID).await;
    // The daemon died after mirroring three records and before their index committed.
    std::fs::create_dir_all(run_events_log_path(&paths, RUN_ID).parent().unwrap()).unwrap();
    let mut mirror = create_event_log(&run_events_log_path(&paths, RUN_ID)).unwrap();
    for mut mirrored in whole_run()[..3].to_vec() {
        append_event(&mut mirror, &mut mirrored, SyncPolicy::Now).unwrap();
    }
    drop(mirror);
    let mut live = subscribe(&running.server.bus);
    let node = Arc::new(fake_node(REMOTE_NODE_ID, true, whole_run()));

    tail(&running, &node).await;

    // The mirror's records were indexed from the mirror, and the node was asked only for what came after them.
    assert_eq!(*node.watches.lock().unwrap(), [3]);
    assert_eq!(run_row(&running).acked_sequence, 6);
    assert_eq!(log_sequences(&run_events_log_path(&paths, RUN_ID)), [1, 2, 3, 4, 5, 6]);
    assert_eq!(table_rows(&running, "run_metrics").len(), 3);
    assert_eq!(
        record_sequences(&published(&mut live), &whole_run()),
        [1, 2, 3, 4, 5, 6]
    );
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_node_report_is_published_and_never_mirrored_or_indexed() {
    let paths = test_paths("tail-report");
    let running = start_test_daemon(&paths).await;
    add_run(&running, REMOTE_NODE_ID).await;
    let mut live = subscribe(&running.server.bus);
    let mut records = whole_run()[..2].to_vec();
    records.push(node_report("RRS-S2002"));
    let node = Arc::new(fake_node(REMOTE_NODE_ID, true, records));

    tail(&running, &node).await;

    assert_eq!(run_row(&running).acked_sequence, 2);
    assert_eq!(log_sequences(&run_events_log_path(&paths, RUN_ID)), [1, 2]);
    let events = published(&mut live);
    assert_eq!(reported_codes(&events), ["RRS-S2002"]);
    let report = events.last().unwrap();
    assert_eq!(report.source, format!("node:{REMOTE_NODE_ID}"));
    assert_eq!(report.subject_id, RUN_ID);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_stream_is_watched_again_after_the_ack() {
    let paths = test_paths("tail-broken");
    let running = start_test_daemon(&paths).await;
    add_run(&running, EMBEDDED_NODE_ID).await;
    let mut node = fake_node(EMBEDDED_NODE_ID, false, whole_run());
    node.break_after = Some(2);
    let node = Arc::new(node);

    tail(&running, &node).await;

    assert_eq!(*node.watches.lock().unwrap(), [0, 2]);
    assert_eq!(run_row(&running).acked_sequence, 6);
    assert_eq!(table_rows(&running, "run_metrics").len(), 3);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_never_starts_a_second_tail_of_a_run() {
    let paths = test_paths("tail-one-per-run");
    let running = start_test_daemon(&paths).await;
    add_run(&running, EMBEDDED_NODE_ID).await;
    let mut live = subscribe(&running.server.bus);
    // The run is still writing: its stream stays open after its first three records.
    let mut node = fake_node(EMBEDDED_NODE_ID, false, whole_run()[..3].to_vec());
    node.hold_open = true;
    let node = Arc::new(node);
    let source: Arc<dyn RunEventSource> = node.clone();

    let first = start_run_tails(&running.server, &source);
    assert_eq!(first.len(), 1);
    // The node link reconnects and calls start_run_tails again while the first tail still watches: no second tail.
    assert!(start_run_tails(&running.server, &source).is_empty());
    assert!(spawn_run_tail(&running.server, &source, RUN_ID).is_none());
    wait_for_ack(&running, 3).await;
    assert_eq!(*node.watches.lock().unwrap(), [0]);
    assert_eq!(record_sequences(&published(&mut live), &whole_run()), [1, 2, 3]);

    // The node ends the stream; the tail ends and frees the run, so the next connect tails it again.
    node.open_streams.lock().unwrap().clear();
    for (_run_id, task) in first {
        task.await.unwrap();
    }
    let again = start_run_tails(&running.server, &source);
    assert_eq!(again.len(), 1);
    wait_for_watches(&node, 2).await;
    node.open_streams.lock().unwrap().clear();
    for (_run_id, task) in again {
        task.await.unwrap();
    }
    assert_eq!(*node.watches.lock().unwrap(), [0, 3]);
    assert_eq!(table_rows(&running, "run_metrics").len(), 2);
    stop_test_daemon(running).await;
    remove_test_folder(&paths);
}

/// Waits, a little at a time, until the run's ack reaches `acked_sequence`.
async fn wait_for_ack(running: &RunningDaemon, acked_sequence: u64) {
    for _ in 0..WAIT_ATTEMPTS {
        if run_row(running).acked_sequence >= acked_sequence {
            return;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("the run's ack never reached {acked_sequence}");
}

/// Waits until the node has been asked to watch `count` times.
async fn wait_for_watches(node: &FakeNode, count: usize) {
    for _ in 0..WAIT_ATTEMPTS {
        if node.watches.lock().unwrap().len() >= count {
            return;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("the node was never asked to watch {count} times");
}

/// A `command.finished` record for `journal_id`, as `record_daemon_event` writes it.
fn command_finished(journal_id: &str, ok: bool, code: &str) -> EventRecord {
    let finished = CommandFinished {
        journal_id: journal_id.to_string(),
        command_id: "token.create".to_string(),
        principal: "owner".to_string(),
        ok,
        code: code.to_string(),
    };
    EventRecord {
        format_version: 1,
        event_type: "command.finished".to_string(),
        event_class: EventClass::Durable as i32,
        emitted_unix_ms: RECORD_TIME_UNIX_MS,
        payload: Some(EventPayload {
            payload: Some(Payload::CommandFinished(finished)),
        }),
        ..EventRecord::default()
    }
}

fn daemon_cursor(running: &RunningDaemon) -> Option<u64> {
    let reader = open_index_reader(&running.server.store.index_readers).unwrap();
    get_log_cursor(&reader, "daemon:1").unwrap()
}

/// Appends records to the daemon log behind the daemon's back, as a crash between append and index leaves them.
fn append_to_daemon_log(paths: &DataPaths, records: Vec<EventRecord>) {
    let mut writer = open_event_log_for_append(&daemon_events_log_path(paths)).unwrap();
    for mut record in records {
        append_event(&mut writer, &mut record, SyncPolicy::Now).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_log_records_after_the_cursor_are_indexed_before_the_listener_opens() {
    let paths = test_paths("daemon-log-replay");
    let running = start_test_daemon(&paths).await;
    // A journaled command's command.finished is logged and indexed: the log's next record, and its cursor. The
    // records before it are the embedded node's connection (node.state_changed).
    let before = log_sequences(&daemon_events_log_path(&paths)).len() as u64;
    let request = execute_request("token.create", r#"{"name": "ci", "preset": "chat-only"}"#);
    let created = run_command(&running.server, &owner_principal(), request).await;
    ok_result(&created);
    let logged = log_sequences(&daemon_events_log_path(&paths));
    assert_eq!(logged.last(), Some(&(before + 1)));
    assert_eq!(daemon_cursor(&running), Some(before + 1));
    // A command that was running when the daemon died: started, never finished in rastai.db.
    let interrupted = JournalStartedRow {
        journal_id: "jrn_01JB2B3C4D5E6F7G8H9J0K1M2N".to_string(),
        command_id: "token.create".to_string(),
        principal: "owner".to_string(),
        arguments_json: "{}".to_string(),
        idempotency_key: String::new(),
        started_unix_ms: RECORD_TIME_UNIX_MS,
        arguments_hash: String::new(),
    };
    let write = IndexWrite::JournalStarted(interrupted.clone());
    write_index(&running.server.store.index, write).await.unwrap();
    stop_test_daemon(running).await;

    // Its command.finished reached the log, and so did another command's, but neither index write committed.
    let rebuilt_only = "jrn_01JC2B3C4D5E6F7G8H9J0K1M2N";
    append_to_daemon_log(
        &paths,
        vec![
            command_finished(&interrupted.journal_id, true, ""),
            command_finished(rebuilt_only, false, "RRS-C0002"),
        ],
    );

    let restarted = start_test_daemon(&paths).await;
    // Nothing is awaited here: the listener is open, so the rows must be there already. The two appended records
    // were indexed, and so were the node's connection records of this start, which came after them.
    assert!(
        daemon_cursor(&restarted) >= Some(before + 3),
        "{:?}",
        daemon_cursor(&restarted)
    );
    let reader = open_index_reader(&restarted.server.store.index_readers).unwrap();
    let finished = get_journal(&reader, &interrupted.journal_id).unwrap().unwrap();
    assert_eq!(finished.ok, Some(true));
    assert_eq!(finished.code, "");
    let failed = get_journal(&reader, rebuilt_only).unwrap().unwrap();
    assert_eq!(failed.ok, Some(false));
    assert_eq!(failed.code, "RRS-C0002");
    // The first command's row is untouched by the replay.
    let first = get_journal(&reader, &created.journal_id).unwrap().unwrap();
    assert_eq!(first.ok, Some(true));
    drop(reader);
    stop_test_daemon(restarted).await;
    remove_test_folder(&paths);
}
