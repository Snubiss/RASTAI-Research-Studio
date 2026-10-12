//! The `rastai.db` writer thread (layout rust-control-plane.md sections 6.3 and 6.7): 256 concurrent writers commit
//! without convoying (a p99 bound, spike 07), one bad write fails alone, a `Batch` lands whole or not at all,
//! `close_store` commits what was queued, and every write reads back through the reader pool.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use rastai_store::index::documents::{DocumentRow, UndoStepRow, get_document, list_undo_steps};
use rastai_store::index::journal::{
    JournalFinishedRow, JournalStartedRow, find_journal_by_idempotency_key, get_journal,
};
use rastai_store::index::kept_tables::{
    NodeRow, PluginRow, ProjectRow, TokenRow, find_token_by_hash, get_node, get_plugin, get_project, list_tokens,
};
use rastai_store::index::reads::open_index_reader;
use rastai_store::index::runs::{JobRow, RunFilter, RunRow, get_job, get_run, list_runs};
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, close_store, open_store};
use rastai_store::writer_thread::{IndexWriter, write_index, writer_batch_count};
use tokio::runtime::Runtime;

mod common;
use common::{remove_test_folder, test_paths};

// The spike's load (07): 256 clients at once, each writing one after another.
const CONCURRENT_WRITERS: usize = 256;
const WRITES_PER_WRITER: usize = 20;
// One writer thread commits a whole queue at once, so a write waits for about one transaction, not 255. Locally
// the p99 is about 4 ms in a debug build; the bound leaves room for slow CI runners.
const P99_BOUND: Duration = Duration::from_millis(250);
const PERCENTILE: usize = 99;
// The convoy's signature: every write pays for its own transaction, one after another. Batching shares one
// transaction among the writes already queued: under this load a batch carries 130 to 240 writes on average
// (measured on 1, 2, 4 and 24 cores). At one write per batch the writes have convoyed.
const MINIMUM_WRITES_PER_BATCH: u64 = 8;

fn multi_thread_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread().build().unwrap()
}

fn run_row(run_id: &str, created_unix_ms: i64) -> RunRow {
    RunRow {
        run_id: run_id.to_string(),
        kind: "train".to_string(),
        state: "running".to_string(),
        project_id: "prj_a".to_string(),
        node_id: "node_local".to_string(),
        parent_run_id: String::new(),
        record_hash: "blake3:00".to_string(),
        acked_sequence: 0,
        created_unix_ms,
        finished_unix_ms: None,
        pinned: false,
        archive_state: "local".to_string(),
    }
}

fn token_row(token_id: &str) -> TokenRow {
    TokenRow {
        token_id: token_id.to_string(),
        name: "script".to_string(),
        token_hash: format!("blake3:{token_id}"),
        scopes: vec!["read".to_string(), "plugin:acme".to_string()],
        created_unix_ms: 10,
        revoked_unix_ms: None,
    }
}

fn document_row(document_id: &str) -> DocumentRow {
    DocumentRow {
        document_id: document_id.to_string(),
        document_type: "rastai.model/1".to_string(),
        path: "models/tiny.rrsmodel".to_string(),
        project_id: "prj_a".to_string(),
        content_hash: "blake3:01".to_string(),
        updated_unix_ms: 20,
    }
}

fn undo_step(document_id: &str, position: u32) -> UndoStepRow {
    UndoStepRow {
        document_id: document_id.to_string(),
        position,
        forward_patch_json: format!("[{{\"op\":\"add\",\"path\":\"/n\",\"value\":{position}}}]"),
        inverse_patch_json: "[{\"op\":\"remove\",\"path\":\"/n\"}]".to_string(),
        journal_id: format!("jrn_{position}"),
    }
}

/// One writer's run of writes, timing each from send to commit.
async fn write_runs_and_time_each(index: IndexWriter, writer_number: usize) -> Vec<Duration> {
    let mut latencies = Vec::with_capacity(WRITES_PER_WRITER);
    for write_number in 0..WRITES_PER_WRITER {
        let run_id = format!("run_{writer_number}_{write_number}");
        let started = Instant::now();
        write_index(&index, IndexWrite::UpsertRun(run_row(&run_id, 1)))
            .await
            .unwrap();
        latencies.push(started.elapsed());
    }
    latencies
}

fn percentile(mut latencies: Vec<Duration>, percent: usize) -> Duration {
    latencies.sort();
    let index = (latencies.len() * percent / 100).min(latencies.len() - 1);
    latencies[index]
}

fn write(runtime: &Runtime, store: &Store, write: IndexWrite) {
    runtime.block_on(write_index(&store.index, write)).unwrap();
}

/// 256 writers at once; returns every write's latency.
fn write_concurrently(runtime: &Runtime, store: &Store) -> Vec<Duration> {
    runtime.block_on(async {
        let mut tasks = Vec::with_capacity(CONCURRENT_WRITERS);
        for writer_number in 0..CONCURRENT_WRITERS {
            tasks.push(tokio::spawn(write_runs_and_time_each(
                store.index.clone(),
                writer_number,
            )));
        }
        let mut latencies = Vec::with_capacity(CONCURRENT_WRITERS * WRITES_PER_WRITER);
        for task in tasks {
            latencies.extend(task.await.unwrap());
        }
        latencies
    })
}

#[test]
fn concurrent_writers_commit_without_convoying() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("convoy");
    let store = open_store(&paths).unwrap();
    let batches_before = writer_batch_count(&store.index);
    let latencies = write_concurrently(&runtime, &store);
    let batches = writer_batch_count(&store.index) - batches_before;
    let reader = open_index_reader(&store.index_readers).unwrap();
    let runs = list_runs(&reader, &RunFilter::default()).unwrap();
    drop(reader);
    runtime.block_on(close_store(store));

    let write_count = latencies.len();
    let p99 = percentile(latencies, PERCENTILE);
    assert_eq!(runs.len(), write_count);
    assert!(p99 <= P99_BOUND, "p99 write latency {p99:?} is above {P99_BOUND:?}");
    assert!(
        batches * MINIMUM_WRITES_PER_BATCH <= write_count as u64,
        "{write_count} writes took {batches} transactions: the writes are not batched"
    );
    remove_test_folder(&paths);
}

#[test]
fn one_bad_write_among_concurrent_writes_fails_alone() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("bad-write");
    let store = open_store(&paths).unwrap();
    // An undo step for a document that doesn't exist breaks its foreign key; the writes around it are good.
    let outcomes = runtime.block_on(async {
        let mut tasks = Vec::new();
        for number in 0..32 {
            let write = if number == 16 {
                IndexWrite::RecordUndoStep(undo_step("doc_missing", 1))
            } else {
                IndexWrite::UpsertRun(run_row(&format!("run_{number}"), 1))
            };
            let index = store.index.clone();
            tasks.push(tokio::spawn(async move { write_index(&index, write).await }));
        }
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.await.unwrap());
        }
        outcomes
    });
    let reader = open_index_reader(&store.index_readers).unwrap();
    let runs = list_runs(&reader, &RunFilter::default()).unwrap();
    drop(reader);
    runtime.block_on(close_store(store));

    for (number, outcome) in outcomes.iter().enumerate() {
        if number == 16 {
            assert_eq!(outcome.as_ref().unwrap_err().code(), "RRS-S3005");
        } else {
            assert!(outcome.is_ok(), "write {number} failed: {outcome:?}");
        }
    }
    assert_eq!(runs.len(), 31);
    remove_test_folder(&paths);
}

#[test]
fn a_batch_lands_whole_or_not_at_all() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("batch");
    let store = open_store(&paths).unwrap();
    let good = IndexWrite::Batch(vec![
        IndexWrite::UpsertDocument(document_row("doc_a")),
        IndexWrite::RecordUndoStep(undo_step("doc_a", 1)),
    ]);
    write(&runtime, &store, good);
    // The second write of this batch breaks a foreign key, so its first write is undone too.
    let bad = IndexWrite::Batch(vec![
        IndexWrite::UpsertDocument(document_row("doc_b")),
        IndexWrite::RecordUndoStep(undo_step("doc_missing", 1)),
    ]);
    let error = runtime.block_on(write_index(&store.index, bad)).unwrap_err();
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert_eq!(error.code(), "RRS-S3005");
    assert!(get_document(&reader, "doc_a").unwrap().is_some());
    assert_eq!(list_undo_steps(&reader, "doc_a").unwrap().len(), 1);
    assert_eq!(get_document(&reader, "doc_b").unwrap(), None);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn close_store_commits_every_queued_write() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("close");
    let store = open_store(&paths).unwrap();
    let index = store.index.clone();
    runtime.block_on(async {
        let mut tasks = Vec::new();
        for number in 0..64 {
            let index = store.index.clone();
            let run_id = format!("run_{number}");
            tasks.push(tokio::spawn(async move {
                write_index(&index, IndexWrite::UpsertRun(run_row(&run_id, 1))).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
    });
    drop(index);
    runtime.block_on(close_store(store));

    let store = open_store(&paths).unwrap();
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert_eq!(list_runs(&reader, &RunFilter::default()).unwrap().len(), 64);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn runs_and_jobs_read_back_and_the_ack_never_moves_back() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("runs");
    let store = open_store(&paths).unwrap();
    let mut run = run_row("run_a", 5);
    run.acked_sequence = 40;
    write(&runtime, &store, IndexWrite::UpsertRun(run.clone()));
    write(&runtime, &store, IndexWrite::UpsertRun(run_row("run_b", 6)));
    // An upsert from an older copy of the row (ack 3) finishes the run but keeps the ack at 40.
    let mut finished = run_row("run_a", 5);
    finished.state = "completed".to_string();
    finished.acked_sequence = 3;
    finished.finished_unix_ms = Some(9);
    write(&runtime, &store, IndexWrite::UpsertRun(finished));
    let job = JobRow {
        job_id: "job_a".to_string(),
        run_id: "run_a".to_string(),
        node_id: "node_local".to_string(),
        kind: "train".to_string(),
        state: "succeeded".to_string(),
        pid: Some(4242),
        process_group_id: Some(4242),
        exit_code: Some(0),
        error_code: String::new(),
    };
    write(&runtime, &store, IndexWrite::UpsertJob(job.clone()));

    let reader = open_index_reader(&store.index_readers).unwrap();
    let stored = get_run(&reader, "run_a").unwrap().unwrap();
    let completed = RunFilter {
        state: "completed".to_string(),
        ..RunFilter::default()
    };
    let newest_one = RunFilter {
        limit: Some(1),
        ..RunFilter::default()
    };
    assert_eq!(stored.state, "completed");
    assert_eq!(stored.acked_sequence, 40);
    assert_eq!(stored.finished_unix_ms, Some(9));
    assert_eq!(stored.archive_state, "local");
    assert_eq!(list_runs(&reader, &completed).unwrap().len(), 1);
    assert_eq!(list_runs(&reader, &newest_one).unwrap()[0].run_id, "run_b");
    assert_eq!(get_job(&reader, "job_a").unwrap(), Some(job));
    assert_eq!(get_run(&reader, "run_missing").unwrap(), None);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn journal_rows_are_found_by_key_and_unfinished_ones_are_closed() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("journal");
    let store = open_store(&paths).unwrap();
    let started = |journal_id: &str, key: &str, started_unix_ms: i64| JournalStartedRow {
        journal_id: journal_id.to_string(),
        command_id: "train.start".to_string(),
        principal: "owner".to_string(),
        arguments_json: "{}".to_string(),
        idempotency_key: key.to_string(),
        started_unix_ms,
    };
    write(
        &runtime,
        &store,
        IndexWrite::JournalStarted(started("jrn_a", "key-1", 1)),
    );
    write(
        &runtime,
        &store,
        IndexWrite::JournalStarted(started("jrn_b", "key-1", 2)),
    );
    write(&runtime, &store, IndexWrite::JournalStarted(started("jrn_c", "", 3)));
    write(&runtime, &store, IndexWrite::MarkUnfinishedJournalRows);

    let reader = open_index_reader(&store.index_readers).unwrap();
    let newest = find_journal_by_idempotency_key(&reader, "owner", "key-1")
        .unwrap()
        .unwrap();
    let interrupted = get_journal(&reader, "jrn_c").unwrap().unwrap();
    assert_eq!(newest.journal_id, "jrn_b");
    assert_eq!(
        find_journal_by_idempotency_key(&reader, "token:tok_a", "key-1").unwrap(),
        None
    );
    assert_eq!(find_journal_by_idempotency_key(&reader, "owner", "").unwrap(), None);
    assert_eq!(interrupted.ok, Some(false));
    assert_eq!(interrupted.code, "RRS-C0005");
    assert!(interrupted.finished_unix_ms.is_some());
    assert_eq!(interrupted.idempotency_key, "");
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_finished_row_keeps_its_result_and_its_first_finish() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("journal-finished");
    let store = open_store(&paths).unwrap();
    let started = JournalStartedRow {
        journal_id: "jrn_a".to_string(),
        command_id: "token.create".to_string(),
        principal: "owner".to_string(),
        arguments_json: "{\"name\":\"ci\"}".to_string(),
        idempotency_key: "key-1".to_string(),
        started_unix_ms: 1,
    };
    let finished = |result_json: &str, finished_unix_ms: i64| JournalFinishedRow {
        journal_id: "jrn_a".to_string(),
        ok: true,
        code: String::new(),
        result_json: result_json.to_string(),
        job_id: String::new(),
        finished_unix_ms,
    };
    write(&runtime, &store, IndexWrite::JournalStarted(started));
    write(
        &runtime,
        &store,
        IndexWrite::JournalFinished(finished("{\"token\":\"<redacted>\"}", 2)),
    );
    // A second finish for the same row changes nothing: the command ran once.
    write(&runtime, &store, IndexWrite::JournalFinished(finished("{}", 3)));
    // A restart's sweep leaves a finished row alone.
    write(&runtime, &store, IndexWrite::MarkUnfinishedJournalRows);

    let reader = open_index_reader(&store.index_readers).unwrap();
    let row = get_journal(&reader, "jrn_a").unwrap().unwrap();
    assert_eq!(row.ok, Some(true));
    assert_eq!(row.code, "");
    assert_eq!(row.result_json, "{\"token\":\"<redacted>\"}");
    assert_eq!(row.job_id, "");
    assert_eq!(row.finished_unix_ms, Some(2));
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn a_new_undo_step_drops_the_redo_steps_above_it() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("undo");
    let store = open_store(&paths).unwrap();
    write(&runtime, &store, IndexWrite::UpsertDocument(document_row("doc_a")));
    for position in 1..=3 {
        write(
            &runtime,
            &store,
            IndexWrite::RecordUndoStep(undo_step("doc_a", position)),
        );
    }
    // Undone twice, then edited: the new step takes position 2 and the old 2 and 3 go.
    write(&runtime, &store, IndexWrite::RecordUndoStep(undo_step("doc_a", 2)));
    let reader = open_index_reader(&store.index_readers).unwrap();
    let positions: Vec<u32> = list_undo_steps(&reader, "doc_a")
        .unwrap()
        .iter()
        .map(|step| step.position)
        .collect();
    assert_eq!(positions, vec![1, 2]);
    assert_eq!(get_document(&reader, "doc_a").unwrap(), Some(document_row("doc_a")));
    drop(reader);

    let clear = IndexWrite::ClearUndoSteps {
        document_id: "doc_a".to_string(),
    };
    write(&runtime, &store, clear);
    let reader = open_index_reader(&store.index_readers).unwrap();
    assert!(list_undo_steps(&reader, "doc_a").unwrap().is_empty());
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}

#[test]
fn the_kept_tables_read_back() {
    let runtime = multi_thread_runtime();
    let paths = test_paths("kept");
    let store = open_store(&paths).unwrap();
    let project = ProjectRow {
        project_id: "prj_a".to_string(),
        path: "/home/user/projects/a".to_string(),
        name: "A".to_string(),
        opened_unix_ms: 7,
    };
    let node = NodeRow {
        node_id: "node_local".to_string(),
        name: "this machine".to_string(),
        transport: "local".to_string(),
        address: String::new(),
        state: "connected".to_string(),
        protocol_version: 1,
        capabilities: vec!["generate.inspectable".to_string(), "lens.live".to_string()],
        last_heartbeat_unix_ms: Some(8),
    };
    let plugin = PluginRow {
        plugin_id: "rastai.psychology".to_string(),
        version: "0.1.0".to_string(),
        state: "enabled".to_string(),
        content_hash: "blake3:02".to_string(),
        granted_permissions: vec!["gpu".to_string()],
        source: "preinstalled".to_string(),
    };
    write(&runtime, &store, IndexWrite::UpsertProject(project.clone()));
    write(&runtime, &store, IndexWrite::UpsertNode(node.clone()));
    write(&runtime, &store, IndexWrite::UpsertPlugin(plugin.clone()));
    write(&runtime, &store, IndexWrite::InsertToken(token_row("tok_a")));
    let revoke = |revoked_unix_ms| IndexWrite::RevokeToken {
        token_id: "tok_a".to_string(),
        revoked_unix_ms,
    };
    write(&runtime, &store, revoke(11));
    // Revoking again keeps the first time.
    write(&runtime, &store, revoke(12));

    let reader = open_index_reader(&store.index_readers).unwrap();
    let token = find_token_by_hash(&reader, "blake3:tok_a").unwrap().unwrap();
    assert_eq!(get_project(&reader, "prj_a").unwrap(), Some(project));
    assert_eq!(get_node(&reader, "node_local").unwrap(), Some(node));
    assert_eq!(get_plugin(&reader, "rastai.psychology").unwrap(), Some(plugin));
    assert_eq!(token.scopes, token_row("tok_a").scopes);
    assert_eq!(token.revoked_unix_ms, Some(11));
    assert_eq!(list_tokens(&reader).unwrap(), vec![token]);
    drop(reader);
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}
