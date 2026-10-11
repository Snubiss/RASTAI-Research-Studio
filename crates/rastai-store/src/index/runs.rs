//! Rows of `runs`, `jobs`, `run_metrics`, `run_tags` and `checkpoints` (contracts 8.2). The daemon upserts runs and
//! jobs when it starts them; the indexer (indexer.rs) changes their states and adds the metrics, tags and
//! checkpoints from the event logs, and moves `runs.acked_sequence`.
//!
//! Every indexer write here is an upsert or an update keyed by ids, so a record applied twice leaves the same rows
//! (layout section 6.4).

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{
    SqlValue, SqliteTransaction, boolean_value, execute, integer_value, optional_integer_value, query_rows,
    query_rows_in_transaction, read_boolean, read_integer, read_optional_integer, read_text, read_unsigned, real_value,
    text_value, unsigned_value,
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

// ---- What the indexer writes (layout section 6.4) ----

// The ack only moves forward, like log_cursors: a late write for an older batch leaves it where it is.
const SET_RUN_ACKED_SEQUENCE: &str = "UPDATE runs SET acked_sequence = MAX(acked_sequence, ?2) WHERE run_id = ?1";
const SELECT_RUN_ACKED_SEQUENCE: &str = "SELECT acked_sequence FROM runs WHERE run_id = ?1";
const SET_RUN_STATE: &str = "UPDATE runs SET state = ?2, finished_unix_ms = ?3 WHERE run_id = ?1";
const SET_RUN_PINNED: &str = "UPDATE runs SET pinned = ?2 WHERE run_id = ?1";
// A job the daemon never recorded (a rebuild has no jobs rows to start from) gets the record's run and '' for the
// node and kind, which the record doesn't carry. A job the daemon recorded keeps its own.
const UPSERT_JOB_STATE: &str = "INSERT INTO jobs (job_id, run_id, node_id, kind, state, exit_code, error_code)
    VALUES (?1, ?2, '', '', ?3, ?4, ?5)
    ON CONFLICT (job_id) DO UPDATE SET state = excluded.state, exit_code = excluded.exit_code,
        error_code = excluded.error_code";
// Keyed by (run_id, sequence, name), so a re-sent record never counts twice (contracts 7.1).
const UPSERT_RUN_METRIC: &str = "INSERT INTO run_metrics (run_id, sequence, step, name, value, emitted_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT (run_id, sequence, name) DO UPDATE SET step = excluded.step, value = excluded.value,
        emitted_unix_ms = excluded.emitted_unix_ms";
const DELETE_RUN_TAGS: &str = "DELETE FROM run_tags WHERE run_id = ?1";
// Tags come from the daemon log, which can name a run whose folder a later run.deleted removed: a rebuild has no
// runs row for it then, so its tags are skipped rather than refused by the foreign key.
const INSERT_RUN_TAG: &str = "INSERT INTO run_tags (run_id, tag)
    SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM runs WHERE run_id = ?1)
    ON CONFLICT (run_id, tag) DO NOTHING";
// The checkpoint is on the run's node. `pinned` is left out of the update: only pin.changed moves it.
const UPSERT_CHECKPOINT: &str = "INSERT INTO checkpoints (checkpoint_id, run_id, step, folder, node_id, is_best)
    VALUES (?1, ?2, ?3, ?4, COALESCE((SELECT node_id FROM runs WHERE run_id = ?2), ''), ?5)
    ON CONFLICT (checkpoint_id) DO UPDATE SET run_id = excluded.run_id, step = excluded.step,
        folder = excluded.folder, node_id = excluded.node_id, is_best = excluded.is_best";
// A run has one best checkpoint: the newest one saved as best.
const CLEAR_OTHER_BEST_CHECKPOINTS: &str =
    "UPDATE checkpoints SET is_best = 0 WHERE run_id = ?1 AND checkpoint_id <> ?2";
const SET_CHECKPOINT_PINNED: &str = "UPDATE checkpoints SET pinned = ?2 WHERE checkpoint_id = ?1";

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

/// A job's state as a `job.state_changed` record reports it. `exit_code` is `None` until the process has exited;
/// `error_code` is "" unless the job failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JobStateUpdate {
    pub(crate) job_id: String,
    pub(crate) run_id: String,
    pub(crate) state: String,
    pub(crate) exit_code: Option<i64>,
    pub(crate) error_code: String,
}

/// One value of a `run.metrics` record: the record's sequence, the step and one metric.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MetricValue<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) sequence: u64,
    pub(crate) step: u64,
    pub(crate) name: &'a str,
    pub(crate) value: f64,
    pub(crate) emitted_unix_ms: i64,
}

/// A checkpoint as `checkpoint.saved` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedCheckpoint<'a> {
    pub(crate) checkpoint_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) step: u64,
    pub(crate) folder: &'a str,
    pub(crate) is_best: bool,
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

/// The run's ack, read inside the writer's transaction; `None` when there is no runs row for it.
pub(crate) fn read_run_acked_sequence(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
) -> Result<Option<u64>, StoreError> {
    let rows = query_rows_in_transaction(transaction, SELECT_RUN_ACKED_SEQUENCE, &[text_value(run_id)])?;
    match rows.first() {
        Some(row) => Ok(Some(read_unsigned(row, 0)?)),
        None => Ok(None),
    }
}

pub(crate) fn set_run_acked_sequence(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    acked_sequence: u64,
) -> Result<(), StoreError> {
    let parameters = [text_value(run_id), unsigned_value(acked_sequence)?];
    execute(transaction, SET_RUN_ACKED_SEQUENCE, &parameters)?;
    Ok(())
}

/// `finished_unix_ms` is `None` while the run goes on, including after a resume.
pub(crate) fn set_run_state(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    state: &str,
    finished_unix_ms: Option<i64>,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(run_id),
        text_value(state),
        optional_integer_value(finished_unix_ms),
    ];
    execute(transaction, SET_RUN_STATE, &parameters)?;
    Ok(())
}

pub(crate) fn set_run_pinned(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    pinned: bool,
) -> Result<(), StoreError> {
    execute(
        transaction,
        SET_RUN_PINNED,
        &[text_value(run_id), boolean_value(pinned)],
    )?;
    Ok(())
}

pub(crate) fn upsert_job_state(transaction: &SqliteTransaction<'_>, update: &JobStateUpdate) -> Result<(), StoreError> {
    let parameters = [
        text_value(&update.job_id),
        text_value(&update.run_id),
        text_value(&update.state),
        optional_integer_value(update.exit_code),
        text_value(&update.error_code),
    ];
    execute(transaction, UPSERT_JOB_STATE, &parameters)?;
    Ok(())
}

pub(crate) fn upsert_run_metric(
    transaction: &SqliteTransaction<'_>,
    metric: &MetricValue<'_>,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(metric.run_id),
        unsigned_value(metric.sequence)?,
        unsigned_value(metric.step)?,
        text_value(metric.name),
        real_value(metric.value),
        integer_value(metric.emitted_unix_ms),
    ];
    execute(transaction, UPSERT_RUN_METRIC, &parameters)?;
    Ok(())
}

/// Replaces the run's tags with `tags`: a `run.tags_changed` record carries the whole set, not the difference.
pub(crate) fn replace_run_tags(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    tags: &[String],
) -> Result<(), StoreError> {
    execute(transaction, DELETE_RUN_TAGS, &[text_value(run_id)])?;
    for tag in tags {
        execute(transaction, INSERT_RUN_TAG, &[text_value(run_id), text_value(tag)])?;
    }
    Ok(())
}

/// Upserts the checkpoint; when it is the new best, the run's other checkpoints stop being best. The run's row must
/// exist: a checkpoint of an unknown run is refused by the foreign key.
pub(crate) fn upsert_checkpoint(
    transaction: &SqliteTransaction<'_>,
    checkpoint: &SavedCheckpoint<'_>,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(checkpoint.checkpoint_id),
        text_value(checkpoint.run_id),
        unsigned_value(checkpoint.step)?,
        text_value(checkpoint.folder),
        boolean_value(checkpoint.is_best),
    ];
    execute(transaction, UPSERT_CHECKPOINT, &parameters)?;
    if checkpoint.is_best {
        let parameters = [text_value(checkpoint.run_id), text_value(checkpoint.checkpoint_id)];
        execute(transaction, CLEAR_OTHER_BEST_CHECKPOINTS, &parameters)?;
    }
    Ok(())
}

pub(crate) fn set_checkpoint_pinned(
    transaction: &SqliteTransaction<'_>,
    checkpoint_id: &str,
    pinned: bool,
) -> Result<(), StoreError> {
    execute(
        transaction,
        SET_CHECKPOINT_PINNED,
        &[text_value(checkpoint_id), boolean_value(pinned)],
    )?;
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
