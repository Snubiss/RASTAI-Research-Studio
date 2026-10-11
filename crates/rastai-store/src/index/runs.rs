//! Rows of `runs` and `jobs` (contracts 8.2). The daemon upserts them when it starts a run or a job; the indexer
//! (M1-25) adds `run_metrics`, `run_tags` and `checkpoints` from the event logs, and moves `runs.acked_sequence`.

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{
    SqlValue, SqliteTransaction, boolean_value, execute, integer_value, optional_integer_value, query_rows,
    read_boolean, read_integer, read_optional_integer, read_text, read_unsigned, text_value, unsigned_value,
};

// archive_state is left out of the insert, so a new run takes the column default, 'local', and an update never
// touches it: only the archive (M11) moves it, through its own writes (contracts 9.9).
// acked_sequence only moves forward, so an upsert built from an older copy of the row can't un-index records.
const UPSERT_RUN: &str = "INSERT INTO runs (run_id, kind, state, project_id, node_id, parent_run_id, record_hash,
        acked_sequence, created_unix_ms, finished_unix_ms, pinned)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
    ON CONFLICT (run_id) DO UPDATE SET kind = excluded.kind, state = excluded.state, project_id = excluded.project_id,
        node_id = excluded.node_id, parent_run_id = excluded.parent_run_id, record_hash = excluded.record_hash,
        acked_sequence = MAX(runs.acked_sequence, excluded.acked_sequence), created_unix_ms = excluded.created_unix_ms,
        finished_unix_ms = excluded.finished_unix_ms, pinned = excluded.pinned";
// Both selects list the columns in the order run_from_columns reads them.
const SELECT_RUN_BY_ID: &str = "SELECT run_id, kind, state, project_id, node_id, parent_run_id, record_hash,
        acked_sequence, created_unix_ms, finished_unix_ms, pinned, archive_state
    FROM runs WHERE run_id = ?1";
// An empty filter field matches every run; LIMIT -1 is SQLite's "no limit". Newest first.
const SELECT_RUNS_BY_FILTER: &str = "SELECT run_id, kind, state, project_id, node_id, parent_run_id, record_hash,
        acked_sequence, created_unix_ms, finished_unix_ms, pinned, archive_state
    FROM runs
    WHERE (?1 = '' OR project_id = ?1) AND (?2 = '' OR state = ?2) AND (?3 = '' OR kind = ?3)
    ORDER BY created_unix_ms DESC, run_id DESC LIMIT ?4";
const NO_LIMIT: i64 = -1;

const UPSERT_JOB: &str = "INSERT INTO jobs (job_id, run_id, node_id, kind, state, pid, process_group_id, exit_code,
        error_code)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
    ON CONFLICT (job_id) DO UPDATE SET run_id = excluded.run_id, node_id = excluded.node_id, kind = excluded.kind,
        state = excluded.state, pid = excluded.pid, process_group_id = excluded.process_group_id,
        exit_code = excluded.exit_code, error_code = excluded.error_code";
const SELECT_JOB_BY_ID: &str = "SELECT job_id, run_id, node_id, kind, state, pid, process_group_id, exit_code,
    error_code FROM jobs WHERE job_id = ?1";

/// One row of `runs`. Ids a run may lack (`project_id`, `parent_run_id`) are "".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub run_id: String,
    pub kind: String,
    pub state: String,
    pub project_id: String,
    pub node_id: String,
    pub parent_run_id: String,
    pub record_hash: String,
    pub acked_sequence: u64,
    pub created_unix_ms: i64,
    pub finished_unix_ms: Option<i64>,
    pub pinned: bool,
    /// `local`, `archived` or `pruned` (contracts 9.9). Read back only: `UpsertRun` never writes it.
    pub archive_state: String,
}

/// One row of `jobs`. `run_id` is "" for a job without a run; the process numbers are `None` until known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub job_id: String,
    pub run_id: String,
    pub node_id: String,
    pub kind: String,
    pub state: String,
    pub pid: Option<i64>,
    pub process_group_id: Option<i64>,
    pub exit_code: Option<i64>,
    pub error_code: String,
}

/// Which runs `list_runs` returns. An empty field matches every run; `limit` `None` returns them all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunFilter {
    pub project_id: String,
    pub state: String,
    pub kind: String,
    pub limit: Option<u32>,
}

pub(crate) fn upsert_run(transaction: &SqliteTransaction<'_>, row: &RunRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.run_id),
        text_value(&row.kind),
        text_value(&row.state),
        text_value(&row.project_id),
        text_value(&row.node_id),
        text_value(&row.parent_run_id),
        text_value(&row.record_hash),
        unsigned_value(row.acked_sequence)?,
        integer_value(row.created_unix_ms),
        optional_integer_value(row.finished_unix_ms),
        boolean_value(row.pinned),
    ];
    execute(transaction, UPSERT_RUN, &parameters)?;
    Ok(())
}

pub(crate) fn upsert_job(transaction: &SqliteTransaction<'_>, row: &JobRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.job_id),
        text_value(&row.run_id),
        text_value(&row.node_id),
        text_value(&row.kind),
        text_value(&row.state),
        optional_integer_value(row.pid),
        optional_integer_value(row.process_group_id),
        optional_integer_value(row.exit_code),
        text_value(&row.error_code),
    ];
    execute(transaction, UPSERT_JOB, &parameters)?;
    Ok(())
}

pub fn get_run(reader: &IndexReader, run_id: &str) -> Result<Option<RunRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_RUN_BY_ID, &[text_value(run_id)])?;
    match rows.first() {
        Some(row) => Ok(Some(run_from_columns(row)?)),
        None => Ok(None),
    }
}

/// Runs that match the filter, newest first.
pub fn list_runs(reader: &IndexReader, filter: &RunFilter) -> Result<Vec<RunRow>, StoreError> {
    let limit = match filter.limit {
        Some(limit) => i64::from(limit),
        None => NO_LIMIT,
    };
    let parameters = [
        text_value(&filter.project_id),
        text_value(&filter.state),
        text_value(&filter.kind),
        integer_value(limit),
    ];
    let rows = query_rows(&reader.connection, SELECT_RUNS_BY_FILTER, &parameters)?;
    let mut runs = Vec::with_capacity(rows.len());
    for row in &rows {
        runs.push(run_from_columns(row)?);
    }
    Ok(runs)
}

pub fn get_job(reader: &IndexReader, job_id: &str) -> Result<Option<JobRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_JOB_BY_ID, &[text_value(job_id)])?;
    match rows.first() {
        Some(row) => Ok(Some(job_from_columns(row)?)),
        None => Ok(None),
    }
}

/// Columns in the order of `SELECT_RUN_BY_ID`.
fn run_from_columns(row: &[SqlValue]) -> Result<RunRow, StoreError> {
    Ok(RunRow {
        run_id: read_text(row, 0)?,
        kind: read_text(row, 1)?,
        state: read_text(row, 2)?,
        project_id: read_text(row, 3)?,
        node_id: read_text(row, 4)?,
        parent_run_id: read_text(row, 5)?,
        record_hash: read_text(row, 6)?,
        acked_sequence: read_unsigned(row, 7)?,
        created_unix_ms: read_integer(row, 8)?,
        finished_unix_ms: read_optional_integer(row, 9)?,
        pinned: read_boolean(row, 10)?,
        archive_state: read_text(row, 11)?,
    })
}

/// Columns in the order of `SELECT_JOB_BY_ID`.
fn job_from_columns(row: &[SqlValue]) -> Result<JobRow, StoreError> {
    Ok(JobRow {
        job_id: read_text(row, 0)?,
        run_id: read_text(row, 1)?,
        node_id: read_text(row, 2)?,
        kind: read_text(row, 3)?,
        state: read_text(row, 4)?,
        pid: read_optional_integer(row, 5)?,
        process_group_id: read_optional_integer(row, 6)?,
        exit_code: read_optional_integer(row, 7)?,
        error_code: read_text(row, 8)?,
    })
}
