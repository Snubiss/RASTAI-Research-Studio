//! Rows of `command_journal` (contracts 8.2): one per executed command, written when it starts. The indexer fills
//! the finished columns from the daemon log's `command.finished` record; a restart marks the rows it interrupted
//! with `RRS-C0005`.

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{
    SqlValue, SqliteTransaction, boolean_value, execute, integer_value, optional_text_value, query_rows, read_integer,
    read_optional_boolean, read_optional_integer, read_text, text_value,
};

// The warning a command interrupted by a daemon restart finishes with (contracts 10.3).
const CODE_INTERRUPTED_BY_RESTART: &str = "RRS-C0005";

const INSERT_JOURNAL_STARTED: &str = "INSERT INTO command_journal (journal_id, command_id, principal, arguments_json,
        idempotency_key, started_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const MARK_UNFINISHED_ROWS: &str = "UPDATE command_journal SET ok = 0, code = ?1, finished_unix_ms = ?2
    WHERE finished_unix_ms IS NULL";
// A rebuild has no started rows, so a command.finished record makes the row (contracts 8.2: commands that never
// finished are not rebuilt). It starts and finishes at the record's time, with the arguments the record doesn't
// carry written as none. A row the daemon wrote at the start keeps its arguments and start.
const RECORD_COMMAND_FINISHED: &str = "INSERT INTO command_journal (journal_id, command_id, principal, arguments_json,
        ok, code, started_unix_ms, finished_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
    ON CONFLICT (journal_id) DO UPDATE SET ok = excluded.ok, code = excluded.code,
        finished_unix_ms = excluded.finished_unix_ms";
const ARGUMENTS_NOT_RECORDED: &str = "{}";

// Both selects list the columns in the order journal_from_columns reads them.
// The newest row wins: whether it is still inside the key's 24 hours is the server's call (flows/platform.md 3.2).
const SELECT_BY_IDEMPOTENCY_KEY: &str = "SELECT journal_id, command_id, principal, arguments_json, ok, code,
        idempotency_key, result_json, job_id, started_unix_ms, finished_unix_ms
    FROM command_journal WHERE principal = ?1 AND idempotency_key = ?2
    ORDER BY started_unix_ms DESC, journal_id DESC LIMIT 1";
const SELECT_BY_JOURNAL_ID: &str = "SELECT journal_id, command_id, principal, arguments_json, ok, code,
        idempotency_key, result_json, job_id, started_unix_ms, finished_unix_ms
    FROM command_journal WHERE journal_id = ?1";

/// What is known when a command starts. `idempotency_key` is "" when the caller sent none. `arguments_json` must
/// already have its `secret` and `content` fields redacted (contracts 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalStartedRow {
    pub journal_id: String,
    pub command_id: String,
    pub principal: String,
    pub arguments_json: String,
    pub idempotency_key: String,
    pub started_unix_ms: i64,
}

/// A whole journal row. Until the command finishes, `ok` and `finished_unix_ms` are `None` and the other finished
/// columns are "". `result_json` is written with the result's `secret` and `content` fields already redacted, like
/// `arguments_json` (contracts 5.1), so an idempotent replay returns the redacted result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRow {
    pub journal_id: String,
    pub command_id: String,
    pub principal: String,
    pub arguments_json: String,
    pub ok: Option<bool>,
    pub code: String,
    pub idempotency_key: String,
    pub result_json: String,
    pub job_id: String,
    pub started_unix_ms: i64,
    pub finished_unix_ms: Option<i64>,
}

pub(crate) fn insert_journal_started(
    transaction: &SqliteTransaction<'_>,
    row: &JournalStartedRow,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.journal_id),
        text_value(&row.command_id),
        text_value(&row.principal),
        text_value(&row.arguments_json),
        idempotency_key_value(&row.idempotency_key),
        integer_value(row.started_unix_ms),
    ];
    execute(transaction, INSERT_JOURNAL_STARTED, &parameters)?;
    Ok(())
}

/// At startup, before anything is served: every row still without a finish belongs to a command the last daemon
/// never finished, so it is closed as failed with `RRS-C0005`.
pub(crate) fn mark_unfinished_journal_rows(
    transaction: &SqliteTransaction<'_>,
    finished_unix_ms: i64,
) -> Result<(), StoreError> {
    let parameters = [text_value(CODE_INTERRUPTED_BY_RESTART), integer_value(finished_unix_ms)];
    execute(transaction, MARK_UNFINISHED_ROWS, &parameters)?;
    Ok(())
}

/// A command finished, as its `command.finished` record says. `code` is "" when it succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinishedCommand<'a> {
    pub(crate) journal_id: &'a str,
    pub(crate) command_id: &'a str,
    pub(crate) principal: &'a str,
    pub(crate) ok: bool,
    pub(crate) code: &'a str,
    pub(crate) finished_unix_ms: i64,
}

pub(crate) fn record_command_finished(
    transaction: &SqliteTransaction<'_>,
    finished: &FinishedCommand<'_>,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(finished.journal_id),
        text_value(finished.command_id),
        text_value(finished.principal),
        text_value(ARGUMENTS_NOT_RECORDED),
        boolean_value(finished.ok),
        optional_text_value(finished.code),
        integer_value(finished.finished_unix_ms),
    ];
    execute(transaction, RECORD_COMMAND_FINISHED, &parameters)?;
    Ok(())
}

/// The newest journal row of this principal with this key, or `None`. An empty key never matches.
pub fn find_journal_by_idempotency_key(
    reader: &IndexReader,
    principal: &str,
    key: &str,
) -> Result<Option<JournalRow>, StoreError> {
    if key.is_empty() {
        return Ok(None);
    }
    let parameters = [text_value(principal), text_value(key)];
    let rows = query_rows(&reader.connection, SELECT_BY_IDEMPOTENCY_KEY, &parameters)?;
    first_journal_row(&rows)
}

pub fn get_journal(reader: &IndexReader, journal_id: &str) -> Result<Option<JournalRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_BY_JOURNAL_ID, &[text_value(journal_id)])?;
    first_journal_row(&rows)
}

/// No key is stored as NULL, so the partial index on keys stays small and "" can never match a lookup.
fn idempotency_key_value(key: &str) -> SqlValue {
    if key.is_empty() {
        return SqlValue::Null;
    }
    text_value(key)
}

fn first_journal_row(rows: &[Vec<SqlValue>]) -> Result<Option<JournalRow>, StoreError> {
    match rows.first() {
        Some(row) => Ok(Some(journal_from_columns(row)?)),
        None => Ok(None),
    }
}

/// Columns in the order of `SELECT_BY_JOURNAL_ID`.
fn journal_from_columns(row: &[SqlValue]) -> Result<JournalRow, StoreError> {
    Ok(JournalRow {
        journal_id: read_text(row, 0)?,
        command_id: read_text(row, 1)?,
        principal: read_text(row, 2)?,
        arguments_json: read_text(row, 3)?,
        ok: read_optional_boolean(row, 4)?,
        code: read_text(row, 5)?,
        idempotency_key: read_text(row, 6)?,
        result_json: read_text(row, 7)?,
        job_id: read_text(row, 8)?,
        started_unix_ms: read_integer(row, 9)?,
        finished_unix_ms: read_optional_integer(row, 10)?,
    })
}
