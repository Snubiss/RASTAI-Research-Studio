//! The indexer applies each event record exactly once (layout rust-control-plane.md 6.4 and 6.7, flows/platform.md
//! 6.1): the same batch applied twice gives the same rows, the ack only moves forward, and the rows and the ack
//! commit in one transaction. Run logs move `runs.acked_sequence`, the daemon log `log_cursors.acked_sequence`.
//! A run's log is untrusted (contracts 7.1): a record of another run, an event only the daemon's log carries, another
//! run's checkpoint id, an id of the wrong kind, a changed existing artifact or a parent with a relation a run's log
//! may not give is refused with `RRS-S2004` and changes nothing.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rastai_proto::common::v1::{Diagnostic, EventClass, JobState, NodeState, PluginState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    ArtifactCreated, ArtifactParent, CheckpointSaved, CommandFinished, DiagnosticReported, DocumentChanged,
    EventPayload, EventRecord, JobStateChanged, NodeStateChanged, PinChanged, PluginEventPayload, PluginStateChanged,
    RunMetrics, RunStateChanged, RunTagsChanged,
};
use rastai_store::index::documents::{DocumentRow, get_document};
use rastai_store::index::journal::{JournalStartedRow, get_journal};
use rastai_store::index::kept_tables::{NodeRow, PluginRow, get_node, get_plugin};
use rastai_store::index::log_cursors::get_log_cursor;
use rastai_store::index::reads::{list_table_columns, list_table_rows, open_index_reader};
use rastai_store::index::runs::{JobRow, RunRow, get_job, get_run};
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, close_store, open_store};
use rastai_store::writer_thread::write_index;
use tokio::runtime::Runtime;

mod common;
use common::{remove_test_folder, test_paths};

const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2N";
// A second run, whose rows a record in the first run's log must never reach.
const OTHER_RUN_ID: &str = "run_01JB2B3C4D5E6F7G8H9J0K1M2N";
const OTHER_CHECKPOINT_ID: &str = "ckpt_01JB2B3C4D5E6F7G8H9J0K1M2N";
const RUN_LOG_RECORD_REFUSED: &str = "RRS-S2004";
const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2N";
const NODE_ID: &str = "node_local";
const CHECKPOINT_ID: &str = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2N";
const ARTIFACT_ID: &str = "art_01JA2B3C4D5E6F7G8H9J0K1M2N";
const PLUGIN_ID: &str = "acme.eval";
const DOCUMENT_ID: &str = "doc_01JA2B3C4D5E6F7G8H9J0K1M2N";
const JOURNAL_ID: &str = "jrn_01JA2B3C4D5E6F7G8H9J0K1M2N";
const DAEMON_LOG: &str = "daemon:events";
const CONTENT_HASH: &str = "blake3:0101010101010101010101010101010101010101010101010101010101010101";
// The time every record says it was written; a run's finish time is its last state change's.
const RECORD_TIME_UNIX_MS: i64 = 1_000;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread().build().unwrap()
}

fn record(sequence: u64, run_id: &str, event_type: &str, payload: Payload) -> EventRecord {
    EventRecord {
        format_version: 1,
        sequence,
        run_id: run_id.to_string(),
        job_id: String::new(),
        emitted_unix_ms: RECORD_TIME_UNIX_MS + sequence as i64,
        event_type: event_type.to_string(),
        event_class: EventClass::Durable as i32,
        payload: Some(EventPayload { payload: Some(payload) }),
        blobs: Vec::new(),
    }
}

fn run_state(sequence: u64, state: RunState) -> EventRecord {
    run_state_of(sequence, RUN_ID, state)
}

fn run_state_of(sequence: u64, run_id: &str, state: RunState) -> EventRecord {
    let change = RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(state as i32)),
    };
    record(sequence, run_id, "run.state_changed", Payload::RunStateChanged(change))
}

fn metrics(sequence: u64, step: u64, loss: f64) -> EventRecord {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), loss);
    values.insert("train.tokens_per_second".to_string(), 512.0);
    let metrics = RunMetrics { step, values };
    record(sequence, RUN_ID, "run.metrics", Payload::RunMetrics(metrics))
}

/// A whole run: it prepares, trains, saves its best checkpoint, exports an artifact, completes, and the job's plugin
/// code reports an event of its own. Job states aren't here: the daemon makes them from heartbeats (contracts 7.1).
fn run_log_batch() -> Vec<EventRecord> {
    let plugin_event = PluginEventPayload {
        plugin_id: PLUGIN_ID.to_string(),
        schema: "EvalNote".to_string(),
        payload_json: "{}".to_string(),
    };
    let checkpoint = CheckpointSaved {
        checkpoint_id: CHECKPOINT_ID.to_string(),
        step: 2,
        folder: "checkpoints/step-00000002".to_string(),
        is_best: true,
        ..Default::default()
    };
    let artifact = ArtifactCreated {
        artifact_id: ARTIFACT_ID.to_string(),
        kind: "exported_model".to_string(),
        content_hash: CONTENT_HASH.to_string(),
        size_bytes: 4096,
        home_node_id: NODE_ID.to_string(),
        parents: vec![ArtifactParent {
            id: CHECKPOINT_ID.to_string(),
            relation: "exported_as".to_string(),
        }],
    };
    vec![
        run_state(1, RunState::Preparing),
        run_state(2, RunState::Running),
        metrics(3, 1, 2.5),
        // A diverging run still logs its step: the NaN is stored as NULL.
        metrics(4, 2, f64::NAN),
        record(5, RUN_ID, "checkpoint.saved", Payload::CheckpointSaved(checkpoint)),
        record(6, RUN_ID, "artifact.created", Payload::ArtifactCreated(artifact)),
        run_state(7, RunState::Completed),
        record(8, RUN_ID, "acme.eval.note", Payload::PluginEvent(plugin_event)),
    ]
}

/// The daemon's own records: tags, pins, a node and a plugin changing state, a document edit, a finished command.
fn daemon_log_batch() -> Vec<EventRecord> {
    let tags = RunTagsChanged {
        run_id: RUN_ID.to_string(),
        tags: vec!["baseline".to_string(), "textcnn".to_string()],
    };
    let node = NodeStateChanged {
        node_id: NODE_ID.to_string(),
        from_state: NodeState::Connected as i32,
        to_state: NodeState::Unreachable as i32,
    };
    let plugin = PluginStateChanged {
        plugin_id: PLUGIN_ID.to_string(),
        from_state: PluginState::Enabled as i32,
        to_state: PluginState::Disabled as i32,
        diagnostic: None,
    };
    let document = DocumentChanged {
        document_id: DOCUMENT_ID.to_string(),
        content_hash: CONTENT_HASH.to_string(),
        undo_position: 1,
    };
    let finished = CommandFinished {
        journal_id: JOURNAL_ID.to_string(),
        command_id: "run.tag".to_string(),
        principal: "owner:local".to_string(),
        ok: true,
        code: String::new(),
    };
    vec![
        record(1, "", "run.tags_changed", Payload::RunTagsChanged(tags)),
        record(2, "", "pin.changed", Payload::PinChanged(pin("run", RUN_ID))),
        record(
            3,
            "",
            "pin.changed",
            Payload::PinChanged(pin("checkpoint", CHECKPOINT_ID)),
        ),
        record(4, "", "pin.changed", Payload::PinChanged(pin("artifact", ARTIFACT_ID))),
        record(5, "", "node.state_changed", Payload::NodeStateChanged(node)),
        record(6, "", "plugin.state_changed", Payload::PluginStateChanged(plugin)),
        record(7, "", "document.changed", Payload::DocumentChanged(document)),
        record(8, "", "command.finished", Payload::CommandFinished(finished)),
    ]
}

fn pin(subject_kind: &str, subject_id: &str) -> PinChanged {
    PinChanged {
        subject_kind: subject_kind.to_string(),
        subject_id: subject_id.to_string(),
        pinned: true,
    }
}

/// The rows the daemon writes itself before any record is indexed: the run, its job, the node, the plugin, the
/// document and the command's started journal row.
fn write_starting_rows(runtime: &Runtime, store: &Store) {
    let writes = vec![
        IndexWrite::UpsertRun(RunRow {
            run_id: RUN_ID.to_string(),
            kind: "train".to_string(),
            state: "created".to_string(),
            project_id: "prj_a".to_string(),
            node_id: NODE_ID.to_string(),
            parent_run_id: String::new(),
            record_hash: CONTENT_HASH.to_string(),
            acked_sequence: 0,
            created_unix_ms: 10,
            finished_unix_ms: None,
            pinned: false,
            archive_state: "local".to_string(),
        }),
        IndexWrite::UpsertJob(JobRow {
            job_id: JOB_ID.to_string(),
            run_id: RUN_ID.to_string(),
            node_id: NODE_ID.to_string(),
            kind: "train".to_string(),
            state: "starting".to_string(),
            pid: Some(4242),
            process_group_id: Some(4242),
            exit_code: None,
            error_code: String::new(),
        }),
        IndexWrite::UpsertNode(node_row()),
        IndexWrite::UpsertPlugin(plugin_row()),
        IndexWrite::UpsertDocument(document_row()),
        IndexWrite::JournalStarted(journal_started_row()),
    ];
    runtime
        .block_on(write_index(&store.index, IndexWrite::Batch(writes)))
        .unwrap();
}

fn node_row() -> NodeRow {
    NodeRow {
        node_id: NODE_ID.to_string(),
        name: "this machine".to_string(),
        transport: "local".to_string(),
        address: String::new(),
        state: "connected".to_string(),
        protocol_version: 1,
        capabilities: Vec::new(),
        last_heartbeat_unix_ms: None,
    }
}

fn plugin_row() -> PluginRow {
    PluginRow {
        plugin_id: PLUGIN_ID.to_string(),
        version: "1.0.0".to_string(),
        state: "enabled".to_string(),
        content_hash: CONTENT_HASH.to_string(),
        granted_permissions: Vec::new(),
        source: "local".to_string(),
    }
}

fn document_row() -> DocumentRow {
    DocumentRow {
        document_id: DOCUMENT_ID.to_string(),
        document_type: "rastai.model/1".to_string(),
        path: "models/tiny.rrsmodel".to_string(),
        project_id: "prj_a".to_string(),
        content_hash: "blake3:00".to_string(),
        updated_unix_ms: 20,
    }
}

fn journal_started_row() -> JournalStartedRow {
    JournalStartedRow {
        journal_id: JOURNAL_ID.to_string(),
        command_id: "run.tag".to_string(),
        principal: "owner:local".to_string(),
        arguments_json: "{\"run_id\":\"run_a\"}".to_string(),
        idempotency_key: String::new(),
        started_unix_ms: 30,
        arguments_hash: String::new(),
    }
}

fn apply_run_batch(runtime: &Runtime, store: &Store, records: Vec<EventRecord>) {
    let write = IndexWrite::ApplyRunEvents {
        run_id: RUN_ID.to_string(),
        records,
    };
    runtime.block_on(write_index(&store.index, write)).unwrap();
}

fn apply_daemon_batch(runtime: &Runtime, store: &Store, records: Vec<EventRecord>) {
    let write = IndexWrite::ApplyDaemonEvents {
        log_name: DAEMON_LOG.to_string(),
        records,
    };
    runtime.block_on(write_index(&store.index, write)).unwrap();
}

/// Every row of every table, as text: what "the same rows" means.
fn every_row(store: &Store) -> Vec<(String, Vec<Vec<String>>)> {
    let reader = open_index_reader(&store.index_readers).unwrap();
    let mut tables = Vec::new();
    for table in list_table_columns(&reader).unwrap() {
        let rows = list_table_rows(&reader, &table.table_name).unwrap();
        tables.push((table.table_name, rows));
    }
    tables
}

fn rows_of(store: &Store, table_name: &str) -> Vec<Vec<String>> {
    let reader = open_index_reader(&store.index_readers).unwrap();
    list_table_rows(&reader, table_name).unwrap()
}

fn run_acked_sequence(store: &Store) -> u64 {
    let reader = open_index_reader(&store.index_readers).unwrap();
    get_run(&reader, RUN_ID).unwrap().unwrap().acked_sequence
}

fn daemon_cursor(store: &Store) -> Option<u64> {
    let reader = open_index_reader(&store.index_readers).unwrap();
    get_log_cursor(&reader, DAEMON_LOG).unwrap()
}

#[test]
fn the_same_run_batch_applied_twice_gives_the_same_rows() {
    let paths = test_paths("indexer-run-twice");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);

    apply_run_batch(&runtime, &store, run_log_batch());
    let rows_after_first = every_row(&store);
    apply_run_batch(&runtime, &store, run_log_batch());

    assert_eq!(every_row(&store), rows_after_first);
    assert_eq!(run_acked_sequence(&store), 8);
    // Two values per run.metrics record, one row each, keyed by the record's sequence.
    assert_eq!(rows_of(&store, "run_metrics").len(), 4);
    assert!(rows_of(&store, "run_metrics").iter().any(|row| row[4] == "NULL"));
    assert_eq!(rows_of(&store, "checkpoints").len(), 1);
    assert_eq!(rows_of(&store, "artifact_copies").len(), 1);
    // checkpoint_of from the checkpoint, exported_as from the artifact's parent.
    assert_eq!(rows_of(&store, "lineage").len(), 2);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_run_batch_writes_the_rows_of_its_records() {
    let paths = test_paths("indexer-run-rows");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);

    apply_run_batch(&runtime, &store, run_log_batch());

    let reader = open_index_reader(&store.index_readers).unwrap();
    let run = get_run(&reader, RUN_ID).unwrap().unwrap();
    assert_eq!(run.state, "completed");
    assert_eq!(run.finished_unix_ms, Some(RECORD_TIME_UNIX_MS + 7));
    // A run's log carries no job state, so the job keeps the state the daemon wrote.
    assert_eq!(get_job(&reader, JOB_ID).unwrap().unwrap().state, "starting");
    let checkpoint = &rows_of(&store, "checkpoints")[0];
    assert_eq!(checkpoint[0], format!("{CHECKPOINT_ID:?}"));
    // node_id comes from the run; is_best is 1.
    assert_eq!(checkpoint[4], format!("{NODE_ID:?}"));
    assert_eq!(checkpoint[5], "1");
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn the_run_ack_only_moves_forward() {
    let paths = test_paths("indexer-run-ack");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    // The run fails rather than completes, so `train.resume` may reopen it below (contracts 9.5: failed → resuming).
    let mut batch = run_log_batch();
    batch[6] = run_state(7, RunState::Failed);
    apply_run_batch(&runtime, &store, batch.clone());
    let rows_after_batch = every_row(&store);
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert!(get_run(&reader, RUN_ID).unwrap().unwrap().finished_unix_ms.is_some());
    drop(reader);

    // An older batch re-sent after a reconnect: its records are below the ack, so nothing changes. Applied again,
    // `run.state_changed running` would put the failed run back to running.
    apply_run_batch(&runtime, &store, batch[..3].to_vec());

    assert_eq!(run_acked_sequence(&store), 8);
    assert_eq!(every_row(&store), rows_after_batch);
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert_eq!(get_run(&reader, RUN_ID).unwrap().unwrap().state, "failed");
    drop(reader);

    // A later batch, overlapping the last one, moves it on and indexes only what is new.
    let resumed = vec![batch[7].clone(), run_state(9, RunState::Resuming), metrics(10, 3, 1.5)];
    apply_run_batch(&runtime, &store, resumed);

    assert_eq!(run_acked_sequence(&store), 10);
    assert_eq!(rows_of(&store, "run_metrics").len(), 6);
    let reader = open_index_reader(&store.index_readers).unwrap();
    let run = get_run(&reader, RUN_ID).unwrap().unwrap();
    assert_eq!(run.state, "resuming");
    // A resumed run is running again, so it has no finish time.
    assert_eq!(run.finished_unix_ms, None);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_failed_record_leaves_neither_rows_nor_ack() {
    let paths = test_paths("indexer-one-transaction");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    let rows_before = every_row(&store);
    // The third record's step is past SQLite's largest integer, so its write fails.
    let mut batch = run_log_batch();
    batch[2] = metrics(3, u64::MAX, 2.5);
    let write = IndexWrite::ApplyRunEvents {
        run_id: RUN_ID.to_string(),
        records: batch,
    };

    let error = runtime.block_on(write_index(&store.index, write)).unwrap_err();

    assert_eq!(error.code(), "RRS-S3005");
    // The first two records' rows went with the ack: the batch is applied whole or not at all.
    assert_eq!(every_row(&store), rows_before);
    assert_eq!(run_acked_sequence(&store), 0);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_batch_for_a_run_without_a_row_is_refused() {
    let paths = test_paths("indexer-unknown-run");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    let rows_before = every_row(&store);

    let write = IndexWrite::ApplyRunEvents {
        run_id: RUN_ID.to_string(),
        records: run_log_batch(),
    };
    let error = runtime.block_on(write_index(&store.index, write)).unwrap_err();

    assert_eq!(error.code(), "RRS-S3005");
    assert_eq!(every_row(&store), rows_before);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn the_same_daemon_batch_applied_twice_gives_the_same_rows() {
    let paths = test_paths("indexer-daemon-twice");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    apply_run_batch(&runtime, &store, run_log_batch());
    assert_eq!(daemon_cursor(&store), None);

    apply_daemon_batch(&runtime, &store, daemon_log_batch());
    let rows_after_first = every_row(&store);
    apply_daemon_batch(&runtime, &store, daemon_log_batch());

    assert_eq!(every_row(&store), rows_after_first);
    assert_eq!(daemon_cursor(&store), Some(8));
    assert_daemon_batch_rows(&store);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

/// The rows `daemon_log_batch` makes: tags, pins, the node's state, the document's hash and the command's finish. The
/// plugin's row keeps its saved state: `plugins.state` is the user's decision, and the record may be the host's.
fn assert_daemon_batch_rows(store: &Store) {
    assert_eq!(rows_of(store, "run_tags").len(), 2);
    // The pinned column: the last of checkpoints (contracts 8.2), the sixth of artifacts.
    assert_eq!(rows_of(store, "checkpoints")[0][6], "1");
    assert_eq!(rows_of(store, "artifacts")[0][5], "1");
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert!(get_run(&reader, RUN_ID).unwrap().unwrap().pinned);
    assert_eq!(get_node(&reader, NODE_ID).unwrap().unwrap().state, "unreachable");
    assert_eq!(get_plugin(&reader, PLUGIN_ID).unwrap().unwrap().state, "enabled");
    let document = get_document(&reader, DOCUMENT_ID).unwrap().unwrap();
    assert_eq!(document.content_hash, CONTENT_HASH);
    assert_eq!(document.updated_unix_ms, RECORD_TIME_UNIX_MS + 7);
    let journal = get_journal(&reader, JOURNAL_ID).unwrap().unwrap();
    assert_eq!(journal.ok, Some(true));
    assert_eq!(journal.finished_unix_ms, Some(RECORD_TIME_UNIX_MS + 8));
    // The arguments and start the daemon wrote when the command started are kept.
    assert_eq!(journal.started_unix_ms, 30);
    assert_eq!(journal.arguments_json, "{\"run_id\":\"run_a\"}");
}

#[test]
fn the_daemon_cursor_only_moves_forward() {
    let paths = test_paths("indexer-daemon-cursor");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    apply_daemon_batch(&runtime, &store, daemon_log_batch());

    // A later record removes the tags and moves the cursor on.
    let untag = RunTagsChanged {
        run_id: RUN_ID.to_string(),
        tags: Vec::new(),
    };
    apply_daemon_batch(
        &runtime,
        &store,
        vec![record(9, "", "run.tags_changed", Payload::RunTagsChanged(untag))],
    );
    assert_eq!(daemon_cursor(&store), Some(9));
    assert!(rows_of(&store, "run_tags").is_empty());

    // The older record that set the tags, re-sent after a crash: below the cursor, so skipped. Applied again, it
    // would bring the removed tags back.
    apply_daemon_batch(&runtime, &store, daemon_log_batch()[..1].to_vec());

    assert_eq!(daemon_cursor(&store), Some(9));
    assert!(rows_of(&store, "run_tags").is_empty());
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_command_finished_without_a_started_row_makes_one() {
    // The rebuild's case (contracts 8.2): the started row is gone, the finished record makes the row from itself.
    let paths = test_paths("indexer-journal-from-record");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    let failed = CommandFinished {
        journal_id: JOURNAL_ID.to_string(),
        command_id: "train.start".to_string(),
        principal: "owner:local".to_string(),
        ok: false,
        code: "RRS-T0001".to_string(),
    };
    apply_daemon_batch(
        &runtime,
        &store,
        vec![record(1, "", "command.finished", Payload::CommandFinished(failed))],
    );

    let reader = open_index_reader(&store.index_readers).unwrap();
    let journal = get_journal(&reader, JOURNAL_ID).unwrap().unwrap();
    assert_eq!(journal.ok, Some(false));
    assert_eq!(journal.code, "RRS-T0001");
    assert_eq!(journal.started_unix_ms, RECORD_TIME_UNIX_MS + 1);
    assert_eq!(journal.finished_unix_ms, Some(RECORD_TIME_UNIX_MS + 1));
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_failed_job_keeps_its_error_code() {
    let paths = test_paths("indexer-job-error");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    let failed = JobStateChanged {
        job_id: JOB_ID.to_string(),
        from_state: JobState::Running as i32,
        to_state: JobState::Failed as i32,
        exit_code: Some(4),
        diagnostic: Some(Diagnostic {
            code: "RRS-S3002".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    // The daemon makes job states from the node's heartbeats and logs them in its own log (contracts 7.1).
    apply_daemon_batch(
        &runtime,
        &store,
        vec![record(1, RUN_ID, "job.state_changed", Payload::JobStateChanged(failed))],
    );

    let reader = open_index_reader(&store.index_readers).unwrap();
    let job = get_job(&reader, JOB_ID).unwrap().unwrap();
    assert_eq!(job.state, "failed");
    assert_eq!(job.exit_code, Some(4));
    assert_eq!(job.error_code, "RRS-S3002");
    // The job's own columns, written by the daemon when it started the job, are kept.
    assert_eq!(job.pid, Some(4242));
    assert_eq!(job.node_id, NODE_ID);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

// ---- A run's log is untrusted (contracts 7.1) ----

/// A second run with a checkpoint of its own, indexed from its own log.
fn write_other_run(runtime: &Runtime, store: &Store) {
    let run = RunRow {
        run_id: OTHER_RUN_ID.to_string(),
        kind: "train".to_string(),
        state: "created".to_string(),
        project_id: "prj_a".to_string(),
        node_id: NODE_ID.to_string(),
        parent_run_id: String::new(),
        record_hash: CONTENT_HASH.to_string(),
        acked_sequence: 0,
        created_unix_ms: 11,
        finished_unix_ms: None,
        pinned: false,
        archive_state: "local".to_string(),
    };
    runtime
        .block_on(write_index(&store.index, IndexWrite::UpsertRun(run)))
        .unwrap();
    let checkpoint = CheckpointSaved {
        checkpoint_id: OTHER_CHECKPOINT_ID.to_string(),
        step: 5,
        folder: "checkpoints/step-00000005".to_string(),
        is_best: true,
        ..Default::default()
    };
    let records = vec![
        run_state_of(1, OTHER_RUN_ID, RunState::Running),
        record(
            2,
            OTHER_RUN_ID,
            "checkpoint.saved",
            Payload::CheckpointSaved(checkpoint),
        ),
    ];
    let write = IndexWrite::ApplyRunEvents {
        run_id: OTHER_RUN_ID.to_string(),
        records,
    };
    runtime.block_on(write_index(&store.index, write)).unwrap();
}

/// Applies `records` as `RUN_ID`'s log and expects the refusal, with nothing changed anywhere.
fn assert_run_batch_refused(runtime: &Runtime, store: &Store, records: Vec<EventRecord>, refused_sequence: u64) {
    let rows_before = every_row(store);
    let acked_before = run_acked_sequence(store);
    let write = IndexWrite::ApplyRunEvents {
        run_id: RUN_ID.to_string(),
        records,
    };

    let error = runtime.block_on(write_index(&store.index, write)).unwrap_err();

    assert_eq!(error.code(), RUN_LOG_RECORD_REFUSED);
    match error {
        rastai_store::error::StoreError::RunLogRecordRefused { run_id, sequence, .. } => {
            assert_eq!(run_id, RUN_ID);
            assert_eq!(sequence, refused_sequence);
        }
        other => panic!("expected a refused record, got {other:?}"),
    }
    assert_eq!(every_row(store), rows_before);
    assert_eq!(run_acked_sequence(store), acked_before);
}

#[test]
fn a_run_log_record_of_another_run_changes_nothing() {
    let paths = test_paths("indexer-other-run");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);
    // The job of RUN_ID writes a record that fails OTHER_RUN_ID, after one of its own.
    let batch = vec![
        run_state(1, RunState::Running),
        run_state_of(2, OTHER_RUN_ID, RunState::Failed),
        metrics(3, 1, 2.5),
    ];

    assert_run_batch_refused(&runtime, &store, batch.clone(), 2);

    // The tail sends the records before the refused one again and stops there: they index, the other run doesn't
    // move, and the ack stays before the refused record.
    apply_run_batch(&runtime, &store, batch[..1].to_vec());
    assert_run_batch_refused(&runtime, &store, batch, 2);
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert_eq!(get_run(&reader, RUN_ID).unwrap().unwrap().state, "running");
    let other = get_run(&reader, OTHER_RUN_ID).unwrap().unwrap();
    assert_eq!(other.state, "running");
    assert_eq!(other.finished_unix_ms, None);
    drop(reader);
    assert_eq!(run_acked_sequence(&store), 1);
    assert!(rows_of(&store, "run_metrics").is_empty());
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

/// Every M1 event a run's log may not carry: the daemon's own (each of `daemon_log_batch`), the node's job state,
/// and the ephemeral ones that are in no log. Each names the log's own run, so only its type is wrong.
fn daemon_only_records() -> Vec<EventRecord> {
    let job_succeeded = JobStateChanged {
        job_id: JOB_ID.to_string(),
        from_state: JobState::Running as i32,
        to_state: JobState::Succeeded as i32,
        exit_code: Some(0),
        ..Default::default()
    };
    let mut records = vec![record(
        1,
        RUN_ID,
        "job.state_changed",
        Payload::JobStateChanged(job_succeeded),
    )];
    for daemon_record in daemon_log_batch() {
        let mut own = daemon_record;
        own.sequence = 1;
        own.run_id = RUN_ID.to_string();
        records.push(own);
    }
    records.push(record(
        1,
        RUN_ID,
        "job.progress",
        Payload::JobProgress(Default::default()),
    ));
    records.push(record(
        1,
        RUN_ID,
        "job.log_line",
        Payload::JobLogLine(Default::default()),
    ));
    records.push(record(
        1,
        RUN_ID,
        "node.heartbeat",
        Payload::NodeHeartbeat(Default::default()),
    ));
    records
}

#[test]
fn a_run_log_carrying_a_daemon_only_event_changes_nothing() {
    let paths = test_paths("indexer-daemon-only");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);

    for daemon_only in daemon_only_records() {
        assert_run_batch_refused(&runtime, &store, vec![daemon_only], 1);
    }

    // The same records from the daemon's own log are applied: the refusal is about where they come from.
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert_eq!(get_job(&reader, JOB_ID).unwrap().unwrap().state, "starting");
    assert!(!get_run(&reader, RUN_ID).unwrap().unwrap().pinned);
    drop(reader);
    apply_daemon_batch(&runtime, &store, daemon_log_batch());
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert!(get_run(&reader, RUN_ID).unwrap().unwrap().pinned);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_run_log_can_not_take_another_runs_checkpoint() {
    let paths = test_paths("indexer-other-checkpoint");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);
    let stolen = CheckpointSaved {
        checkpoint_id: OTHER_CHECKPOINT_ID.to_string(),
        step: 1,
        folder: "checkpoints/step-00000001".to_string(),
        is_best: true,
        ..Default::default()
    };

    let batch = vec![record(1, RUN_ID, "checkpoint.saved", Payload::CheckpointSaved(stolen))];
    assert_run_batch_refused(&runtime, &store, batch, 1);

    // The other run still owns it: column 1 is run_id (contracts 8.2).
    let checkpoints = rows_of(&store, "checkpoints");
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0][1], format!("{OTHER_RUN_ID:?}"));
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

/// The other run makes `ARTIFACT_ID` at sequence 3 (so its `created_unix_ms` is `RECORD_TIME_UNIX_MS + 3`), exported
/// from its checkpoint.
fn write_other_run_artifact(runtime: &Runtime, store: &Store) -> ArtifactCreated {
    let made = ArtifactCreated {
        artifact_id: ARTIFACT_ID.to_string(),
        kind: "exported_model".to_string(),
        content_hash: CONTENT_HASH.to_string(),
        size_bytes: 4096,
        home_node_id: NODE_ID.to_string(),
        parents: vec![parent(OTHER_CHECKPOINT_ID, "exported_as")],
    };
    let write = IndexWrite::ApplyRunEvents {
        run_id: OTHER_RUN_ID.to_string(),
        records: vec![record(
            3,
            OTHER_RUN_ID,
            "artifact.created",
            Payload::ArtifactCreated(made.clone()),
        )],
    };
    runtime.block_on(write_index(&store.index, write)).unwrap();
    made
}

fn parent(id: &str, relation: &str) -> ArtifactParent {
    ArtifactParent {
        id: id.to_string(),
        relation: relation.to_string(),
    }
}

fn artifact_record(sequence: u64, artifact: ArtifactCreated) -> EventRecord {
    record(sequence, RUN_ID, "artifact.created", Payload::ArtifactCreated(artifact))
}

fn checkpoint_record(sequence: u64, checkpoint_id: &str) -> EventRecord {
    let saved = CheckpointSaved {
        checkpoint_id: checkpoint_id.to_string(),
        step: sequence,
        folder: format!("checkpoints/step-{sequence:08}"),
        is_best: false,
        ..Default::default()
    };
    record(sequence, RUN_ID, "checkpoint.saved", Payload::CheckpointSaved(saved))
}

#[test]
fn a_run_log_can_not_rewrite_an_existing_artifact() {
    let paths = test_paths("indexer-rewrite-artifact");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);
    let made = write_other_run_artifact(&runtime, &store);
    let artifacts_before = rows_of(&store, "artifacts");
    let lineage_before = rows_of(&store, "lineage");
    let copies_before = rows_of(&store, "artifact_copies");

    let mut rewritten = made.clone();
    rewritten.content_hash = "blake3:0202020202020202020202020202020202020202020202020202020202020202".to_string();
    assert_run_batch_refused(&runtime, &store, vec![artifact_record(3, rewritten)], 3);
    // The same contents at another time would move created_unix_ms: sequences 1 and 2 give 1001 and 1002.
    assert_run_batch_refused(&runtime, &store, vec![artifact_record(1, made.clone())], 1);
    assert_run_batch_refused(&runtime, &store, vec![artifact_record(2, made.clone())], 2);
    // The same contents at the same time, but with a parent more, would graft a lineage row onto the other run's
    // artifact.
    let mut grafted = made.clone();
    grafted.parents.push(parent(CHECKPOINT_ID, "evaluated_by"));
    assert_run_batch_refused(&runtime, &store, vec![artifact_record(3, grafted)], 3);
    let mut claimed = made.clone();
    claimed.parents = vec![parent(RUN_ID, "checkpoint_of")];
    assert_run_batch_refused(&runtime, &store, vec![artifact_record(3, claimed)], 3);

    // Column 6 is created_unix_ms (contracts 8.2): still the other run's time.
    let artifacts = rows_of(&store, "artifacts");
    assert_eq!(artifacts, artifacts_before);
    assert_eq!(artifacts[0][6], (RECORD_TIME_UNIX_MS + 3).to_string());
    assert_eq!(rows_of(&store, "lineage"), lineage_before);
    assert_eq!(rows_of(&store, "artifact_copies"), copies_before);

    // An exact repeat, time and parents included, changes no row, so it is accepted: a re-sent record is never
    // refused. Only the run's ack moves.
    apply_run_batch(&runtime, &store, vec![artifact_record(3, made)]);
    assert_eq!(run_acked_sequence(&store), 3);
    assert_eq!(rows_of(&store, "artifacts"), artifacts_before);
    assert_eq!(rows_of(&store, "lineage"), lineage_before);
    assert_eq!(rows_of(&store, "artifact_copies"), copies_before);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_run_log_can_not_give_a_parent_a_relation_outside_its_own() {
    let paths = test_paths("indexer-parent-relation");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);
    let lineage_before = rows_of(&store, "lineage");
    let new_artifact = ArtifactCreated {
        artifact_id: ARTIFACT_ID.to_string(),
        kind: "exported_model".to_string(),
        content_hash: CONTENT_HASH.to_string(),
        size_bytes: 4096,
        home_node_id: NODE_ID.to_string(),
        parents: Vec::new(),
    };

    // checkpoint_of comes only from checkpoint.saved; branch_of and trained_on name a message or a run; the last is
    // in no list.
    for relation in ["checkpoint_of", "branch_of", "trained_on", "made_up_relation"] {
        let mut linked = new_artifact.clone();
        linked.parents = vec![parent(OTHER_RUN_ID, relation)];
        assert_run_batch_refused(&runtime, &store, vec![artifact_record(1, linked)], 1);
    }

    assert!(rows_of(&store, "artifacts").is_empty());
    assert_eq!(rows_of(&store, "lineage"), lineage_before);
    // A parent from another run with an artifact's relation is lineage, not a graft: its child is the new artifact.
    let mut evaluated = new_artifact;
    evaluated.parents = vec![parent(OTHER_CHECKPOINT_ID, "evaluated_by")];
    apply_run_batch(&runtime, &store, vec![artifact_record(1, evaluated)]);
    assert_eq!(rows_of(&store, "lineage").len(), lineage_before.len() + 1);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_run_log_can_not_name_another_kind_of_id() {
    let paths = test_paths("indexer-id-kind");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    write_other_run(&runtime, &store);
    let made = write_other_run_artifact(&runtime, &store);

    // An artifact named like another run's checkpoint or like the other run would hang its lineage rows on them.
    for subject_id in [OTHER_CHECKPOINT_ID, OTHER_RUN_ID, "art_short"] {
        let mut misnamed = made.clone();
        misnamed.artifact_id = subject_id.to_string();
        assert_run_batch_refused(&runtime, &store, vec![artifact_record(1, misnamed)], 1);
    }
    // A checkpoint named like the other run's artifact or the other run would hang a checkpoint_of row on them.
    for subject_id in [ARTIFACT_ID, OTHER_RUN_ID] {
        assert_run_batch_refused(&runtime, &store, vec![checkpoint_record(1, subject_id)], 1);
    }

    assert_eq!(rows_of(&store, "artifacts").len(), 1);
    assert_eq!(rows_of(&store, "checkpoints").len(), 1);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_run_log_may_carry_a_diagnostic() {
    let paths = test_paths("indexer-run-diagnostic");
    let runtime = runtime();
    let store = open_store(&paths).unwrap();
    write_starting_rows(&runtime, &store);
    let rows_before = every_row(&store);
    let reported = DiagnosticReported {
        diagnostic: Some(Diagnostic {
            code: "RRS-S3002".to_string(),
            ..Default::default()
        }),
        subject_id: RUN_ID.to_string(),
    };

    apply_run_batch(
        &runtime,
        &store,
        vec![record(
            1,
            RUN_ID,
            "diagnostic.reported",
            Payload::DiagnosticReported(reported),
        )],
    );

    // No rows, but the ack moves past it.
    assert_eq!(run_acked_sequence(&store), 1);
    let mut rows_after = every_row(&store);
    let mut expected = rows_before;
    for tables in [&mut rows_after, &mut expected] {
        tables.retain(|(table_name, _)| table_name != "runs");
    }
    assert_eq!(rows_after, expected);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}
