//! The offline rebuild of `rastai.db` (layout rust-control-plane.md 13.2, flows/platform.md 6.4): what
//! `rastai storage rebuild-index` runs, the fix for `RRS-S0003`. The caller holds `daemon.lock`, so no daemon writes
//! the index or the logs meanwhile.
//!
//! - **Same code as live indexing.** Run logs go through `apply_run_events` (the checks of `ApplyRunEvents`, then
//!   `apply_event_record`) and the daemon log through `apply_daemon_events`, so rebuilt rows can't differ from live
//!   ones. The kept tables (`tokens`, `nodes`, `plugins`, `projects`) are copied from the old file, whatever of them
//!   it still reads; a table it can't read starts empty, which fails closed (contracts 8.2).
//! - **The old file is kept as it was.** The kept tables are read from a copy, `rastai.db.salvage`, which is
//!   migrated to this build's columns if it is older and removed once read. SQLite never opens the old file, so
//!   `rastai.db.broken-<unix_ms>` holds the very bytes `rastai.db` had (with its `-wal`, if it had one).
//!   A copy that can't be made stops the rebuild before it changes anything: an old file that can't be read is
//!   `RRS-S0006` naming it; a copy that can't be written for a permission reason is `RRS-S0004` naming the data
//!   folder, any other failed write is `RRS-S3005` with the io error, and a full disk is `RRS-S3002`.
//! - **Crash-safe.** The new index is written to `rastai.db.rebuilding` and only takes the old one's place once it
//!   is whole and has passed `quick_check`. The old file is copied to `rastai.db.broken-<unix_ms>` first and stays
//!   at `rastai.db` until the rename, so a kill at any point leaves a usable index: the old one, or the new one. A
//!   rebuild started again removes the `rastai.db.rebuilding` and `rastai.db.salvage` a killed one left. A rebuild
//!   killed after the copy leaves its `rastai.db.broken-<unix_ms>` behind, and every finished rebuild keeps one: they
//!   are the user's to delete, and the report names only the one this rebuild made. The files the rebuild creates
//!   (`.rebuilding`, `.salvage`, `broken-*`) are never written through a symbolic link planted at their path, and the
//!   kept copy takes the old file's mode.
//! - **Reading is tolerant.** A torn tail ends a log; a corrupt frame (`RRS-S2002`), a sequence gap (`RRS-S2003`)
//!   or a record a run's log may not carry (`RRS-S2004`) ends it with that code in the report, and the rebuild goes
//!   on with the next log. A run folder with no `run.json` was never indexed (`submit_job` writes the record before
//!   the row), so it is passed over quietly. A `run.json` that can't be read (`RRS-S0006`) leaves its run out; one
//!   with a value the rebuild can't use (`RRS-E2006` at that value: a run id that isn't its folder's, a `created_at`
//!   that isn't a time, a `job_spec` of another run's job) leaves its run, or its job, out.
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

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::paths::{
    DataPaths, blobs_dir, rastai_db_broken_path, rastai_db_path, rastai_db_rebuilding_path, rastai_db_salvage_path,
    runs_dir,
};
use rastai_core::proto_json::job_spec_from_json;
use rastai_core::states::{job_kind_name, job_state_name, run_kind_name};
use rastai_core::time::{now_unix_ms, parse_rfc3339};
use rastai_diag::diagnostic::{Diagnostic, Severity, diagnostic_at};
use rastai_diag::path::{DiagnosticPath, format_path};
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
    NodeRow, PluginRow, ProjectRow, TOKEN_CREATOR_PREFIX, TokenRow, insert_token, list_nodes, list_plugins,
    list_projects, list_tokens, upsert_node, upsert_plugin, upsert_project,
};
use crate::index::reads::{IndexReader, TableRows};
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
/// How much of a file `copy_file` holds in memory at once.
const COPY_BUFFER_BYTES: usize = 64 * 1024;
/// What the user can do when the old index can't be read, added to its `RRS-S0006` reason. The rebuild stops rather
/// than drop the kept tables: a fixed permission or a disk that reads again gives them back.
const OLD_FILE_UNREADABLE_NEXT_STEP: &str = "fix the cause and rebuild again, or move the file aside to rebuild without its tokens, nodes, plugins and projects";
/// `RRS-S0006`, the store's code for a failed read (crate CLAUDE.md): a `run.json` the file system won't give up.
const CODE_READ_FAILED: &str = "RRS-S0006";
/// `RRS-E2006`, "Value isn't in its required form", at the value: what a document check reports for a value it
/// can't use, here one that leaves a run or its job out of the rebuilt index.
const CODE_VALUE_FORM: &str = "RRS-E2006";
/// The run record's document type, which a `run.json` must be.
const RUN_RECORD_EXPECTED: &str = "a rastai.run_record document";
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
    std::fs::create_dir_all(&paths.data).map_err(|error| write_error(&paths.data, &error))?;
    let rebuilding_path = rastai_db_rebuilding_path(paths);
    remove_database_files(&rebuilding_path)?;
    remove_database_files(&rastai_db_salvage_path(paths))?;
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
    create_scratch_file(rebuilding_path)?;
    let fresh = open_database_for_writing(rebuilding_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS)?;
    let kept = read_kept_tables(paths, report)?;
    let home_node_id = embedded_node_id(&kept.nodes);
    write_kept_tables(&fresh, &kept, report)?;
    rebuild_from_run_folders(paths, &fresh, &home_node_id, report)?;
    pause_if_asked(options, PAUSE_AFTER_RUN_FOLDERS);
    rebuild_from_daemon_log(paths, &fresh, report)?;
    drop_rows_with_missing_blobs(paths, &fresh, &home_node_id, options, report)?;
    finish_database_file(fresh)?;
    match_old_index_mode(&rastai_db_path(paths), rebuilding_path)?;
    sync_file(rebuilding_path)
}

// ---- The kept tables ----

/// The four kept tables as the old file still reads them. No old file: all four start empty. They are read from a
/// copy, `rastai.db.salvage`, removed afterwards: opening the old file would migrate it and write to it, and the
/// copy kept at `rastai.db.broken-<unix_ms>` must be the file as it was. A copy that can't be made stops the
/// rebuild, since going on would drop every token and node: an old file that can't be read (a permission, an I/O
/// error) is `RRS-S0006` naming it, since it may read again once the cause is fixed. A copy that can't be written is
/// the write codes of `write_error`.
fn read_kept_tables(paths: &DataPaths, report: &mut RebuildReport) -> Result<KeptRows, StoreError> {
    let old_path = rastai_db_path(paths);
    if !old_path.exists() {
        return Ok(KeptRows::default());
    }
    let salvage_path = rastai_db_salvage_path(paths);
    let kept = match copy_database_files(&old_path, &salvage_path) {
        Ok(()) => read_kept_tables_from(&old_path, &salvage_path, report),
        Err(error) => Err(error),
    };
    remove_database_files(&salvage_path)?;
    kept
}

/// The kept tables of the copy at `salvage_path`. A copy this build can't open, or a table it can't read, is
/// reported and starts empty. A file a newer app migrated stops the rebuild (`RRS-S0002`, naming the old file):
/// this build would drop what it doesn't know.
fn read_kept_tables_from(
    old_path: &Path,
    salvage_path: &Path,
    report: &mut RebuildReport,
) -> Result<KeptRows, StoreError> {
    let connection = match open_database_for_salvage(salvage_path, INDEX_MIGRATIONS) {
        Ok(connection) => connection,
        Err(StoreError::SchemaTooNew {
            found_version,
            newest_known_version,
            ..
        }) => {
            return Err(StoreError::SchemaTooNew {
                path: old_path.to_path_buf(),
                found_version,
                newest_known_version,
            });
        }
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
    write_kept_tokens(&transaction, &kept.tokens, report);
    commit_transaction(transaction)
}

/// Writes the tokens with every creator before the tokens it made, whatever their times: a child can be older than
/// its creator, and the creator check refuses a child written first. Each pass writes the tokens whose creator is not
/// still waiting to be written. A damaged file whose tokens name each other has no such token, so the rest go in
/// order, and the creator check revokes what it refuses.
fn write_kept_tokens(transaction: &SqliteTransaction<'_>, tokens: &[TokenRow], report: &mut RebuildReport) {
    let mut waiting: Vec<&TokenRow> = tokens.iter().collect();
    while !waiting.is_empty() {
        let mut ready: Vec<&TokenRow> = waiting
            .iter()
            .copied()
            .filter(|token| !waits_for_creator(token, &waiting))
            .collect();
        if ready.is_empty() {
            ready = waiting.clone();
        }
        for token in &ready {
            write_kept_token(transaction, token, report);
        }
        waiting.retain(|token| !ready.iter().any(|written| written.token_id == token.token_id));
    }
}

/// Whether the token's creator is one of the `waiting` tokens, not yet written.
fn waits_for_creator(token: &TokenRow, waiting: &[&TokenRow]) -> bool {
    match token.created_by.strip_prefix(TOKEN_CREATOR_PREFIX) {
        Some(creator_token_id) => waiting.iter().any(|other| other.token_id == creator_token_id),
        None => false,
    }
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
        // A kill between making the folder and writing the record: `submit_job` writes `run.json` before the run's
        // row, so the live index never had this run either.
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            let reason = format!("{file_name}: {error}");
            let unreadable = diagnostic_at(CODE_READ_FAILED, &[("reason", &reason)], &file_name);
            report.diagnostics.push(unreadable);
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
        report_unusable_value(report, &path, "", "this document", RUN_RECORD_EXPECTED);
        return None;
    };
    let folder_name = run_folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if folder_name != record.run_id || !id_has_prefix(&record.run_id, IdPrefix::Run) {
        let found = format!("{:?}", record.run_id);
        let expected = format!("{folder_name}, the name of its folder");
        report_unusable_value(report, &path, "/run_id", &found, &expected);
        return None;
    }
    Some(record)
}

/// A value of `run.json` the rebuild can't use, which leaves its run or its job out: `RRS-E2006` at that value, as a
/// document check reports one. `json_pointer` is "" for the whole document.
fn report_unusable_value(
    report: &mut RebuildReport,
    record_path: &Path,
    json_pointer: &str,
    found: &str,
    expected: &str,
) {
    let location = DiagnosticPath {
        file: record_path.display().to_string(),
        json_pointer: json_pointer.to_string(),
        ..DiagnosticPath::default()
    };
    let values = [("found", found), ("expected", expected)];
    report
        .diagnostics
        .push(diagnostic_at(CODE_VALUE_FORM, &values, &format_path(&location)));
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
    let record_path = run_folder.join(RUN_RECORD_FILE);
    let Ok(created_unix_ms) = parse_rfc3339(&record.created_at) else {
        let found = format!("{:?}", record.created_at);
        report_unusable_value(report, &record_path, "/created_at", &found, "an RFC 3339 time");
        return Ok(None);
    };
    upsert_run(transaction, &run_row(record, home_node_id, created_unix_ms))?;
    write_first_job(transaction, &record_path, record, home_node_id, report)?;
    let applied = apply_run_log(transaction, &run_folder.join(RUN_EVENTS_FILE), &record.run_id, report)?;
    Ok(Some(applied))
}

/// The run's row as `submit_job` writes it when the run starts: its log then moves the state and the ack.
fn run_row(record: &RunRecord, home_node_id: &str, created_unix_ms: i64) -> RunRow {
    RunRow {
        run_id: record.run_id.clone(),
        kind: run_kind_name(record.kind),
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

/// The `queued` row of the job `run.json`'s `job_spec` started the run with, as `submit_job` writes it; the daemon
/// log's `job.state_changed` records then move it. A run without a job (a chain or imported run) has none. A
/// `job_spec` that names a job of another run, or no job id at all, is reported and left out.
fn write_first_job(
    transaction: &SqliteTransaction<'_>,
    record_path: &Path,
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
            let found = format!("a job_spec that can't be read ({reason})");
            report_unusable_value(report, record_path, "/job_spec", &found, "a JobSpec");
            return Ok(());
        }
    };
    let owner = read_job_run_id(transaction, &spec.job_id)?;
    let owned_elsewhere = owner.is_some_and(|owner| owner != record.run_id);
    if !id_has_prefix(&spec.job_id, IdPrefix::Job) || spec.run_id != record.run_id || owned_elsewhere {
        let found = format!("job {:?} of run {:?}", spec.job_id, spec.run_id);
        let expected = format!("a job of run {} that no other run's job_spec names", record.run_id);
        report_unusable_value(report, record_path, "/job_spec/job_id", &found, &expected);
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
    let refused_sequence = match applied {
        Ok(()) => return Ok((batch.len() as u64, false)),
        Err(refused @ StoreError::RunLogRecordRefused { sequence, .. }) => {
            report.diagnostics.push(store_error_diagnostic(&refused));
            sequence
        }
        Err(other) => return Err(other),
    };
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

/// Puts the finished new index at `rastai.db`. The old file is copied to `rastai.db.broken-<unix_ms>`
/// with its `-wal` first, and stays at `rastai.db` until the rename replaces it, so there is always an index there.
/// Its `-wal` and `-shm` are removed before the rename, since SQLite would apply an old `-wal` to the new file; the
/// kept copy has the `-wal` under its own name, and SQLite makes a new `-shm` when it is opened. A kill between the
/// removal and the rename leaves the old file at `rastai.db` without its `-wal`, so pages only the `-wal` held are
/// missing there, but never from the kept copy beside it. Returns where the old index was kept.
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
    std::fs::rename(rebuilding_path, &index_path).map_err(|error| write_error(&index_path, &error))?;
    sync_folder(&paths.data)?;
    Ok(old_index_path)
}

/// Keeps the old file under its kept name as a copy of its own. Not a hard link: a link shares the old file's bytes,
/// so a daemon killed before the rename and started again would write its rows into the kept index too, and the kept
/// index would no longer be the file the user had (contracts 8.1).
fn keep_old_file(path: &Path, kept_path: &Path) -> Result<(), StoreError> {
    if !path.exists() {
        return Ok(());
    }
    copy_file(path, kept_path)?;
    sync_file(kept_path)
}

/// Makes the empty file a scratch index is opened on, with `create_new`: a link or file planted at the path between
/// the removal and the creation makes this fail, so the open never writes through it. Only the folders above the
/// file are followed, as the daemon's own open follows them, so a data folder reached through a symbolic link works.
fn create_scratch_file(path: &Path) -> Result<(), StoreError> {
    remove_file_if_present(path)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| write_error(path, &error))?;
    drop(file);
    Ok(())
}

/// Gives the new index the old index's mode, so a 0600 index stays 0600. The kept copy has the old mode already.
fn match_old_index_mode(index_path: &Path, rebuilding_path: &Path) -> Result<(), StoreError> {
    if !index_path.exists() {
        return Ok(());
    }
    let old_permissions = std::fs::metadata(index_path)
        .map_err(|error| old_file_unreadable(index_path, &error))?
        .permissions();
    let rebuilding = File::open(rebuilding_path).map_err(|error| write_error(rebuilding_path, &error))?;
    rebuilding
        .set_permissions(old_permissions)
        .map_err(|error| write_error(rebuilding_path, &error))
}

/// Copies a database file and its `-wal`, if it has one, to `copy_path`, so the copy holds every committed page.
/// The `-shm` is left: it is only an index into the `-wal`, which SQLite builds again on open. No one writes the
/// file meanwhile (the caller holds `daemon.lock`).
fn copy_database_files(database_path: &Path, copy_path: &Path) -> Result<(), StoreError> {
    copy_file(database_path, copy_path)?;
    let wal_path = companion_path(database_path, WAL_FILE_SUFFIX);
    if !wal_path.exists() {
        return Ok(());
    }
    copy_file(&wal_path, &companion_path(copy_path, WAL_FILE_SUFFIX))
}

/// Copies one file, telling a failed read of `source` (`RRS-S0006`, naming it) from a failed write of the copy
/// (`write_error`), which `std::fs::copy` reports alike.
fn copy_file(source: &Path, destination: &Path) -> Result<(), StoreError> {
    let mut source_file = File::open(source).map_err(|error| old_file_unreadable(source, &error))?;
    let permissions = source_file
        .metadata()
        .map_err(|error| old_file_unreadable(source, &error))?
        .permissions();
    let mut copy = create_new_file(destination, &permissions)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let read_bytes = match source_file.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read_bytes) => read_bytes,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(old_file_unreadable(source, &error)),
        };
        copy.write_all(&buffer[..read_bytes])
            .map_err(|error| write_error(destination, &error))?;
    }
}

/// Creates a file that is not there yet, with the mode `permissions` gives. What is at the path now, a symbolic link
/// included, is removed first, and the file is made with `create_new`, which refuses a path that exists: a link
/// planted in between is refused, never written through.
fn create_new_file(path: &Path, permissions: &std::fs::Permissions) -> Result<File, StoreError> {
    remove_file_if_present(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(std::os::unix::fs::PermissionsExt::mode(permissions));
    }
    let file = options.open(path).map_err(|error| write_error(path, &error))?;
    // The umask can only clear bits of the mode above, so the exact mode is set here.
    file.set_permissions(permissions.clone())
        .map_err(|error| write_error(path, &error))?;
    Ok(file)
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
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(write_error(path, &error)),
    }
}

fn sync_file(path: &Path) -> Result<(), StoreError> {
    let file = File::open(path).map_err(|error| write_error(path, &error))?;
    file.sync_all().map_err(|error| write_error(path, &error))
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

/// A file in the data folder that can't be created, written, copied, renamed or removed. A permission (or a read-only
/// folder) is `RRS-S0004` naming the data folder, whose fix is the folder's permissions. A full disk is `RRS-S3002`,
/// whose fix is space. Any other failure is `RRS-S3005` with the file and the io error: the user needs to know which
/// write failed and why, not only that the folder is the problem.
fn write_error(path: &Path, error: &std::io::Error) -> StoreError {
    match error.kind() {
        ErrorKind::StorageFull => StoreError::DiskFull {
            reason: format!("{}: {error}", path.display()),
        },
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => StoreError::DataDirectoryNotWritable {
            path: path.parent().unwrap_or(path).to_path_buf(),
            reason: error.to_string(),
        },
        _ => StoreError::DatabaseWriteFailed {
            reason: format!("{}: {error}", path.display()),
        },
    }
}

/// The old index, or its `-wal`, that can't be read: `RRS-S0006` naming it, with what the user can do next.
fn old_file_unreadable(path: &Path, error: &std::io::Error) -> StoreError {
    StoreError::DatabaseReadFailed {
        reason: format!("{}: {error}; {OLD_FILE_UNREADABLE_NEXT_STEP}", path.display()),
    }
}

// ---- Comparing a rebuilt index with the live one ----

/// How `rebuilt` differs from `live` beyond what a rebuild can't bring back: one line per difference, none when they
/// match. Every table must be in both; a table of `TABLES_NOT_REBUILT` must be empty in `rebuilt`, and every other
/// one must hold the same rows once the `COLUMNS_NOT_REBUILT` are left out. `tests/rebuild_index.rs` (here and in
/// rastai-cli) checks a rebuild with it.
pub fn rebuilt_table_differences(live: &[TableRows], rebuilt: &[TableRows]) -> Vec<String> {
    let live_names: Vec<&str> = live.iter().map(|table| table.table_name.as_str()).collect();
    let rebuilt_names: Vec<&str> = rebuilt.iter().map(|table| table.table_name.as_str()).collect();
    if live_names != rebuilt_names {
        return vec![format!(
            "the tables differ: live {live_names:?}, rebuilt {rebuilt_names:?}"
        )];
    }
    let mut differences = Vec::new();
    for (live_table, rebuilt_table) in live.iter().zip(rebuilt) {
        let table_name = &live_table.table_name;
        if TABLES_NOT_REBUILT.contains(&table_name.as_str()) {
            if !rebuilt_table.rows.is_empty() {
                differences.push(format!("{table_name} should start empty: {:?}", rebuilt_table.rows));
            }
            continue;
        }
        let live_rows = rebuildable_rows(live_table);
        let rebuilt_rows = rebuildable_rows(rebuilt_table);
        if live_rows != rebuilt_rows {
            differences.push(format!(
                "table {table_name} differs: live {live_rows:?}, rebuilt {rebuilt_rows:?}"
            ));
        }
    }
    differences
}

/// The table's rows without the columns no source holds, sorted again.
fn rebuildable_rows(table: &TableRows) -> Vec<Vec<String>> {
    let skipped: &[&str] = match COLUMNS_NOT_REBUILT
        .iter()
        .find(|columns| columns.table_name == table.table_name)
    {
        Some(columns) => columns.column_names,
        None => &[],
    };
    let mut kept_rows = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        let mut kept = Vec::new();
        for (value, column_name) in row.iter().zip(&table.column_names) {
            if !skipped.contains(&column_name.as_str()) {
                kept.push(value.clone());
            }
        }
        kept_rows.push(kept);
    }
    kept_rows.sort();
    kept_rows
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
    use crate::sqlite::{
        Migration, SqlValue, execute, integer_value, open_database_for_reading, open_database_for_writing, text_value,
    };

    // The migration a newer app would have added; this build doesn't know it.
    const NEWER_MIGRATION_SQL: &str = "CREATE TABLE added_by_a_newer_app (note TEXT NOT NULL) STRICT;";
    // A creator and the token it made, both live when the damage happens.
    const CREATOR_TOKEN_ID: &str = "tok_01JA2B3C4D5E6F7G8H9J0K1M2A";
    const CHILD_TOKEN_ID: &str = "tok_01JA2B3C4D5E6F7G8H9J0K1M2B";
    const TOKEN_CREATED_UNIX_MS: i64 = 1_791_700_000_000;
    const TOKEN_REVOKED_UNIX_MS: i64 = 1_791_700_100_000;
    // Damage that loses a revoke's cascade, or the creator's row, which the store's own writes never do.
    const REVOKE_CREATOR_ALONE_SQL: &str = "UPDATE tokens SET revoked_unix_ms = ?1 WHERE token_id = ?2";
    const DELETE_CREATOR_SQL: &str = "DELETE FROM tokens WHERE token_id = ?1";
    const TOKEN_REVOKED_CODE: &str = "RRS-C1006";
    // A child token made this many milliseconds before its creator, as an old clock can record it.
    const CHILD_EARLIER_BY_MS: i64 = 1_000;
    // The unix time the kept copies of these tests are named after.
    const KEPT_UNIX_MS: i64 = 1_791_700_200_000;
    // An old index no one but root can read, and the mode that lets the test remove it again.
    #[cfg(unix)]
    const UNREADABLE_MODE: u32 = 0o000;
    #[cfg(unix)]
    const READABLE_MODE: u32 = 0o600;
    // The mode of an index only its owner reads, which a rebuild must keep.
    #[cfg(unix)]
    const PRIVATE_MODE: u32 = 0o600;

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

    // An old index the rebuild can't read (a permission here; an I/O error the same way): it may read again once the
    // cause is fixed, so the rebuild stops with the read code naming it, not RRS-S0004 for the scratch copy, and
    // leaves the data folder as it was.
    #[cfg(unix)]
    #[test]
    fn an_old_index_that_cannot_be_read_stops_the_rebuild_with_rrs_s0006_naming_it() {
        use std::os::unix::fs::PermissionsExt;
        let paths = test_paths("unreadable-old-index");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap());
        std::fs::set_permissions(&index_path, std::fs::Permissions::from_mode(UNREADABLE_MODE)).unwrap();
        if File::open(&index_path).is_ok() {
            // Root reads any file, so no permission makes this one unreadable.
            eprintln!("skipped: this user reads files whatever their permissions");
            std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
            return;
        }

        let error = rebuild_index(&paths, &RebuildOptions::default()).unwrap_err();

        std::fs::set_permissions(&index_path, std::fs::Permissions::from_mode(READABLE_MODE)).unwrap();
        assert_eq!(error.code(), "RRS-S0006");
        let message = store_error_diagnostic(&error).message;
        assert!(
            message.contains(&format!("{}: ", index_path.display())),
            "the message doesn't name the old index: {message}"
        );
        assert!(message.contains("move the file aside"), "no next step: {message}");
        let names: Vec<String> = std::fs::read_dir(&paths.data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["rastai.db"], "the rebuild left a file behind");
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // std::fs::copy reports both alike; a full disk needs space, not permissions, so it gets its own code.
    #[test]
    fn a_full_disk_is_rrs_s3002_and_a_permission_rrs_s0004_naming_the_folder() {
        let path = Path::new("/data/rastai.db.salvage");
        let disk_full = std::io::Error::from(ErrorKind::StorageFull);
        let permission_denied = std::io::Error::from(ErrorKind::PermissionDenied);

        assert_eq!(write_error(path, &disk_full).code(), "RRS-S3002");
        let denied = store_error_diagnostic(&write_error(path, &permission_denied)).message;
        assert_eq!(denied, "The data directory /data is not writable.");
    }

    // Any other failed write names the file and keeps the io error's words, so a failed salvage write doesn't read as
    // a folder that can't be written.
    #[test]
    fn any_other_failed_write_is_rrs_s3005_with_the_io_error() {
        let path = Path::new("/data/rastai.db.salvage");
        let injected = std::io::Error::other("injected write failure");

        let error = write_error(path, &injected);
        assert_eq!(error.code(), "RRS-S3005");
        let message = store_error_diagnostic(&error).message;
        assert!(
            message.contains("/data/rastai.db.salvage: injected write failure"),
            "{message}"
        );
    }

    // A salvage copy into a read-only data folder: the write is refused, and the error names the folder (RRS-S0004),
    // not the scratch file the rebuild was writing.
    #[cfg(unix)]
    #[test]
    fn a_salvage_copy_into_a_read_only_folder_is_rrs_s0004_naming_the_folder() {
        use std::os::unix::fs::PermissionsExt;
        const READ_ONLY_FOLDER_MODE: u32 = 0o500;
        const OWNER_FOLDER_MODE: u32 = 0o700;
        let paths = test_paths("read-only-salvage");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap());
        std::fs::set_permissions(&paths.data, std::fs::Permissions::from_mode(READ_ONLY_FOLDER_MODE)).unwrap();
        let folder_refuses_writes = std::fs::File::create(paths.data.join("probe")).is_err();
        if !folder_refuses_writes {
            // Root writes into any folder, so no permission makes this one read-only.
            eprintln!("skipped: this user writes into read-only folders");
            std::fs::set_permissions(&paths.data, std::fs::Permissions::from_mode(OWNER_FOLDER_MODE)).unwrap();
            std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
            return;
        }

        let error = rebuild_index(&paths, &RebuildOptions::default()).unwrap_err();

        std::fs::set_permissions(&paths.data, std::fs::Permissions::from_mode(OWNER_FOLDER_MODE)).unwrap();
        assert_eq!(error.code(), "RRS-S0004");
        let message = store_error_diagnostic(&error).message;
        assert_eq!(
            message,
            format!("The data directory {} is not writable.", paths.data.display())
        );
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // An older index is migrated to be read, but only in the scratch copy: the kept old index is the file as it was,
    // so its schema and bytes are the ones the user had (RRS-S0003 is the reason to keep it).
    #[test]
    fn the_kept_old_index_is_the_file_as_it_was_when_its_schema_is_older() {
        let paths = test_paths("older-schema");
        std::fs::create_dir_all(&paths.data).unwrap();
        let older_migrations = &INDEX_MIGRATIONS[..INDEX_MIGRATIONS.len() - 1];
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, older_migrations).unwrap());
        let bytes_before = std::fs::read(&index_path).unwrap();

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        let old_index_path = report.old_index_path.unwrap();
        assert_eq!(
            std::fs::read(&old_index_path).unwrap(),
            bytes_before,
            "the kept old index was changed"
        );
        assert!(
            !rastai_db_salvage_path(&paths).exists(),
            "the scratch copy was left behind"
        );
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A daemon killed with rows still in rastai.db-wal: the scratch copy takes the -wal along, so those rows are
    // kept, and the kept old index has its -wal under its own name.
    #[test]
    fn rows_still_in_the_old_index_wal_are_kept_and_the_wal_is_kept_with_it() {
        let killed_paths = test_paths("wal-killed-daemon");
        let paths = test_paths("wal-rebuilt");
        std::fs::create_dir_all(&killed_paths.data).unwrap();
        std::fs::create_dir_all(&paths.data).unwrap();
        let killed_index_path = rastai_db_path(&killed_paths);
        let connection =
            open_database_for_writing(&killed_index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        insert_token(&transaction, &live_token(CREATOR_TOKEN_ID, "owner")).unwrap();
        commit_transaction(transaction).unwrap();
        // Copied while the connection is open, as a kill leaves them: the token is only in the -wal.
        let killed_wal_path = companion_path(&killed_index_path, WAL_FILE_SUFFIX);
        let wal_bytes = std::fs::read(&killed_wal_path).unwrap();
        assert!(!wal_bytes.is_empty(), "the commit went straight into the database file");
        std::fs::copy(&killed_index_path, rastai_db_path(&paths)).unwrap();
        std::fs::write(companion_path(&rastai_db_path(&paths), WAL_FILE_SUFFIX), &wal_bytes).unwrap();
        drop(connection);

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert_eq!(report_codes(&report), Vec::<String>::new());
        assert!(rebuilt_token(&paths, CREATOR_TOKEN_ID).revoked_unix_ms.is_none());
        let kept_wal_path = companion_path(&report.old_index_path.unwrap(), WAL_FILE_SUFFIX);
        assert_eq!(std::fs::read(kept_wal_path).unwrap(), wal_bytes);
        assert!(!companion_path(&rastai_db_path(&paths), WAL_FILE_SUFFIX).exists());
        std::fs::remove_dir_all(killed_paths.data.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    fn live_token(token_id: &str, created_by: &str) -> TokenRow {
        TokenRow {
            token_id: token_id.to_string(),
            name: token_id.to_string(),
            token_hash: format!("blake3:{token_id}"),
            scopes: vec!["chat".to_string()],
            created_unix_ms: TOKEN_CREATED_UNIX_MS,
            revoked_unix_ms: None,
            created_by: created_by.to_string(),
        }
    }

    /// An old index whose creator token made a live child, then lost its creator's state by `damage_sql`.
    fn write_damaged_token_index(paths: &DataPaths, damage_sql: &str, damage_parameters: &[SqlValue]) {
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(paths);
        let connection = open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        insert_token(&transaction, &live_token(CREATOR_TOKEN_ID, "owner")).unwrap();
        let created_by = format!("token:{CREATOR_TOKEN_ID}");
        insert_token(&transaction, &live_token(CHILD_TOKEN_ID, &created_by)).unwrap();
        execute(&transaction, damage_sql, damage_parameters).unwrap();
        commit_transaction(transaction).unwrap();
    }

    /// The rebuilt index's row for `token_id`.
    fn rebuilt_token(paths: &DataPaths, token_id: &str) -> TokenRow {
        let reader = IndexReader {
            connection: open_database_for_reading(&rastai_db_path(paths)).unwrap(),
        };
        let tokens = list_tokens(&reader).unwrap();
        tokens.into_iter().find(|token| token.token_id == token_id).unwrap()
    }

    fn report_codes(report: &RebuildReport) -> Vec<String> {
        report.diagnostics.iter().map(|item| item.code.clone()).collect()
    }

    // A damaged file that kept a revoked creator's child live: the rebuild fails closed and revokes the child too.
    #[test]
    fn a_live_token_of_a_revoked_creator_is_rebuilt_revoked_with_rrs_c1006() {
        let paths = test_paths("token-creator-revoked");
        let parameters = [integer_value(TOKEN_REVOKED_UNIX_MS), text_value(CREATOR_TOKEN_ID)];
        write_damaged_token_index(&paths, REVOKE_CREATOR_ALONE_SQL, &parameters);

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert_eq!(report_codes(&report), [TOKEN_REVOKED_CODE]);
        let child = rebuilt_token(&paths, CHILD_TOKEN_ID);
        assert!(
            child.revoked_unix_ms.is_some(),
            "the child of a revoked creator is live: {child:?}"
        );
        assert_eq!(child.created_by, format!("token:{CREATOR_TOKEN_ID}"));
        let creator = rebuilt_token(&paths, CREATOR_TOKEN_ID);
        assert_eq!(creator.revoked_unix_ms, Some(TOKEN_REVOKED_UNIX_MS));
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A damaged file that lost the creator's row: a creator that is gone admits no one, so the child is revoked.
    #[test]
    fn a_live_token_whose_creator_is_gone_is_rebuilt_revoked_with_rrs_c1006() {
        let paths = test_paths("token-creator-gone");
        write_damaged_token_index(&paths, DELETE_CREATOR_SQL, &[text_value(CREATOR_TOKEN_ID)]);

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert_eq!(report_codes(&report), [TOKEN_REVOKED_CODE]);
        let child = rebuilt_token(&paths, CHILD_TOKEN_ID);
        assert!(
            child.revoked_unix_ms.is_some(),
            "the child of a missing creator is live: {child:?}"
        );
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A child made before its creator, as an old clock can record it: the creator is written first all the same, so
    // the child stays live. Written in list order, the child would be refused and revoked with RRS-C1006.
    #[test]
    fn a_child_token_older_than_its_creator_stays_live() {
        let paths = test_paths("token-child-older");
        std::fs::create_dir_all(&paths.data).unwrap();
        let connection =
            open_database_for_writing(&rastai_db_path(&paths), &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        insert_token(&transaction, &live_token(CREATOR_TOKEN_ID, "owner")).unwrap();
        let mut child = live_token(CHILD_TOKEN_ID, &format!("token:{CREATOR_TOKEN_ID}"));
        child.created_unix_ms = TOKEN_CREATED_UNIX_MS - CHILD_EARLIER_BY_MS;
        insert_token(&transaction, &child).unwrap();
        commit_transaction(transaction).unwrap();
        drop(connection);

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert_eq!(report_codes(&report), Vec::<String>::new());
        assert!(rebuilt_token(&paths, CHILD_TOKEN_ID).revoked_unix_ms.is_none());
        assert!(rebuilt_token(&paths, CREATOR_TOKEN_ID).revoked_unix_ms.is_none());
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A kept copy that is a hard link would share the live index's bytes: a daemon killed before the rename would
    // write its rows into the kept copy too. The kept copy must go on being the old file, whatever the live one gets.
    #[test]
    fn the_kept_copy_does_not_change_when_the_live_index_is_written() {
        let paths = test_paths("kept-copy-own-bytes");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        std::fs::write(&index_path, b"the old index's bytes").unwrap();
        let kept_path = rastai_db_broken_path(&paths, KEPT_UNIX_MS);
        keep_old_file(&index_path, &kept_path).unwrap();
        let kept_bytes = std::fs::read(&kept_path).unwrap();

        let mut live = OpenOptions::new().append(true).open(&index_path).unwrap();
        live.write_all(b" and the rows a daemon wrote after the kill").unwrap();

        assert_eq!(std::fs::read(&kept_path).unwrap(), kept_bytes);
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A link planted at the kept name is replaced by the copy; the file it points at is never written.
    #[cfg(unix)]
    #[test]
    fn a_link_planted_at_the_kept_path_is_replaced_and_its_target_is_left_alone() {
        use std::os::unix::fs::symlink;
        let paths = test_paths("kept-link-planted");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        std::fs::write(&index_path, b"the old index's bytes").unwrap();
        let victim_path = paths.data.join("victim.txt");
        std::fs::write(&victim_path, b"not the index").unwrap();
        let kept_path = rastai_db_broken_path(&paths, KEPT_UNIX_MS);
        symlink(&victim_path, &kept_path).unwrap();

        keep_old_file(&index_path, &kept_path).unwrap();

        assert_eq!(std::fs::read(&victim_path).unwrap(), b"not the index");
        assert!(!std::fs::symlink_metadata(&kept_path).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&kept_path).unwrap(), b"the old index's bytes");
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // A link planted at a scratch path is replaced by the rebuild's own file; the file it points at is never written.
    #[cfg(unix)]
    #[test]
    fn a_link_planted_at_a_scratch_path_is_replaced_and_its_target_is_left_alone() {
        use std::os::unix::fs::symlink;
        let paths = test_paths("scratch-link-planted");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap());
        let victim_path = paths.data.join("victim.txt");
        std::fs::write(&victim_path, b"not the index").unwrap();
        symlink(&victim_path, rastai_db_rebuilding_path(&paths)).unwrap();
        symlink(&victim_path, rastai_db_salvage_path(&paths)).unwrap();

        rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert_eq!(std::fs::read(&victim_path).unwrap(), b"not the index");
        assert!(!std::fs::symlink_metadata(&index_path).unwrap().file_type().is_symlink());
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    // The data folder reached through a symbolic link (a symlinked $HOME or temp dir, the default on macOS) is
    // rebuilt like any other: only the file the rebuild creates is checked, not the folders above it.
    #[cfg(unix)]
    #[test]
    fn a_data_folder_whose_parent_is_a_symbolic_link_is_rebuilt() {
        use std::os::unix::fs::symlink;
        let real_root = std::env::temp_dir().join(format!("rastai-store-rebuild-{}-parent-real", std::process::id()));
        let link_root = std::env::temp_dir().join(format!("rastai-store-rebuild-{}-parent-link", std::process::id()));
        let _ = std::fs::remove_dir_all(&real_root);
        let _ = std::fs::remove_file(&link_root);
        std::fs::create_dir_all(real_root.join("data")).unwrap();
        symlink(&real_root, &link_root).unwrap();
        let paths = DataPaths {
            config: link_root.join("config"),
            data: link_root.join("data"),
            cache: link_root.join("cache"),
            state: link_root.join("state"),
            runtime: link_root.join("runtime"),
        };
        drop(open_database_for_writing(&rastai_db_path(&paths), &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap());

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        assert!(report.old_index_path.is_some());
        assert!(
            !std::fs::symlink_metadata(real_root.join("data").join("rastai.db"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_file(&link_root).unwrap();
        std::fs::remove_dir_all(&real_root).unwrap();
    }

    // The mode a user set on the index (0600 here) is the mode of the rebuilt index and of its kept copy.
    #[cfg(unix)]
    #[test]
    fn a_rebuild_keeps_the_mode_of_the_old_index_and_of_its_kept_copy() {
        use std::os::unix::fs::PermissionsExt;
        let paths = test_paths("keeps-mode");
        std::fs::create_dir_all(&paths.data).unwrap();
        let index_path = rastai_db_path(&paths);
        drop(open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap());
        std::fs::set_permissions(&index_path, std::fs::Permissions::from_mode(PRIVATE_MODE)).unwrap();

        let report = rebuild_index(&paths, &RebuildOptions::default()).unwrap();

        let mode_of = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode_of(&index_path),
            PRIVATE_MODE,
            "the rebuilt index is not the old one's mode"
        );
        assert_eq!(mode_of(&report.old_index_path.unwrap()), PRIVATE_MODE);
        std::fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }
}
