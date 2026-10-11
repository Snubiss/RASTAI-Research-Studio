//! The indexer applies each event record exactly once (layout rust-control-plane.md 6.4 and 6.7, flows/platform.md
//! 6.1): the same batch applied twice gives the same rows, the ack only moves forward, and the rows and the ack
//! commit in one transaction. Run logs move `runs.acked_sequence`, the daemon log `log_cursors.acked_sequence`.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rastai_proto::common::v1::{Diagnostic, EventClass, JobState, NodeState, PluginState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    ArtifactCreated, ArtifactParent, CheckpointSaved, CommandFinished, DocumentChanged, EventPayload, EventRecord,
    JobStateChanged, NodeStateChanged, PinChanged, PluginStateChanged, RunMetrics, RunStateChanged, RunTagsChanged,
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
    let change = RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(state as i32)),
    };
    record(sequence, RUN_ID, "run.state_changed", Payload::RunStateChanged(change))
}

fn metrics(sequence: u64, step: u64, loss: f64) -> EventRecord {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), loss);
    values.insert("train.tokens_per_second".to_string(), 512.0);
    let metrics = RunMetrics { step, values };
    record(sequence, RUN_ID, "run.metrics", Payload::RunMetrics(metrics))
}

/// A whole run: the job starts, the run trains, saves its best checkpoint, exports an artifact and completes.
fn run_log_batch() -> Vec<EventRecord> {
    let job_running = JobStateChanged {
        job_id: JOB_ID.to_string(),
        from_state: JobState::Starting as i32,
        to_state: JobState::Running as i32,
        ..Default::default()
    };
    let job_succeeded = JobStateChanged {
        job_id: JOB_ID.to_string(),
        from_state: JobState::Running as i32,
        to_state: JobState::Succeeded as i32,
        exit_code: Some(0),
        ..Default::default()
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
        record(1, RUN_ID, "job.state_changed", Payload::JobStateChanged(job_running)),
        run_state(2, RunState::Running),
        metrics(3, 1, 2.5),
        // A diverging run still logs its step: the NaN is stored as NULL.
        metrics(4, 2, f64::NAN),
        record(5, RUN_ID, "checkpoint.saved", Payload::CheckpointSaved(checkpoint)),
        record(6, RUN_ID, "artifact.created", Payload::ArtifactCreated(artifact)),
        run_state(7, RunState::Completed),
        record(8, RUN_ID, "job.state_changed", Payload::JobStateChanged(job_succeeded)),
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
    let job = get_job(&reader, JOB_ID).unwrap().unwrap();
    assert_eq!(job.state, "succeeded");
    assert_eq!(job.exit_code, Some(0));
    // The job's own columns, written by the daemon, are kept.
    assert_eq!(job.pid, Some(4242));
    assert_eq!(job.node_id, NODE_ID);
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
    // The third record's checkpoint belongs to a run the index doesn't know, which its foreign key refuses.
    let mut batch = run_log_batch();
    let orphan = CheckpointSaved {
        checkpoint_id: "ckpt_orphan".to_string(),
        step: 1,
        folder: "checkpoints/step-00000001".to_string(),
        ..Default::default()
    };
    batch[2] = record(3, "run_unknown", "checkpoint.saved", Payload::CheckpointSaved(orphan));
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

/// The rows `daemon_log_batch` makes: tags, pins, the node's and the plugin's state, the document's hash and the
/// command's finish.
fn assert_daemon_batch_rows(store: &Store) {
    assert_eq!(rows_of(store, "run_tags").len(), 2);
    // The pinned column: the last of checkpoints (contracts 8.2), the sixth of artifacts.
    assert_eq!(rows_of(store, "checkpoints")[0][6], "1");
    assert_eq!(rows_of(store, "artifacts")[0][5], "1");
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert!(get_run(&reader, RUN_ID).unwrap().unwrap().pinned);
    assert_eq!(get_node(&reader, NODE_ID).unwrap().unwrap().state, "unreachable");
    assert_eq!(get_plugin(&reader, PLUGIN_ID).unwrap().unwrap().state, "disabled");
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
    apply_run_batch(
        &runtime,
        &store,
        vec![record(1, RUN_ID, "job.state_changed", Payload::JobStateChanged(failed))],
    );

    let reader = open_index_reader(&store.index_readers).unwrap();
    let job = get_job(&reader, JOB_ID).unwrap().unwrap();
    assert_eq!(job.state, "failed");
    assert_eq!(job.exit_code, Some(4));
    assert_eq!(job.error_code, "RRS-S3002");
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}
