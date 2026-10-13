//! What the rows read and the checks every row ends with (flows/platform.md 6.5, "Checked after every row"):
//!
//! 1. every durable record in every events.log is indexed exactly once: each run's `run_metrics` rows are the log's
//!    metric values, one row each, with no gap and no extra, and each log's ack (a run's `acked_sequence`, the daemon
//!    log's cursor) is its last whole record;
//! 2. nothing half-written is visible: every checkpoint folder under its final name is complete, and the blob
//!    store's `.incoming/` holds nothing;
//! 3. no worker's process group is left;
//! 4. every failure the user sees has a catalog code: failed jobs, unfinished commands, and the diagnostics in the
//!    run logs and the daemon log.
//!
//! The index is read from outside the daemon, through read-only connections (`open_index_readers`), while the
//! daemon runs.

use std::path::Path;

use rastai_core::paths::{DataPaths, blobs_dir, daemon_events_log_path, run_events_log_path};
use rastai_diag::catalog::find_entry;
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, JobState};
use rastai_proto::events::v1::EventRecord;
use rastai_proto::events::v1::event_payload::Payload;
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame};
use rastai_store::index::reads::{list_table_rows, open_index_reader};
use rastai_store::index::runs::{RunRow, get_run};
use rastai_store::store::open_index_readers;

use crate::kill_matrix::processes::live_group_members;
use crate::kill_matrix::scenario::{Scenario, recorded_workers, run_folder};

const RUN_METRICS: &str = "run.metrics";
const RUN_METRICS_TABLE: &str = "run_metrics";
const JOURNAL_TABLE: &str = "command_journal";
const LOG_CURSORS_TABLE: &str = "log_cursors";
const DAEMON_LOG_NAME_PREFIX: &str = "daemon:";
const FAILED_JOB_STATE: &str = "failed";
const CHECKPOINTS_FOLDER: &str = "checkpoints";
/// A checkpoint folder under its final name is complete: it was renamed into place after its last file
/// (rastai_train checkpoints.py). Its resume.json is written last of all.
const CHECKPOINT_LAST_FILE: &str = "resume.json";
const PARTIAL_SUFFIX: &str = ".partial";
const INCOMING_FOLDER: &str = ".incoming";
/// SQL NULL as `list_table_rows` writes it, and the quotes around its text.
const NULL_TEXT: &str = "NULL";
const QUOTE: char = '"';

/// How a log's frames ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogEnd {
    /// Every byte is a whole frame.
    Whole,
    /// The file ends inside a frame: a writer killed mid-write left a torn tail.
    TornTail,
    /// A bad frame before the end, or a file that can't be read.
    Damaged,
}

/// A log's whole records, and how it ended.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRead {
    pub records: Vec<EventRecord>,
    pub end: LogEnd,
}

impl LogRead {
    pub fn last_sequence(&self) -> u64 {
        self.records.last().map(|record| record.sequence).unwrap_or(0)
    }
}

/// A log's records. A log that doesn't exist yet holds none (the daemon log before its first record).
pub fn read_log(path: &Path) -> LogRead {
    let mut records = Vec::new();
    if !path.exists() {
        return LogRead {
            records,
            end: LogEnd::Whole,
        };
    }
    let Ok(mut reader) = open_event_log(path) else {
        return LogRead {
            records,
            end: LogEnd::Damaged,
        };
    };
    loop {
        let end = match read_next_frame(&mut reader) {
            Ok(FrameRead::Record(record)) => {
                records.push(record);
                continue;
            }
            Ok(FrameRead::EndOfFile) => LogEnd::Whole,
            Ok(FrameRead::Incomplete { .. }) => LogEnd::TornTail,
            Ok(FrameRead::Corrupt { .. }) | Err(_) => LogEnd::Damaged,
        };
        return LogRead { records, end };
    }
}

pub fn run_log(paths: &DataPaths, run_id: &str) -> LogRead {
    read_log(&run_events_log_path(paths, run_id))
}

// ---- The index, read from outside the daemon ----

pub fn run_row(paths: &DataPaths, run_id: &str) -> Option<RunRow> {
    let readers = open_index_readers(paths).ok()?;
    let reader = open_index_reader(&readers).ok()?;
    get_run(&reader, run_id).ok()?
}

/// Every row of a table, each column as `list_table_rows` writes it (text quoted, NULL as `NULL`).
pub fn table_rows(paths: &DataPaths, table: &str) -> Vec<Vec<String>> {
    let readers = open_index_readers(paths).unwrap();
    let reader = open_index_reader(&readers).unwrap();
    list_table_rows(&reader, table).unwrap()
}

/// A text column without its quotes; NULL is "".
pub fn unquoted(column: &str) -> String {
    if column == NULL_TEXT {
        return String::new();
    }
    column.trim_matches(QUOTE).to_string()
}

/// The run's `run_metrics` rows as (sequence, name), sorted, every row kept: a duplicate would show twice.
pub fn metric_rows(paths: &DataPaths, run_id: &str) -> Vec<(u64, String)> {
    // Columns: run_id, sequence, step, name, value, emitted_unix_ms (contracts 8.2).
    let mut rows: Vec<(u64, String)> = table_rows(paths, RUN_METRICS_TABLE)
        .into_iter()
        .filter(|row| unquoted(&row[0]) == run_id)
        .map(|row| (row[1].parse().unwrap(), unquoted(&row[3])))
        .collect();
    rows.sort();
    rows
}

/// Every metric value the log holds, as (sequence, name), sorted: what the index must hold, once each.
pub fn logged_metrics(records: &[EventRecord]) -> Vec<(u64, String)> {
    let mut metrics = Vec::new();
    for record in records.iter().filter(|record| record.event_type == RUN_METRICS) {
        if let Some(Payload::RunMetrics(values)) = record.payload.as_ref().and_then(|payload| payload.payload.as_ref())
        {
            for name in values.values.keys() {
                metrics.push((record.sequence, name.clone()));
            }
        }
    }
    metrics.sort();
    metrics
}

/// Waits until the daemon has indexed the run's log to its last whole record and the run's state is `run_state`.
pub fn wait_until_run_indexed(scenario: &Scenario, run_id: &str, run_state: &str) {
    let what = format!("run {run_id} to be indexed to its last record and {run_state}");
    scenario.wait_until(&what, || {
        let last = run_log(&scenario.paths, run_id).last_sequence();
        match run_row(&scenario.paths, run_id) {
            Some(run) => last > 0 && run.acked_sequence == last && run.state == run_state,
            None => false,
        }
    });
}

/// The diagnostic of the `job.state_changed` record that failed the job, from the daemon log.
pub fn job_failure(paths: &DataPaths, job_id: &str) -> Option<ProtoDiagnostic> {
    let mut found = None;
    for record in read_log(&daemon_events_log_path(paths)).records {
        if let Some(Payload::JobStateChanged(changed)) = record.payload.and_then(|payload| payload.payload)
            && changed.job_id == job_id
            && changed.to_state == JobState::Failed as i32
        {
            found = changed.diagnostic;
        }
    }
    found
}

// ---- After every row ----

/// The four checks of the top, for the runs the row made. The daemon that indexed them is still running.
pub fn check_after_row(scenario: &Scenario, run_ids: &[&str]) {
    for run_id in run_ids {
        check_run_indexed_exactly_once(&scenario.paths, run_id);
        check_checkpoints_complete(&scenario.paths, run_id);
        check_run_log_codes(&scenario.paths, run_id);
    }
    check_daemon_log_indexed(&scenario.paths);
    check_daemon_log_codes(&scenario.paths);
    check_failed_jobs_have_codes(&scenario.paths);
    check_unfinished_commands_have_codes(&scenario.paths);
    check_no_incoming_blob(&scenario.paths);
    check_no_worker_group_left(scenario);
}

/// The run's metrics are the log's, one row each, and its ack is the log's last whole record. A torn tail is not a
/// record: nothing of it is indexed.
pub fn check_run_indexed_exactly_once(paths: &DataPaths, run_id: &str) {
    let log = run_log(paths, run_id);
    assert_ne!(log.end, LogEnd::Damaged, "run {run_id}'s log is damaged");
    let expected = logged_metrics(&log.records);
    assert!(!expected.is_empty(), "run {run_id} logged no metric");
    assert_eq!(
        metric_rows(paths, run_id),
        expected,
        "run {run_id}'s run_metrics against its log"
    );
    let sequences: Vec<u64> = log.records.iter().map(|record| record.sequence).collect();
    let in_order: Vec<u64> = (1..=log.last_sequence()).collect();
    assert_eq!(sequences, in_order, "run {run_id}'s log has a gap");
    let run = run_row(paths, run_id).unwrap();
    assert_eq!(
        run.acked_sequence,
        log.last_sequence(),
        "run {run_id}'s ack against its log"
    );
}

/// Every checkpoint folder under its final name holds its last file, and every `checkpoints` row names one of them.
fn check_checkpoints_complete(paths: &DataPaths, run_id: &str) {
    let folder = run_folder(paths, run_id).join(CHECKPOINTS_FOLDER);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(PARTIAL_SUFFIX) {
            continue;
        }
        assert!(
            entry.path().join(CHECKPOINT_LAST_FILE).is_file(),
            "checkpoint {} is visible but incomplete",
            entry.path().display()
        );
    }
    // Columns: checkpoint_id, run_id, step, folder, node_id, is_best, pinned (contracts 8.2).
    for row in table_rows(paths, "checkpoints") {
        if unquoted(&row[1]) == *run_id {
            let checkpoint = run_folder(paths, run_id).join(unquoted(&row[3]));
            assert!(
                checkpoint.join(CHECKPOINT_LAST_FILE).is_file(),
                "the index names checkpoint {}, which isn't complete",
                checkpoint.display()
            );
        }
    }
}

/// Every diagnostic a run's log carries has a catalog code.
fn check_run_log_codes(paths: &DataPaths, run_id: &str) {
    for record in run_log(paths, run_id).records {
        if let Some(Payload::DiagnosticReported(reported)) = record.payload.and_then(|payload| payload.payload) {
            let code = reported
                .diagnostic
                .map(|diagnostic| diagnostic.code)
                .unwrap_or_default();
            assert_catalog_code(&code, &format!("a diagnostic in run {run_id}'s log"));
        }
    }
}

/// The daemon log's cursor is its last record: every record the daemon logged is indexed, once (the cursor skips
/// what it already covers).
fn check_daemon_log_indexed(paths: &DataPaths) {
    let log = read_log(&daemon_events_log_path(paths));
    assert_eq!(log.end, LogEnd::Whole, "the daemon log after a restart");
    // Columns: log_name, acked_sequence (contracts 8.2). One segment: the rows never roll the log.
    let cursors: Vec<u64> = table_rows(paths, LOG_CURSORS_TABLE)
        .into_iter()
        .filter(|row| unquoted(&row[0]).starts_with(DAEMON_LOG_NAME_PREFIX))
        .map(|row| row[1].parse().unwrap())
        .collect();
    assert!(cursors.len() <= 1, "more than one daemon log segment: {cursors:?}");
    let indexed = cursors.first().copied().unwrap_or(0);
    assert_eq!(indexed, log.last_sequence(), "the daemon log's cursor against the log");
}

/// Every failed job and every run state the daemon logged with a diagnostic carries a catalog code.
fn check_daemon_log_codes(paths: &DataPaths) {
    for record in read_log(&daemon_events_log_path(paths)).records {
        let diagnostic = match record.payload.and_then(|payload| payload.payload) {
            Some(Payload::JobStateChanged(changed)) => changed.diagnostic,
            Some(Payload::DiagnosticReported(reported)) => reported.diagnostic,
            _ => None,
        };
        if let Some(diagnostic) = diagnostic {
            assert_catalog_code(&diagnostic.code, "a diagnostic in the daemon log");
        }
    }
}

/// `jobs` rows that failed carry their code.
fn check_failed_jobs_have_codes(paths: &DataPaths) {
    // Columns: job_id, run_id, node_id, kind, state, pid, process_group_id, exit_code, error_code (contracts 8.2).
    for row in table_rows(paths, "jobs") {
        if unquoted(&row[4]) == FAILED_JOB_STATE {
            assert_catalog_code(&unquoted(&row[8]), &format!("failed job {}", row[0]));
        }
    }
}

/// Journal rows that failed, or that a restart closed, carry their code.
fn check_unfinished_commands_have_codes(paths: &DataPaths) {
    // Columns: journal_id, command_id, principal, arguments_json, ok, code, ... (contracts 8.2).
    for row in table_rows(paths, JOURNAL_TABLE) {
        assert_ne!(row[4], NULL_TEXT, "journal row {} was never finished or closed", row[0]);
        if row[4] == "0" {
            assert_catalog_code(&unquoted(&row[5]), &format!("journal row {}", row[0]));
        }
    }
}

fn check_no_incoming_blob(paths: &DataPaths) {
    let incoming = blobs_dir(paths).join(INCOMING_FOLDER);
    let leftovers: Vec<String> = match std::fs::read_dir(&incoming) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => Vec::new(),
    };
    assert!(
        leftovers.is_empty(),
        "half-written blobs in {}: {leftovers:?}",
        incoming.display()
    );
}

/// No process is left in any worker's group. A group just killed may take a moment to go, so this waits.
pub fn check_no_worker_group_left(scenario: &Scenario) {
    for worker in recorded_workers(&scenario.paths) {
        let what = format!("worker group {} to be gone", worker.process_group_id);
        scenario.wait_until(&what, || live_group_members(worker.process_group_id).is_empty());
    }
}

pub fn assert_catalog_code(code: &str, what: &str) {
    assert!(!code.is_empty(), "{what} has no code");
    assert!(
        find_entry(code).is_some(),
        "{what} has {code}, which isn't in the catalog"
    );
}
