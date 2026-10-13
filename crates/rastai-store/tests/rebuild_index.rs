//! The offline rebuild of `rastai.db` (plan M1-33; layout rust-control-plane.md 13.2, flows/platform.md 6.4) on the
//! fixture data folder `fixtures/data-dirs/two_runs/`: two runs' folders, the daemon log and one blob. Each test
//! copies it, indexes it live with the writes the daemon sends (the kept tables, each run's start, the journal, then
//! the logs through `ApplyDaemonEvents` and `ApplyRunEvents`, interleaved as a daemon would), then rebuilds and
//! compares. rastai-cli's `tests/rebuild_index.rs` does the same against the real daemon.
//!
//! The fixture is built by the recipe below and committed. `committed_fixture_matches_its_recipe` fails when the
//! files and the recipe differ; after a deliberate change, rewrite them with
//! `cargo test -p rastai-store --test rebuild_index -- --ignored write_rebuild_fixture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use rastai_core::paths::{
    DataPaths, blobs_dir, rastai_db_path, rastai_db_rebuilding_path, run_dir, run_events_log_path,
};
use rastai_core::proto_json::job_spec_to_json;
use rastai_core::time::parse_rfc3339;
use rastai_formats::documents::Document;
use rastai_formats::documents::run_record::{RUN_RECORD_TYPE_NAME, RunKind, RunRecipe, RunRecord};
use rastai_formats::hashing::hash_bytes;
use rastai_formats::header::SharedHeader;
use rastai_formats::validate::write_document;
use rastai_proto::common::v1::{Diagnostic, EventClass, JobState, NodeState, PluginState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    ArtifactCreated, ArtifactParent, CheckpointSaved, CommandFinished, EventPayload, EventRecord, JobStateChanged,
    NodeStateChanged, PinChanged, PluginStateChanged, RunMetrics, RunStateChanged, RunTagsChanged,
};
use rastai_proto::node::v1::{JobKind, JobSpec};
use rastai_store::event_log::frame::{EVENT_LOG_HEADER, encode_frame};
use rastai_store::index::journal::{JournalFinishedRow, JournalStartedRow};
use rastai_store::index::kept_tables::{NodeRow, PluginRow, ProjectRow, TokenRow};
use rastai_store::index::reads::{TableRows, list_every_table_rows, open_index_reader};
use rastai_store::index::runs::{JobRow, RunRow};
use rastai_store::index::writes::IndexWrite;
use rastai_store::rebuild::{RebuildOptions, RebuildReport, rebuild_index, rebuilt_table_differences};
use rastai_store::store::{close_store, open_store};
use rastai_store::writer_thread::write_index;

use common::{remove_test_folder, test_paths};

const FIXTURE_DATA_FOLDER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/data-dirs/two_runs/data");

// Ids in the form the proto goldens use (fixtures/proto-golden/events/EventRecord.json).
const RUN_A: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2S";
const RUN_B: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2T";
const JOB_A: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q";
const JOB_B: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2R";
const CHECKPOINT_A: &str = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2V";
const ARTIFACT_A: &str = "art_01JA2B3C4D5E6F7G8H9J0K1M2W";
const JOURNAL_A: &str = "jrn_01JA2B3C4D5E6F7G8H9J0K1M2X";
const JOURNAL_B: &str = "jrn_01JA2B3C4D5E6F7G8H9J0K1M2Y";
const NODE_ID: &str = "node_fixture_embedded";
const PLUGIN_ID: &str = "acme_demo";
const PROJECT_ID: &str = "prj_01JA2B3C4D5E6F7G8H9J0K1M2Z";
/// A fixed version, so the committed run.json files don't change with the app's.
const FIXTURE_APP_VERSION: &str = "0.1.0";
const RUN_RECORD_SCHEMA_URL: &str = "https://schemas.rastai.dev/rastai.run_record/1.json";
const CREATED_AT_A: &str = "2026-10-08T12:00:00.123Z";
const CREATED_AT_B: &str = "2026-10-08T12:05:00.456Z";
const FIRST_EMITTED_UNIX_MS: i64 = 1_791_700_000_000;
/// The exported file run A's artifact names: its bytes are the one blob in the fixture.
const EXPORTED_BYTES: &[u8] = b"an exported model, standing in for a GGUF file";
/// Live, a tail sends a run's records in batches; two records each here, so the ack moves several times.
const LIVE_BATCH_RECORDS: usize = 2;
/// The daemon records indexed before run A's log is: the node's connect and the job's start.
const DAEMON_RECORDS_BEFORE_RUN_A: usize = 3;
const DAEMON_LOG_NAME: &str = "daemon:1";
/// Run A's log record whose frame the corrupt test damages: the third of six (the second metrics record).
const CORRUPTED_RUN_A_RECORD: usize = 2;
const TOKEN_REVOKED_UNIX_MS: i64 = 1_791_700_100_000;

// ---- The recipe ----

fn record(sequence: u64, run_id: &str, job_id: &str, event_type: &str, payload: Payload) -> EventRecord {
    EventRecord {
        format_version: 1,
        sequence,
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        emitted_unix_ms: FIRST_EMITTED_UNIX_MS + 1000 * sequence as i64,
        event_type: event_type.to_string(),
        event_class: EventClass::Durable as i32,
        payload: Some(EventPayload { payload: Some(payload) }),
        blobs: Vec::new(),
    }
}

fn run_state(state: RunState) -> Payload {
    Payload::RunStateChanged(RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(state as i32)),
    })
}

fn metrics(step: u64, loss: f64) -> Payload {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), loss);
    values.insert("train.accuracy".to_string(), 1.0 - loss / 4.0);
    Payload::RunMetrics(RunMetrics { step, values })
}

/// Run A trains, checkpoints and exports: states, metrics, a checkpoint and an artifact with a blob.
fn run_a_records() -> Vec<EventRecord> {
    let checkpoint = Payload::CheckpointSaved(CheckpointSaved {
        checkpoint_id: CHECKPOINT_A.to_string(),
        step: 2,
        folder: "checkpoints/step_00000002".to_string(),
        file_hashes: BTreeMap::new(),
        is_best: true,
    });
    let artifact = Payload::ArtifactCreated(ArtifactCreated {
        artifact_id: ARTIFACT_A.to_string(),
        kind: "export_gguf".to_string(),
        content_hash: hash_bytes(EXPORTED_BYTES),
        size_bytes: EXPORTED_BYTES.len() as u64,
        home_node_id: NODE_ID.to_string(),
        parents: vec![ArtifactParent {
            id: CHECKPOINT_A.to_string(),
            relation: "exported_as".to_string(),
        }],
    });
    let payloads = vec![
        ("run.state_changed", run_state(RunState::Running)),
        ("run.metrics", metrics(1, 2.25)),
        ("run.metrics", metrics(2, 1.5)),
        ("checkpoint.saved", checkpoint),
        ("artifact.created", artifact),
        ("run.state_changed", run_state(RunState::Completed)),
    ];
    numbered(RUN_A, JOB_A, payloads)
}

fn numbered(run_id: &str, job_id: &str, payloads: Vec<(&str, Payload)>) -> Vec<EventRecord> {
    let mut records = Vec::new();
    for (index, (event_type, payload)) in payloads.into_iter().enumerate() {
        records.push(record(index as u64 + 1, run_id, job_id, event_type, payload));
    }
    records
}

fn job_moved(job_id: &str, from: JobState, to: JobState, exit_code: Option<i32>, code: &str) -> Payload {
    let diagnostic = (!code.is_empty()).then(|| Diagnostic {
        code: code.to_string(),
        ..Diagnostic::default()
    });
    Payload::JobStateChanged(JobStateChanged {
        job_id: job_id.to_string(),
        from_state: from as i32,
        to_state: to as i32,
        reason: String::new(),
        exit_code,
        diagnostic,
    })
}

fn command_finished(journal_id: &str) -> Payload {
    Payload::CommandFinished(CommandFinished {
        journal_id: journal_id.to_string(),
        command_id: "train.start".to_string(),
        principal: "owner".to_string(),
        ok: true,
        code: String::new(),
    })
}

/// The daemon's own records, in the order a daemon wrote them: run B's job failed before it started, so the daemon
/// wrote its run's end; the plugin's `plugin.state_changed` must not move `plugins.state` (layout 6.4).
fn daemon_records() -> Vec<EventRecord> {
    let node = Payload::NodeStateChanged(NodeStateChanged {
        node_id: NODE_ID.to_string(),
        from_state: NodeState::Handshaking as i32,
        to_state: NodeState::Connected as i32,
    });
    let pin = Payload::PinChanged(PinChanged {
        subject_kind: "checkpoint".to_string(),
        subject_id: CHECKPOINT_A.to_string(),
        pinned: true,
    });
    let tags = Payload::RunTagsChanged(RunTagsChanged {
        run_id: RUN_A.to_string(),
        tags: vec!["baseline".to_string(), "textcnn".to_string()],
    });
    let plugin = Payload::PluginStateChanged(PluginStateChanged {
        plugin_id: PLUGIN_ID.to_string(),
        from_state: PluginState::Enabled as i32,
        to_state: PluginState::Incompatible as i32,
        diagnostic: None,
    });
    let entries: Vec<(&str, &str, &str, Payload)> = vec![
        ("", "", "node.state_changed", node),
        (
            RUN_A,
            JOB_A,
            "job.state_changed",
            job_moved(JOB_A, JobState::Queued, JobState::Starting, None, ""),
        ),
        (
            RUN_A,
            JOB_A,
            "job.state_changed",
            job_moved(JOB_A, JobState::Starting, JobState::Running, None, ""),
        ),
        (
            RUN_A,
            JOB_A,
            "job.state_changed",
            job_moved(JOB_A, JobState::Running, JobState::Succeeded, Some(0), ""),
        ),
        ("", "", "command.finished", command_finished(JOURNAL_A)),
        ("", "", "pin.changed", pin),
        ("", "", "run.tags_changed", tags),
        (
            RUN_B,
            JOB_B,
            "job.state_changed",
            job_moved(JOB_B, JobState::Queued, JobState::Failed, None, "RRS-N0004"),
        ),
        (RUN_B, JOB_B, "run.state_changed", run_state(RunState::Failed)),
        ("", "", "plugin.state_changed", plugin),
        ("", "", "command.finished", command_finished(JOURNAL_B)),
    ];
    let mut records = Vec::new();
    for (index, (run_id, job_id, event_type, payload)) in entries.into_iter().enumerate() {
        records.push(record(index as u64 + 1, run_id, job_id, event_type, payload));
    }
    records
}

/// A run's `run.json` as `submit_job` writes it.
fn run_record_bytes(run_id: &str, job_id: &str, created_at: &str) -> Vec<u8> {
    let spec = JobSpec {
        job_id: job_id.to_string(),
        run_id: run_id.to_string(),
        kind: JobKind::Train as i32,
        environment_id: "cpu".to_string(),
        arguments_json: r#"{"design_path":"/designs/textcnn.rrsmodel"}"#.to_string(),
        data_policy: "local-only".to_string(),
        ..JobSpec::default()
    };
    let record = RunRecord {
        header: SharedHeader {
            schema_url: RUN_RECORD_SCHEMA_URL.to_string(),
            document_type: RUN_RECORD_TYPE_NAME.to_string(),
            schema_version: 1,
            created_with: FIXTURE_APP_VERSION.to_string(),
            id: run_id.to_string(),
            content_hash: String::new(),
            derived_from: Vec::new(),
            authors: Vec::new(),
            license: String::new(),
            source_url: String::new(),
            requires: BTreeMap::new(),
        },
        run_id: run_id.to_string(),
        kind: RunKind::Train,
        job_spec: Some(job_spec_to_json(&spec)),
        recipe: Some(RunRecipe {
            name: "classification".to_string(),
            settings: BTreeMap::new(),
        }),
        dataset_hashes: Vec::new(),
        base_model: None,
        plugin_versions: BTreeMap::new(),
        environment_lock_hash: String::new(),
        seed: Some(7),
        hardware_fingerprint: None,
        git_commit: String::new(),
        app_version: FIXTURE_APP_VERSION.to_string(),
        parent_run_id: String::new(),
        chain: None,
        source: None,
        created_at: created_at.to_string(),
    };
    write_document(&Document::RunRecord(record)).unwrap()
}

fn log_bytes(records: &[EventRecord]) -> Vec<u8> {
    let mut bytes = EVENT_LOG_HEADER.to_vec();
    for record in records {
        bytes.extend_from_slice(&encode_frame(record).unwrap());
    }
    bytes
}

/// `<root>/<first two hex digits>/<64 hex digits>`, where the blob store keeps a blob.
fn blob_relative_path(bytes: &[u8]) -> String {
    let hash = hash_bytes(bytes);
    let hex = hash.trim_start_matches("blake3:");
    format!("blobs/{}/{hex}", &hex[..2])
}

/// Every file of the fixture's data folder, by its path inside it. Run B's job failed before it started, so its
/// folder holds no log.
fn fixture_files() -> Vec<(String, Vec<u8>)> {
    vec![
        (
            format!("runs/{RUN_A}/run.json"),
            run_record_bytes(RUN_A, JOB_A, CREATED_AT_A),
        ),
        (format!("runs/{RUN_A}/events.log"), log_bytes(&run_a_records())),
        (
            format!("runs/{RUN_B}/run.json"),
            run_record_bytes(RUN_B, JOB_B, CREATED_AT_B),
        ),
        ("daemon/events.log".to_string(), log_bytes(&daemon_records())),
        (blob_relative_path(EXPORTED_BYTES), EXPORTED_BYTES.to_vec()),
    ]
}

/// Every file under `folder`, by its path inside it.
fn files_under(folder: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).unwrap().to_string_lossy().replace('\\', "/");
            found.push((relative, std::fs::read(&path).unwrap()));
        }
    }
    found.sort();
    found
}

#[test]
fn committed_fixture_matches_its_recipe() {
    let mut expected = fixture_files();
    expected.sort();
    assert_eq!(files_under(Path::new(FIXTURE_DATA_FOLDER)), expected);
}

/// Rewrites the committed fixture from the recipe (see the top).
#[test]
#[ignore = "rewrites fixtures/data-dirs/two_runs; run it only after a deliberate change to the recipe"]
fn write_rebuild_fixture() {
    let folder = Path::new(FIXTURE_DATA_FOLDER);
    let _ = std::fs::remove_dir_all(folder);
    for (relative, bytes) in fixture_files() {
        let path = folder.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

// ---- Live indexing, as the daemon does it ----

fn copy_fixture(paths: &DataPaths) {
    for (relative, bytes) in fixture_files() {
        let path = paths.data.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

fn token(token_id: &str, created_by: &str) -> TokenRow {
    TokenRow {
        token_id: token_id.to_string(),
        name: token_id.to_string(),
        token_hash: hash_bytes(token_id.as_bytes()),
        scopes: vec!["chat".to_string()],
        created_unix_ms: FIRST_EMITTED_UNIX_MS,
        revoked_unix_ms: None,
        created_by: created_by.to_string(),
    }
}

/// The kept tables as a daemon fills them: the embedded node, a plugin the user enabled, a project, and three
/// tokens, one of them made by another that is then revoked, which revokes it too (contracts 8.2).
fn kept_table_writes() -> Vec<IndexWrite> {
    vec![
        IndexWrite::UpsertNode(NodeRow {
            node_id: NODE_ID.to_string(),
            name: "this machine".to_string(),
            transport: "local".to_string(),
            address: String::new(),
            state: "connected".to_string(),
            protocol_version: 1,
            capabilities: vec!["cpu".to_string()],
            last_heartbeat_unix_ms: Some(FIRST_EMITTED_UNIX_MS),
        }),
        IndexWrite::UpsertPlugin(PluginRow {
            plugin_id: PLUGIN_ID.to_string(),
            version: "1.0.0".to_string(),
            state: "enabled".to_string(),
            content_hash: hash_bytes(PLUGIN_ID.as_bytes()),
            granted_permissions: vec!["events.read".to_string()],
            source: "installed".to_string(),
        }),
        IndexWrite::UpsertProject(ProjectRow {
            project_id: PROJECT_ID.to_string(),
            path: "/projects/textcnn".to_string(),
            name: "textcnn".to_string(),
            opened_unix_ms: FIRST_EMITTED_UNIX_MS,
        }),
        IndexWrite::InsertToken(token("tok_01JA2B3C4D5E6F7G8H9J0K1M2A", "owner")),
        IndexWrite::InsertToken(token(
            "tok_01JA2B3C4D5E6F7G8H9J0K1M2B",
            "token:tok_01JA2B3C4D5E6F7G8H9J0K1M2A",
        )),
        IndexWrite::InsertToken(token("tok_01JA2B3C4D5E6F7G8H9J0K1M2C", "owner")),
        IndexWrite::RevokeToken {
            token_id: "tok_01JA2B3C4D5E6F7G8H9J0K1M2A".to_string(),
            revoked_unix_ms: TOKEN_REVOKED_UNIX_MS,
        },
    ]
}

/// What `submit_job` writes when a run starts: its row and its job's, `queued`, then the journal's started row.
fn run_start_writes(run_id: &str, job_id: &str, created_at: &str, journal_id: &str) -> Vec<IndexWrite> {
    let created_unix_ms = parse_rfc3339(created_at).unwrap();
    let record_hash = record_hash_of(run_id);
    let run = RunRow {
        run_id: run_id.to_string(),
        kind: "train".to_string(),
        state: "created".to_string(),
        project_id: String::new(),
        node_id: NODE_ID.to_string(),
        parent_run_id: String::new(),
        record_hash,
        acked_sequence: 0,
        created_unix_ms,
        finished_unix_ms: None,
        pinned: false,
        archive_state: String::new(),
    };
    let job = JobRow {
        job_id: job_id.to_string(),
        run_id: run_id.to_string(),
        node_id: NODE_ID.to_string(),
        kind: "train".to_string(),
        state: "queued".to_string(),
        pid: None,
        process_group_id: None,
        exit_code: None,
        error_code: String::new(),
    };
    let started = JournalStartedRow {
        journal_id: journal_id.to_string(),
        command_id: "train.start".to_string(),
        principal: "owner".to_string(),
        arguments_json: r#"{"recipe":"classification"}"#.to_string(),
        idempotency_key: String::new(),
        started_unix_ms: created_unix_ms - 5,
        arguments_hash: String::new(),
    };
    let finished = JournalFinishedRow {
        journal_id: journal_id.to_string(),
        ok: true,
        code: String::new(),
        result_json: format!(r#"{{"run_id":"{run_id}"}}"#),
        job_id: job_id.to_string(),
        finished_unix_ms: created_unix_ms + 5,
    };
    vec![
        IndexWrite::JournalStarted(started),
        IndexWrite::Batch(vec![IndexWrite::UpsertRun(run), IndexWrite::UpsertJob(job)]),
        IndexWrite::JournalFinished(finished),
    ]
}

/// The content hash the run's committed `run.json` carries, which the `runs` row keeps.
fn record_hash_of(run_id: &str) -> String {
    let bytes = if run_id == RUN_A {
        run_record_bytes(RUN_A, JOB_A, CREATED_AT_A)
    } else {
        run_record_bytes(RUN_B, JOB_B, CREATED_AT_B)
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["content_hash"].as_str().unwrap().to_string()
}

/// Indexes the copied fixture as a daemon would (see the top), and returns every table.
fn index_live(paths: &DataPaths) -> Vec<TableRows> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let store = open_store(paths).unwrap();
    let daemon = daemon_records();
    let mut writes = kept_table_writes();
    writes.extend(run_start_writes(RUN_A, JOB_A, CREATED_AT_A, JOURNAL_A));
    writes.extend(run_start_writes(RUN_B, JOB_B, CREATED_AT_B, JOURNAL_B));
    writes.push(daemon_write(&daemon[..DAEMON_RECORDS_BEFORE_RUN_A]));
    for batch in run_a_records().chunks(LIVE_BATCH_RECORDS) {
        writes.push(IndexWrite::ApplyRunEvents {
            run_id: RUN_A.to_string(),
            records: batch.to_vec(),
        });
    }
    // The whole log again: the records already indexed are skipped by the cursor, as after a restart's replay.
    writes.push(daemon_write(&daemon));
    runtime.block_on(async {
        for write in writes {
            write_index(&store.index, write).await.unwrap();
        }
    });
    let tables = read_tables(&store);
    runtime.block_on(close_store(store));
    tables
}

fn daemon_write(records: &[EventRecord]) -> IndexWrite {
    IndexWrite::ApplyDaemonEvents {
        log_name: DAEMON_LOG_NAME.to_string(),
        records: records.to_vec(),
    }
}

// ---- Reading and comparing indexes ----

fn read_tables(store: &rastai_store::store::Store) -> Vec<TableRows> {
    let reader = open_index_reader(&store.index_readers).unwrap();
    list_every_table_rows(&reader).unwrap()
}

/// Every table of the index in `paths`, opened as the daemon opens it (quick_check, migrations).
fn read_index(paths: &DataPaths) -> Vec<TableRows> {
    let store = open_store(paths).unwrap();
    let tables = read_tables(&store);
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(close_store(store));
    tables
}

fn rows_of<'a>(tables: &'a [TableRows], table_name: &str) -> &'a Vec<Vec<String>> {
    &tables.iter().find(|table| table.table_name == table_name).unwrap().rows
}

/// Every rebuildable table of `rebuilt` equals `live` row by row; the tables a rebuild starts empty are empty.
fn assert_rebuilt_matches(live: &[TableRows], rebuilt: &[TableRows]) {
    assert_eq!(rebuilt_table_differences(live, rebuilt), Vec::<String>::new());
}

fn codes(report: &RebuildReport) -> Vec<String> {
    report.diagnostics.iter().map(|item| item.code.clone()).collect()
}

/// A run's acked sequence as the rebuilt index has it.
fn acked_sequence(tables: &[TableRows], run_id: &str) -> String {
    let quoted = format!("{run_id:?}");
    let run = rows_of(tables, "runs").iter().find(|row| row[0] == quoted).unwrap();
    // Column 7 is acked_sequence (contracts 8.2).
    run[7].clone()
}

fn rebuild(paths: &DataPaths) -> RebuildReport {
    rebuild_index(paths, &RebuildOptions::default()).unwrap()
}

// ---- The tests ----

#[test]
fn a_rebuilt_index_matches_the_live_one_row_by_row() {
    let paths = test_paths("rebuild-matches");
    copy_fixture(&paths);
    let live = index_live(&paths);
    // Every kind of row the fixture makes is there, so the comparison below covers each.
    for table_name in [
        "runs",
        "jobs",
        "run_metrics",
        "run_tags",
        "checkpoints",
        "artifacts",
        "lineage",
    ] {
        assert!(
            !rows_of(&live, table_name).is_empty(),
            "the live index has no {table_name}"
        );
    }

    let report = rebuild(&paths);

    assert_eq!(codes(&report), Vec::<String>::new());
    assert_eq!((report.runs, report.artifacts), (2, 1));
    assert_eq!(report.records, (run_a_records().len() + daemon_records().len()) as u64);
    let rebuilt = read_index(&paths);
    assert_rebuilt_matches(&live, &rebuilt);
    // The revoked token and the one it made stay revoked, and the plugin keeps the user's decision.
    assert_eq!(rows_of(&rebuilt, "tokens"), rows_of(&live, "tokens"));
    assert_eq!(rows_of(&rebuilt, "plugins")[0][2], "\"enabled\"");
    // The old index is kept beside the new one, until the user deletes it.
    let old_index = report.old_index_path.unwrap();
    assert!(old_index.exists());
    assert!(!rastai_db_rebuilding_path(&paths).exists());
    remove_test_folder(&paths);
}

#[test]
fn a_corrupt_frame_ends_that_runs_log_with_rrs_s2002_and_the_rest_is_rebuilt() {
    let paths = test_paths("rebuild-corrupt");
    copy_fixture(&paths);
    index_live(&paths);
    let log_path = run_events_log_path(&paths, RUN_A);
    damage_frame(&log_path, CORRUPTED_RUN_A_RECORD);

    let report = rebuild(&paths);

    assert_eq!(codes(&report), ["RRS-S2002"]);
    let rebuilt = read_index(&paths);
    // The records before the damage are indexed; the run goes on to be rebuilt from the daemon log.
    assert_eq!(acked_sequence(&rebuilt, RUN_A), CORRUPTED_RUN_A_RECORD.to_string());
    assert_eq!(rows_of(&rebuilt, "checkpoints").len(), 0);
    assert_eq!(rows_of(&rebuilt, "jobs").len(), 2);
    assert_eq!(rows_of(&rebuilt, "run_tags").len(), 2);
    remove_test_folder(&paths);
}

/// Flips one byte in the body of the record at `index`, so whole frames follow the bad one.
fn damage_frame(log_path: &Path, index: usize) {
    let mut offset = EVENT_LOG_HEADER.len();
    for record in &run_a_records()[..index] {
        offset += encode_frame(record).unwrap().len();
    }
    let mut bytes = std::fs::read(log_path).unwrap();
    // Past the frame's 8-byte header, inside its body.
    bytes[offset + 12] ^= 0x40;
    std::fs::write(log_path, bytes).unwrap();
}

#[test]
fn a_record_the_runs_log_may_not_carry_ends_it_with_rrs_s2004() {
    let paths = test_paths("rebuild-refused");
    copy_fixture(&paths);
    // Run B's job writes a log after all: its second record names run A, which a run's log may not do.
    let records = vec![
        record(1, RUN_B, JOB_B, "run.state_changed", run_state(RunState::Running)),
        record(2, RUN_A, JOB_B, "run.metrics", metrics(9, 0.5)),
        record(3, RUN_B, JOB_B, "run.metrics", metrics(1, 0.75)),
    ];
    std::fs::write(run_events_log_path(&paths, RUN_B), log_bytes(&records)).unwrap();

    let report = rebuild(&paths);

    assert_eq!(codes(&report), ["RRS-S2004"]);
    let rebuilt = read_index(&paths);
    assert_eq!(acked_sequence(&rebuilt, RUN_B), "1");
    let step_nine = rows_of(&rebuilt, "run_metrics").iter().any(|row| row[2] == "9");
    assert!(!step_nine, "the refused record's metrics reached run A");
    assert_eq!(acked_sequence(&rebuilt, RUN_A), run_a_records().len().to_string());
    remove_test_folder(&paths);
}

#[test]
fn an_artifact_whose_blob_is_gone_is_dropped_with_rrs_s1002() {
    let paths = test_paths("rebuild-missing-blob");
    copy_fixture(&paths);
    // The live index holds the embedded node, which says the artifact's home is this machine.
    index_live(&paths);
    std::fs::remove_file(paths.data.join(blob_relative_path(EXPORTED_BYTES))).unwrap();

    let report = rebuild(&paths);

    assert_eq!(codes(&report), ["RRS-S1002"]);
    assert_eq!(report.artifacts, 0);
    let rebuilt = read_index(&paths);
    assert!(rows_of(&rebuilt, "artifacts").is_empty());
    assert!(rows_of(&rebuilt, "artifact_copies").is_empty());
    // The checkpoint's own lineage row stays; the export's, whose child is the dropped artifact, goes.
    let relations: Vec<&String> = rows_of(&rebuilt, "lineage").iter().map(|row| &row[2]).collect();
    assert_eq!(relations, ["\"checkpoint_of\""]);
    remove_test_folder(&paths);
}

#[test]
fn verify_blobs_quarantines_a_changed_blob_and_drops_its_rows() {
    let paths = test_paths("rebuild-verify");
    copy_fixture(&paths);
    index_live(&paths);
    let blob_path = paths.data.join(blob_relative_path(EXPORTED_BYTES));
    std::fs::write(&blob_path, b"bit rot").unwrap();
    let options = RebuildOptions {
        verify_blobs: true,
        ..RebuildOptions::default()
    };

    let report = rebuild_index(&paths, &options).unwrap();

    assert_eq!(codes(&report), ["RRS-S1001"]);
    assert!(!blob_path.exists(), "the changed blob wasn't quarantined");
    assert!(blobs_dir(&paths).join(".quarantine").exists());
    assert!(rows_of(&read_index(&paths), "artifacts").is_empty());
    remove_test_folder(&paths);
}

#[test]
fn an_old_index_that_isnt_a_database_leaves_the_kept_tables_empty() {
    let paths = test_paths("rebuild-unreadable");
    copy_fixture(&paths);
    std::fs::write(rastai_db_path(&paths), vec![0x5a; 8192]).unwrap();

    let report = rebuild(&paths);

    assert_eq!(codes(&report), ["RRS-S0003"]);
    let rebuilt = read_index(&paths);
    // Fail closed: no tokens, no nodes, so no artifact has a home here to check; the runs come back from the logs.
    assert!(rows_of(&rebuilt, "tokens").is_empty());
    assert!(rows_of(&rebuilt, "nodes").is_empty());
    assert_eq!(rows_of(&rebuilt, "runs").len(), 2);
    assert_eq!(std::fs::read(report.old_index_path.unwrap()).unwrap(), vec![0x5a; 8192]);
    remove_test_folder(&paths);
}

#[test]
fn a_data_folder_without_an_index_gets_one_and_what_a_killed_rebuild_left_goes() {
    let paths = test_paths("rebuild-fresh");
    copy_fixture(&paths);
    let leftover = rastai_db_rebuilding_path(&paths);
    std::fs::write(&leftover, b"half of a killed rebuild's index").unwrap();

    let report = rebuild(&paths);

    assert_eq!(report.old_index_path, None);
    assert_eq!(report.runs, 2);
    assert!(!leftover.exists());
    assert_eq!(rows_of(&read_index(&paths), "runs").len(), 2);
    remove_test_folder(&paths);
}

#[test]
fn a_run_json_that_names_another_run_is_left_out() {
    let paths = test_paths("rebuild-foreign-record");
    copy_fixture(&paths);
    let run_b_record = run_dir(&paths, RUN_B).join("run.json");
    std::fs::write(&run_b_record, run_record_bytes(RUN_A, JOB_B, CREATED_AT_B)).unwrap();

    let report = rebuild(&paths);

    // The value the rebuild can't use, at its place in the file, as a document check reports it.
    assert_eq!(codes(&report), ["RRS-E2006"]);
    let expected_path = format!("{}#/run_id", run_b_record.display());
    assert_eq!(report.diagnostics[0].path, expected_path);
    let rebuilt = read_index(&paths);
    assert_eq!(rows_of(&rebuilt, "runs").len(), 1);
    // Run A's row is its own run.json's, not the copy's in run B's folder.
    assert_eq!(rows_of(&rebuilt, "runs")[0][6], format!("{:?}", record_hash_of(RUN_A)));
    remove_test_folder(&paths);
}

#[test]
fn a_job_spec_that_names_another_runs_job_leaves_that_job_out_with_rrs_e2006() {
    let paths = test_paths("rebuild-foreign-job");
    copy_fixture(&paths);
    // Run B's run.json claims run A's job, which run A's own run.json (read first, in id order) already holds.
    let run_b_record = run_dir(&paths, RUN_B).join("run.json");
    std::fs::write(&run_b_record, run_record_bytes(RUN_B, JOB_A, CREATED_AT_B)).unwrap();

    let report = rebuild(&paths);

    assert_eq!(codes(&report), ["RRS-E2006"]);
    let expected_path = format!("{}#/job_spec/job_id", run_b_record.display());
    assert_eq!(report.diagnostics[0].path, expected_path);
    let rebuilt = read_index(&paths);
    // Run B keeps its row; run A's job stays run A's.
    assert_eq!(rows_of(&rebuilt, "runs").len(), 2);
    let job_a = rows_of(&rebuilt, "jobs")
        .iter()
        .find(|row| row[0] == format!("{JOB_A:?}"))
        .unwrap();
    assert_eq!(job_a[1], format!("{RUN_A:?}"));
    remove_test_folder(&paths);
}
