//! The offline rebuild of `rastai.db` (layout rust-control-plane.md 13.2, flows/platform.md 6.4): what
//! `rastai storage rebuild-index` runs, the fix for `RRS-S0003`. The caller holds `daemon.lock`, so no daemon writes
//! the index or the logs meanwhile.
//!
//! - **Same code as live indexing.** Run logs go through `apply_run_events` (the checks of `ApplyRunEvents`, then
//!   `apply_event_record`) and the daemon log through `apply_daemon_events`, so rebuilt rows can't differ from live
//!   ones. The kept tables (`tokens`, `nodes`, `plugins`, `projects`) are copied from the old file, whatever of them
//!   it still reads; a table it can't read starts empty, which fails closed (contracts 8.2).
//! - **Crash-safe.** The new index is written to `rastai.db.rebuilding` and only takes the old one's place once it
//!   is whole and has passed `quick_check`. The old file is linked to `rastai.db.broken-<unix_ms>` first and stays
//!   at `rastai.db` until the rename, so a kill at any point leaves a usable index: the old one, or the new one. A
//!   rebuild started again removes what a killed one left.
//! - **Reading is tolerant.** A torn tail ends a log; a corrupt frame (`RRS-S2002`), a sequence gap (`RRS-S2003`)
//!   or a record a run's log may not carry (`RRS-S2004`) ends it with that code in the report, and the rebuild goes
//!   on with the next log. A `run.json` that can't be read leaves its run out, with the reason.
//! - **Blobs.** An artifact whose home is this machine and whose blob is gone is dropped (`RRS-S1002`); with
//!   `verify_blobs` each such blob is hashed again and a mismatch quarantined first (`RRS-S1001`).
//!
//! What the sources don't hold, the rebuild can't bring back (contracts 8.2 and 8.4):
//! - `runs.project_id` is "": `run.json` doesn't record the project. Every run folder is the embedded node's, whose
//!   id comes from the copied `nodes` table, since `run.json` doesn't record the node either and only the embedded
//!   node runs jobs until remote nodes (M9).
//! - A job's row comes from `run.json`'s `job_spec` (id, run, kind, `queued`, as `submit_job` writes it), then from
//!   the daemon log's `job.state_changed`. The process numbers aren't in any log; the daemon never writes them yet.
//! - A journal row comes from `command.finished`: its arguments, key, result and job, which only the started and
//!   finished writes carry, are not rebuilt, and a command that never finished has no row.
//! - `undo_steps`, `transfers`, `plugin_event_deliveries` start empty (13.2). The documents scan of the projects
//!   (M4), the HF cache scan for `models` (M2), `archive.pruned` stubs (M11) and copies from `transfer.finished` (M9)
//!   arrive with the milestones that write those rows live.

use std::fs::File;
use std::path::{Path, PathBuf};

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::paths::{
    DataPaths, blobs_dir, rastai_db_broken_path, rastai_db_path, rastai_db_rebuilding_path, runs_dir,
};
use rastai_core::proto_json::job_spec_from_json;
use rastai_core::states::{job_kind_name, job_state_name};
use rastai_core::time::{now_unix_ms, parse_rfc3339};
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::documents::Document;
use rastai_formats::documents::run_record::RunRecord;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::read_document;
use rastai_proto::common::v1::JobState;
use rastai_proto::events::v1::EventRecord;

use crate::blobs::blob_store::{BlobStore, has_blob, verify_blob};
use crate::error::{StoreError, store_error_diagnostic};
use crate::event_log::daemon_log::daemon_log_segments;
use crate::event_log::reader::{EventLogReader, FrameRead, open_event_log, read_next_frame};
use crate::index::artifacts::{delete_artifact, list_artifact_blobs};
use crate::index::kept_tables::{
    NodeRow, PluginRow, ProjectRow, TokenRow, insert_token, list_nodes, list_plugins, list_projects, list_tokens,
    upsert_node, upsert_plugin, upsert_project,
};
use crate::index::reads::IndexReader;
use crate::index::runs::{JobRow, RunRow, read_job_run_id, upsert_job, upsert_run};
use crate::indexer::{apply_daemon_events, apply_run_events};
use crate::sqlite::{
    SqliteConnection, SqliteTransaction, begin_transaction, commit_transaction, finish_database_file,
    open_database_for_salvage, open_database_for_writing, run_in_savepoint,
};
use crate::store::{INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS};

/// Records per savepoint, the live tail's batch size (contracts 15.10), so a refused record undoes as much here as
/// it does live before the records ahead of it are applied again.
const REBUILD_BATCH_RECORDS: usize = 256;
const BATCH_SAVEPOINT: &str = "rebuild_batch";
const KEPT_ROW_SAVEPOINT: &str = "rebuild_kept_row";
/// `runs/<run_id>/run.json` and `events.log` (contracts 8.4).
const RUN_RECORD_FILE: &str = "run.json";
const RUN_EVENTS_FILE: &str = "events.log";
/// `runs.state` of a run whose job hasn't written a record yet (contracts 9.5), as `submit_job` writes the row.
const RUN_CREATED_STATE: &str = "created";
/// `nodes.transport` of the embedded node, whose folders the run folders are (see the top).
const LOCAL_TRANSPORT: &str = "local";
/// SQLite's companions of a database file in WAL mode: they belong to the file of the same name.
const WAL_FILE_SUFFIX: &str = "-wal";
const SHARED_MEMORY_FILE_SUFFIX: &str = "-shm";
/// `RRS-S0006`, "Couldn't read saved data": a `run.json` the rebuild can't use.
const CODE_READ_FAILED: &str = "RRS-S0006";
/// The test-only points `RebuildOptions.pause_at` may name (contracts 15.9): the kill -9 test stops the rebuild
/// there and kills it.
pub const PAUSE_AFTER_RUN_FOLDERS: &str = "store.rebuild.after_run_folders";
pub const PAUSE_BEFORE_SWAP: &str = "store.rebuild.before_swap";
pub const PAUSE_BEFORE_RENAME: &str = "store.rebuild.before_rename";
/// The procps tool that sends this process SIGSTOP without `unsafe` code, as the node's signals do.
const KILL_TOOL: &str = "kill";
const STOP_SIGNAL: &str = "STOP";

/// Columns of one table that no durable source holds (see the top): a rebuild writes "" or NULL there, or for
/// `started_unix_ms` the finish time. Every other column of a rebuilt table equals the live index's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnsNotRebuilt {
    pub table_name: &'static str,
    pub column_names: &'static [&'static str],
}

/// The columns a rebuild can't bring back. `schema_migrations.applied_unix_ms` is when the new file was migrated.
/// `tests/rebuild_index.rs` (here and in rastai-cli) compares every other column with the live index's.
pub const COLUMNS_NOT_REBUILT: &[ColumnsNotRebuilt] = &[
    ColumnsNotRebuilt {
        table_name: "command_journal",
        column_names: &[
            "arguments_json",
            "idempotency_key",
            "result_json",
            "job_id",
            "started_unix_ms",
            "arguments_hash",
        ],
    },
    ColumnsNotRebuilt {
        table_name: "runs",
        column_names: &["project_id"],
    },
    ColumnsNotRebuilt {
        table_name: "schema_migrations",
        column_names: &["applied_unix_ms"],
    },
];

/// The tables a rebuild starts empty (13.2).
pub const TABLES_NOT_REBUILT: &[&str] = &["plugin_event_deliveries", "transfers", "undo_steps"];

/// What the caller asks of a rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildOptions {
    /// `--verify-blobs`: hash every artifact's blob again; a mismatch is quarantined and its rows dropped.
    pub verify_blobs: bool,
    /// Tests only: the `PAUSE_*` point where the rebuild stops itself with SIGSTOP, so a kill lands there. The CLI
    /// fills it from `RASTAI_TEST_PAUSE_AT` in debug builds only; "" never pauses.
    pub pause_at: String,
}

/// What a rebuild did. `diagnostics` holds every problem it passed over, each with its catalog code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildReport {
    /// Run folders whose `run.json` was read and whose rows were rebuilt.
    pub runs: u64,
    /// Event records applied, from the run logs and the daemon log.
    pub records: u64,
    /// Artifacts in the rebuilt index, after those whose blobs are gone were dropped.
    pub artifacts: u64,
    /// Where the old index was kept (`rastai.db.broken-<unix_ms>`); `None` when there was none.
    pub old_index_path: Option<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The four tables a rebuild copies from the old file (13.2).
#[derive(Debug, Default)]
struct KeptRows {
    tokens: Vec<TokenRow>,
    nodes: Vec<NodeRow>,
    plugins: Vec<PluginRow>,
    projects: Vec<ProjectRow>,
}

/// Rebuilds `rastai.db` from the run folders, the daemon log and the blob store, keeping the four tables the logs
/// can't rebuild from the old file (see the top). The caller holds `daemon.lock`. An error leaves the old index in
/// place and removes the half-written new one.
pub fn rebuild_index(paths: &DataPaths, options: &RebuildOptions) -> Result<RebuildReport, StoreError> {
    std::fs::create_dir_all(&paths.data).map_err(|error| not_writable(&paths.data, &error))?;
    let rebuilding_path = rastai_db_rebuilding_path(paths);
    remove_database_files(&rebuilding_path)?;
    let mut report = RebuildReport::default();
    if let Err(error) = write_rebuilt_index(paths, &rebuilding_path, options, &mut report) {
        let _ = remove_database_files(&rebuilding_path);
        return Err(error);
    }
    pause_if_asked(options, PAUSE_BEFORE_SWAP);
    report.old_index_path = swap_in_rebuilt_index(paths, &rebuilding_path, options)?;
    Ok(report)
}

/// Writes the whole new index into `rebuilding_path`, checked and in one file.
fn write_rebuilt_index(
    paths: &DataPaths,
    rebuilding_path: &Path,
    options: &RebuildOptions,
    report: &mut RebuildReport,
) -> Result<(), StoreError> {
    let fresh = open_database_for_writing(rebuilding_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS)?;
    let kept = read_kept_tables(paths, report)?;
    let home_node_id = embedded_node_id(&kept.nodes);
    write_kept_tables(&fresh, &kept, report)?;
    rebuild_from_run_folders(paths, &fresh, &home_node_id, report)?;
    pause_if_asked(options, PAUSE_AFTER_RUN_FOLDERS);
    rebuild_from_daemon_log(paths, &fresh, report)?;
    drop_rows_with_missing_blobs(paths, &fresh, &home_node_id, options, report)?;
    finish_database_file(fresh)?;
    sync_file(rebuilding_path)
}

// ---- The kept tables ----

/// The four kept tables as the old file still reads them. No old file: all four start empty. A file this build
/// can't open, or a table it can't read, is reported and starts empty. A file a newer app migrated stops the rebuild
/// (`RRS-S0002`): this build would drop what it doesn't know.
fn read_kept_tables(paths: &DataPaths, report: &mut RebuildReport) -> Result<KeptRows, StoreError> {
    let old_path = rastai_db_path(paths);
    if !old_path.exists() {
        return Ok(KeptRows::default());
    }
    let connection = match open_database_for_salvage(&old_path, INDEX_MIGRATIONS) {
        Ok(connection) => connection,
        Err(error @ StoreError::SchemaTooNew { .. }) => return Err(error),
        Err(error) => {
            report.diagnostics.push(store_error_diagnostic(&error));
            return Ok(KeptRows::default());
        }
    };
    let old = IndexReader { connection };
    Ok(KeptRows {
        tokens: rows_or_report(list_tokens(&old), report),
        nodes: rows_or_report(list_nodes(&old), report),
        plugins: rows_or_report(list_plugins(&old), report),
        projects: rows_or_report(list_projects(&old), report),
    })
}

/// The rows a table read gave, or none, with the reason in the report.
fn rows_or_report<T>(rows: Result<Vec<T>, StoreError>, report: &mut RebuildReport) -> Vec<T> {
    match rows {
        Ok(rows) => rows,
        Err(error) => {
            report.diagnostics.push(store_error_diagnostic(&error));
            Vec::new()
        }
    }
}

/// The embedded node's id: the `local` row of the kept `nodes` table, or "" when it had none.
fn embedded_node_id(nodes: &[NodeRow]) -> String {
    match nodes.iter().find(|node| node.transport == LOCAL_TRANSPORT) {
        Some(node) => node.node_id.clone(),
        None => String::new(),
    }
}

/// Writes the kept rows into the new index, in one transaction. A row that can't be written is reported and left
/// out; the others still land.
fn write_kept_tables(fresh: &SqliteConnection, kept: &KeptRows, report: &mut RebuildReport) -> Result<(), StoreError> {
    let transaction = begin_transaction(fresh)?;
    for project in &kept.projects {
        let written = run_in_savepoint(&transaction, KEPT_ROW_SAVEPOINT, |inner| upsert_project(inner, project));
        report_problem(written, report);
    }
    for node in &kept.nodes {
        let written = run_in_savepoint(&transaction, KEPT_ROW_SAVEPOINT, |inner| upsert_node(inner, node));
        report_problem(written, report);
    }
    for plugin in &kept.plugins {
        let written = run_in_savepoint(&transaction, KEPT_ROW_SAVEPOINT, |inner| upsert_plugin(inner, plugin));
        report_problem(written, report);
    }
    // Oldest first (list_tokens), so a creator is written before the tokens it made.
    for token in &kept.tokens {
        write_kept_token(&transaction, token, report);
    }
    commit_transaction(transaction)
}

/// Writes one token as the old file had it. A live token whose creator is revoked or gone is written revoked now,
/// with `RRS-C1006` in the report: a damaged file may have lost the cascade that revoked it, so it fails closed
/// (contracts 8.2). A row already revoked is written as it is; `insert_token` doesn't check its creator (SEC-01).
fn write_kept_token(transaction: &SqliteTransaction<'_>, token: &TokenRow, report: &mut RebuildReport) {
    let written = run_in_savepoint(transaction, KEPT_ROW_SAVEPOINT, |inner| insert_token(inner, token));
    let Err(error @ StoreError::TokenCreatorRevoked { .. }) = written else {
        report_problem(written, report);
        return;
    };
    report.diagnostics.push(store_error_diagnostic(&error));
    let mut revoked = token.clone();
    revoked.revoked_unix_ms = Some(now_unix_ms());
    let written = run_in_savepoint(transaction, KEPT_ROW_SAVEPOINT, |inner| insert_token(inner, &revoked));
    report_problem(written, report);
}

fn report_problem(outcome: Result<(), StoreError>, report: &mut RebuildReport) {
    if let Err(error) = outcome {
        report.diagnostics.push(store_error_diagnostic(&error));
    }
}

// ---- The run folders ----

/// Every run folder, in id order (ids begin with their time, contracts 8.6, so this is the order they were made),
/// one transaction each.
fn rebuild_from_run_folders(
    paths: &DataPaths,
    fresh: &SqliteConnection,
    home_node_id: &str,
    report: &mut RebuildReport,
) -> Result<(), StoreError> {
    for run_folder in list_run_folders(&runs_dir(paths)) {
        let Some(record) = read_run_record(&run_folder, report) else {
            continue;
        };
        let transaction = begin_transaction(fresh)?;
        let applied = rebuild_one_run(&transaction, &run_folder, &record, home_node_id, report)?;
        commit_transaction(transaction)?;
        if let Some(record_count) = applied {
            report.runs += 1;
            report.records += record_count;
        }
    }
    Ok(())
}

/// The folders in `<data>/runs/`, sorted by name. No runs folder means no runs.
fn list_run_folders(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
}

/// The folder's `run.json` as a run record, or `None` with the reason in the report. It must name its own folder's
/// run: a run's rows are only ever rebuilt from that run's folder.
fn read_run_record(run_folder: &Path, report: &mut RebuildReport) -> Option<RunRecord> {
    let path = run_folder.join(RUN_RECORD_FILE);
    let file_name = path.display().to_string();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            report_unusable_run(report, &format!("{file_name}: {error}"));
            return None;
        }
    };
    let outcome = read_document(&file_name, &SliceSource { bytes: &bytes }, &ImportLimits::default());
    let errors: Vec<Diagnostic> = outcome
        .diagnostics
        .into_iter()
        .filter(|item| item.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        report.diagnostics.extend(errors);
        return None;
    }
    let Some(Document::RunRecord(record)) = outcome.document else {
        report_unusable_run(report, &format!("{file_name} is not a run record"));
        return None;
    };
    let folder_name = run_folder.file_name().map(|name| name.to_string_lossy().into_owned());
    if folder_name.as_deref() != Some(record.run_id.as_str()) || !id_has_prefix(&record.run_id, IdPrefix::Run) {
        report_unusable_run(
            report,
            &format!("{file_name} names run {:?}, not its folder's", record.run_id),
        );
        return None;
    }
    Some(record)
}

fn report_unusable_run(report: &mut RebuildReport, reason: &str) {
    report
        .diagnostics
        .push(diagnostic(CODE_READ_FAILED, &[("reason", reason)]));
}

/// One run's rows: its `runs` row and its first job's row from `run.json`, then its log. Returns how many records
/// were applied, or `None` when `run.json`'s time can't be read (the run is left out, with the reason).
fn rebuild_one_run(
    transaction: &SqliteTransaction<'_>,
    run_folder: &Path,
    record: &RunRecord,
    home_node_id: &str,
    report: &mut RebuildReport,
) -> Result<Option<u64>, StoreError> {
    let created_unix_ms = match parse_rfc3339(&record.created_at) {
        Ok(created_unix_ms) => created_unix_ms,
        Err(error) => {
            let reason = format!("{}: created_at {error:?}", run_folder.join(RUN_RECORD_FILE).display());
            report_unusable_run(report, &reason);
            return Ok(None);
        }
    };
    upsert_run(transaction, &run_row(record, home_node_id, created_unix_ms))?;
    write_first_job(transaction, record, home_node_id, report)?;
    let applied = apply_run_log(transaction, &run_folder.join(RUN_EVENTS_FILE), &record.run_id, report)?;
    Ok(Some(applied))
}

/// The run's row as `submit_job` writes it when the run starts: its log then moves the state and the ack.
fn run_row(record: &RunRecord, home_node_id: &str, created_unix_ms: i64) -> RunRow {
    RunRow {
        run_id: record.run_id.clone(),
        kind: run_kind_name(record),
        state: RUN_CREATED_STATE.to_string(),
        project_id: String::new(),
        node_id: home_node_id.to_string(),
        parent_run_id: record.parent_run_id.clone(),
        record_hash: record.header.content_hash.clone(),
        acked_sequence: 0,
        created_unix_ms,
        finished_unix_ms: None,
        pinned: false,
        archive_state: String::new(),
    }
}

/// The run kind's name in contracts 9.5, as `run.json` writes it (`train`).
fn run_kind_name(record: &RunRecord) -> String {
    match serde_json::to_value(record.kind) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// The `queued` row of the job `run.json`'s `job_spec` started the run with, as `submit_job` writes it; the daemon
/// log's `job.state_changed` records then move it. A run without a job (a chain or imported run) has none. A
/// `job_spec` that names a job of another run, or no job id at all, is reported and left out.
fn write_first_job(
    transaction: &SqliteTransaction<'_>,
    record: &RunRecord,
    home_node_id: &str,
    report: &mut RebuildReport,
) -> Result<(), StoreError> {
    let Some(spec_json) = &record.job_spec else {
        return Ok(());
    };
    let spec = match job_spec_from_json(spec_json) {
        Ok(spec) => spec,
        Err(reason) => {
            report_unusable_run(report, &format!("run {}: job_spec {reason}", record.run_id));
            return Ok(());
        }
    };
    let owner = read_job_run_id(transaction, &spec.job_id)?;
    let owned_elsewhere = owner.is_some_and(|owner| owner != record.run_id);
    if !id_has_prefix(&spec.job_id, IdPrefix::Job) || spec.run_id != record.run_id || owned_elsewhere {
        let reason = format!(
            "run {}: job_spec names job {:?} of run {:?}",
            record.run_id, spec.job_id, spec.run_id
        );
        report_unusable_run(report, &reason);
        return Ok(());
    }
    let job = JobRow {
        job_id: spec.job_id,
        run_id: record.run_id.clone(),
        node_id: home_node_id.to_string(),
        kind: job_kind_name(spec.kind),
        state: job_state_name(JobState::Queued).to_string(),
        pid: None,
        process_group_id: None,
        exit_code: None,
        error_code: String::new(),
    };
    upsert_job(transaction, &job)
}

/// Applies a run's log in batches, with the checks of `ApplyRunEvents`. Returns how many records were applied. A
/// run that never started has no log, which is no problem; any other log that can't be read is reported.
fn apply_run_log(
    transaction: &SqliteTransaction<'_>,
    log_path: &Path,
    run_id: &str,
    report: &mut RebuildReport,
) -> Result<u64, StoreError> {
    if !log_path.exists() {
        return Ok(0);
    }
    let mut reader = match open_event_log(log_path) {
        Ok(reader) => reader,
        Err(error) => {
            report.diagnostics.push(store_error_diagnostic(&error));
            return Ok(0);
        }
    };
    let mut applied = 0;
    loop {
        let (batch, log_ended) = read_batch(&mut reader, log_path, report);
        let (batch_applied, refused) = apply_run_batch(transaction, run_id, &batch, report)?;
        applied += batch_applied;
        if log_ended || refused {
            return Ok(applied);
        }
    }
}

/// Applies one batch in a savepoint. When the run's log may not carry one of its records (`RRS-S2004`), the batch
/// is undone, the records before that one are applied again, and the log ends there, as the live tail does. Returns
/// how many records were applied and whether one was refused.
fn apply_run_batch(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    batch: &[EventRecord],
    report: &mut RebuildReport,
) -> Result<(u64, bool), StoreError> {
    let applied = run_in_savepoint(transaction, BATCH_SAVEPOINT, |inner| {
        apply_run_events(inner, run_id, batch)
    });
    let refused_sequence = match &applied {
        Ok(()) => return Ok((batch.len() as u64, false)),
        Err(StoreError::RunLogRecordRefused { sequence, .. }) => *sequence,
        Err(other) => return Err(other.clone()),
    };
    if let Err(error) = &applied {
        report.diagnostics.push(store_error_diagnostic(error));
    }
    let before: Vec<EventRecord> = batch
        .iter()
        .filter(|record| record.sequence < refused_sequence)
        .cloned()
        .collect();
    run_in_savepoint(transaction, BATCH_SAVEPOINT, |inner| {
        apply_run_events(inner, run_id, &before)
    })?;
    Ok((before.len() as u64, true))
}

/// Up to `REBUILD_BATCH_RECORDS` records, and whether the log ended after them: at its end, a torn tail, or damage,
/// which is reported.
fn read_batch(reader: &mut EventLogReader, log_path: &Path, report: &mut RebuildReport) -> (Vec<EventRecord>, bool) {
    let mut batch = Vec::new();
    while batch.len() < REBUILD_BATCH_RECORDS {
        match read_next_frame(reader) {
            Ok(FrameRead::Record(record)) => batch.push(record),
            Ok(FrameRead::EndOfFile) | Ok(FrameRead::Incomplete { .. }) => return (batch, true),
            Ok(FrameRead::Corrupt { offset }) => {
                let corrupt = StoreError::EventLogCorrupt {
                    path: log_path.to_path_buf(),
                    offset,
                };
                report.diagnostics.push(store_error_diagnostic(&corrupt));
                return (batch, true);
            }
            Err(error) => {
                report.diagnostics.push(store_error_diagnostic(&error));
                return (batch, true);
            }
        }
    }
    (batch, false)
}

// ---- The daemon log ----

/// Every segment of the daemon log in order, each with its own cursor, one transaction each.
fn rebuild_from_daemon_log(
    paths: &DataPaths,
    fresh: &SqliteConnection,
    report: &mut RebuildReport,
) -> Result<(), StoreError> {
    for segment in daemon_log_segments(paths) {
        if !segment.path.exists() {
            continue;
        }
        let mut reader = match open_event_log(&segment.path) {
            Ok(reader) => reader,
            Err(error) => {
                report.diagnostics.push(store_error_diagnostic(&error));
                continue;
            }
        };
        let transaction = begin_transaction(fresh)?;
        loop {
            let (batch, log_ended) = read_batch(&mut reader, &segment.path, report);
            run_in_savepoint(&transaction, BATCH_SAVEPOINT, |inner| {
                apply_daemon_events(inner, &segment.log_name, &batch)
            })?;
            report.records += batch.len() as u64;
            if log_ended {
                break;
            }
        }
        commit_transaction(transaction)?;
    }
    Ok(())
}

// ---- Blobs ----

/// Drops every artifact whose home is this machine and whose blob is missing (`RRS-S1002`), or, with
/// `verify_blobs`, doesn't match its hash and was quarantined (`RRS-S1001`). A blob that can't be read is reported
/// and its rows kept: its bytes may still be there.
fn drop_rows_with_missing_blobs(
    paths: &DataPaths,
    fresh: &SqliteConnection,
    home_node_id: &str,
    options: &RebuildOptions,
    report: &mut RebuildReport,
) -> Result<(), StoreError> {
    let blobs = BlobStore { root: blobs_dir(paths) };
    let transaction = begin_transaction(fresh)?;
    let artifacts = list_artifact_blobs(&transaction)?;
    let mut kept = artifacts.len() as u64;
    for artifact in &artifacts {
        if home_node_id.is_empty() || artifact.home_node_id != home_node_id {
            continue;
        }
        let Some(problem) = blob_problem(&blobs, &artifact.content_hash, options.verify_blobs) else {
            continue;
        };
        report.diagnostics.push(store_error_diagnostic(&problem));
        if matches!(
            problem,
            StoreError::BlobMissing { .. } | StoreError::BlobHashMismatch { .. }
        ) {
            delete_artifact(&transaction, &artifact.artifact_id)?;
            kept -= 1;
        }
    }
    commit_transaction(transaction)?;
    report.artifacts = kept;
    Ok(())
}

/// What is wrong with a blob, or `None` when it is there (and, with `verify`, matches its hash).
fn blob_problem(blobs: &BlobStore, content_hash: &str, verify: bool) -> Option<StoreError> {
    if verify {
        return verify_blob(blobs, content_hash).err();
    }
    if has_blob(blobs, content_hash) {
        return None;
    }
    Some(StoreError::BlobMissing {
        hash: content_hash.to_string(),
    })
}

// ---- Putting the new index in place ----

/// Puts the finished new index at `rastai.db`. The old file is linked (or copied) to `rastai.db.broken-<unix_ms>`
/// with its `-wal` first, and stays at `rastai.db` until the rename replaces it, so there is always an index there.
/// Its `-wal` and `-shm` are removed before the rename, since SQLite would apply an old `-wal` to the new file. The
/// rebuild opened and closed the old file to read it, which moves a `-wal`'s pages into it, so normally there is
/// none. Returns where the old index was kept.
fn swap_in_rebuilt_index(
    paths: &DataPaths,
    rebuilding_path: &Path,
    options: &RebuildOptions,
) -> Result<Option<PathBuf>, StoreError> {
    let index_path = rastai_db_path(paths);
    let mut old_index_path = None;
    if index_path.exists() {
        let broken_path = rastai_db_broken_path(paths, now_unix_ms());
        keep_old_file(&index_path, &broken_path)?;
        keep_old_file(
            &companion_path(&index_path, WAL_FILE_SUFFIX),
            &companion_path(&broken_path, WAL_FILE_SUFFIX),
        )?;
        old_index_path = Some(broken_path);
    }
    pause_if_asked(options, PAUSE_BEFORE_RENAME);
    remove_file_if_present(&companion_path(&index_path, WAL_FILE_SUFFIX))?;
    remove_file_if_present(&companion_path(&index_path, SHARED_MEMORY_FILE_SUFFIX))?;
    std::fs::rename(rebuilding_path, &index_path).map_err(|error| not_writable(&index_path, &error))?;
    sync_folder(&paths.data)?;
    Ok(old_index_path)
}

/// A second name for the old file: a hard link costs no space; a filesystem without links gets a copy.
fn keep_old_file(path: &Path, kept_path: &Path) -> Result<(), StoreError> {
    if !path.exists() {
        return Ok(());
    }
    if std::fs::hard_link(path, kept_path).is_ok() {
        return Ok(());
    }
    std::fs::copy(path, kept_path).map_err(|error| not_writable(kept_path, &error))?;
    sync_file(kept_path)
}

/// `rastai.db-wal` for `rastai.db`: SQLite names a database's companion files this way.
fn companion_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut name = database_path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Removes a database file and its companions; a file that isn't there is fine.
fn remove_database_files(database_path: &Path) -> Result<(), StoreError> {
    remove_file_if_present(database_path)?;
    remove_file_if_present(&companion_path(database_path, WAL_FILE_SUFFIX))?;
    remove_file_if_present(&companion_path(database_path, SHARED_MEMORY_FILE_SUFFIX))
}

fn remove_file_if_present(path: &Path) -> Result<(), StoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(not_writable(path, &error)),
    }
}

fn sync_file(path: &Path) -> Result<(), StoreError> {
    let file = File::open(path).map_err(|error| not_writable(path, &error))?;
    file.sync_all().map_err(|error| not_writable(path, &error))
}

/// Makes a rename in the folder durable.
#[cfg(unix)]
fn sync_folder(folder: &Path) -> Result<(), StoreError> {
    sync_file(folder)
}

/// Windows can't open a folder to sync it; a rename there is made durable by the file system itself.
#[cfg(not(unix))]
fn sync_folder(_folder: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// A file in the data folder that can't be created, linked, renamed or removed: `RRS-S0004`.
fn not_writable(path: &Path, error: &std::io::Error) -> StoreError {
    StoreError::DataDirectoryNotWritable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

// ---- The test-only pause ----

/// Stops this process with SIGSTOP when `options.pause_at` names `point`, so the kill -9 test's SIGKILL lands
/// there every time (contracts 15.9). The line on stderr tells a failing test's output where it stopped.
fn pause_if_asked(options: &RebuildOptions, point: &str) {
    if options.pause_at.is_empty() || options.pause_at != point {
        return;
    }
    let pid = std::process::id();
    eprintln!("RASTAI_TEST_PAUSE_AT: process {pid} paused at {point}");
    let _ = std::process::Command::new(KILL_TOOL)
        .args(["-s", STOP_SIGNAL, &pid.to_string()])
        .status();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::sqlite::Migration;

    // The migration a newer app would have added; this build doesn't know it.
    const NEWER_MIGRATION_SQL: &str = "CREATE TABLE added_by_a_newer_app (note TEXT NOT NULL) STRICT;";

    fn test_paths(test_name: &str) -> DataPaths {
        let root = std::env::temp_dir().join(format!("rastai-store-rebuild-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        }
    }

    // Here rather than in tests/rebuild_index.rs: a newer app's file is made with the crate-private migrations.
    // Rebuilding it would drop what this build doesn't know, so the rebuild stops before it changes anything.
    #[test]
    fn an_index_from_a_newer_app_stops_the_rebuild_and_stays_as_it_was() {
        let paths = test_paths("newer-schema");
        std::fs::create_dir_all(&paths.data).unwrap();
        let mut newer_migrations: Vec<Migration> = INDEX_MIGRATIONS
            .iter()
            .map(|migration| Migration {
                version: migration.version,
                sql: migration.sql,
            })
            .collect();
        newer_migrations.push(Migration {
            version: i64::try_from(INDEX_MIGRATIONS.len()).unwrap() + 1,
            sql: NEWER_MIGRATION_SQL,
        });
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, &newer_migrations).unwrap());
        let bytes_before = std::fs::read(&index_path).unwrap();

        let error = rebuild_index(&paths, &RebuildOptions::default()).unwrap_err();

        assert_eq!(error.code(), "RRS-S0002");
        assert_eq!(std::fs::read(&index_path).unwrap(), bytes_before);
        assert!(!rastai_db_rebuilding_path(&paths).exists());
        let names: Vec<String> = std::fs::read_dir(&paths.data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["rastai.db"], "a newer app's index was copied or set aside");
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }
}
