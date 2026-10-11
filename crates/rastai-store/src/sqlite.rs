//! The only file that imports rusqlite (contracts 2.5; layout rust-control-plane.md section 6.6). Everything else in
//! the crate talks to SQLite through these plain functions: open a file with its pragmas and migrations, run a
//! statement with `SqlValue` parameters, read rows back as `SqlValue`s, and frame work in transactions and
//! savepoints. SQLite's errors are sorted here into `StoreError` variants, so each failure gets its catalog code.
//!
//! The SQL text itself lives with the table code in `index/`; nothing outside this crate ever passes SQL in.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, ErrorCode, OpenFlags, params_from_iter};

use crate::error::StoreError;

// Readers wait this long for the writer's checkpoint before giving up (layout section 6.3, `busy_timeout`).
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

// What `PRAGMA quick_check` answers for a healthy file, and what `PRAGMA journal_mode = WAL` answers once set.
const QUICK_CHECK_PASSED: &str = "ok";
const WAL_JOURNAL_MODE: &str = "wal";

const QUICK_CHECK: &str = "PRAGMA quick_check";
const SET_WAL_JOURNAL_MODE: &str = "PRAGMA journal_mode = WAL";
const SET_FOREIGN_KEYS_ON: &str = "PRAGMA foreign_keys = ON";
const COUNT_SCHEMA_MIGRATIONS_TABLES: &str =
    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'";
const SELECT_SCHEMA_VERSION: &str = "SELECT COALESCE(MAX(version), 0) FROM schema_migrations";
const INSERT_SCHEMA_VERSION: &str = "INSERT INTO schema_migrations (version, applied_unix_ms) VALUES (?1, ?2)";

// IMMEDIATE takes the write lock at BEGIN, so a transaction never fails half-way for want of it.
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";
const COMMIT: &str = "COMMIT";
const ROLLBACK: &str = "ROLLBACK";

/// One SQLite value, as a parameter or as a column read back. The five storage classes of SQLite.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// The pragmas that differ between `rastai.db` and `vault.db` (layout section 6.3).
pub(crate) struct DatabaseSettings {
    /// `NORMAL` for the index (the logs restore what a power cut loses), `FULL` for the vault (primary data).
    pub(crate) synchronous: &'static str,
    /// On for the vault, so a deleted conversation doesn't linger in free pages.
    pub(crate) secure_delete: bool,
}

/// One numbered migration: `sql` takes the schema from `version - 1` to `version`. It holds no BEGIN or COMMIT;
/// `open_database_for_writing` runs it in a transaction with its `schema_migrations` row. Crate-private, like all
/// SQL: no other crate can hand SQL to a database (layout section 6.6).
pub(crate) struct Migration {
    pub(crate) version: i64,
    pub(crate) sql: &'static str,
}

/// One open SQLite connection and the file it belongs to. Public only as an opaque handle in the crate's own
/// signatures: its fields are private and only this crate can open one.
pub struct SqliteConnection {
    connection: Connection,
    path: PathBuf,
    // A reader pool's connection: its failures are failed reads (`RRS-S0006`), never failed writes.
    read_only: bool,
}

/// An open `BEGIN IMMEDIATE` transaction on a connection. Every write goes through one, so a row and the rows that
/// belong with it commit together.
pub struct SqliteTransaction<'a> {
    connection: &'a SqliteConnection,
}

/// Opens (or creates) a database for its one writer: checks the schema version and the file's health before
/// anything is written, then sets WAL and the pragmas and applies the migrations the file doesn't have yet.
/// A file migrated by a newer app is refused with `RRS-S0002` and left byte for byte as it was.
///
/// Crate-private: the writer thread owns the one write connection, and no other crate opens one (layout section 6.6).
pub(crate) fn open_database_for_writing(
    path: &Path,
    settings: &DatabaseSettings,
    migrations: &[Migration],
) -> Result<SqliteConnection, StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = open_connection(path, flags, false)?;
    check_schema_version(&connection, migrations)?;
    check_integrity(&connection)?;
    apply_settings(&connection, settings)?;
    apply_pending_migrations(&connection, migrations)?;
    Ok(connection)
}

/// Opens a read-only connection to a database its writer has already opened (and so created and migrated).
pub(crate) fn open_database_for_reading(path: &Path) -> Result<SqliteConnection, StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    open_connection(path, flags, true)
}

/// Starts a write transaction. It must end in `commit_transaction` or `rollback_transaction`.
pub(crate) fn begin_transaction(connection: &SqliteConnection) -> Result<SqliteTransaction<'_>, StoreError> {
    execute_script(connection, BEGIN_IMMEDIATE)?;
    Ok(SqliteTransaction { connection })
}

/// Commits. If the commit fails, the transaction is rolled back, so the connection is ready for the next one.
pub(crate) fn commit_transaction(transaction: SqliteTransaction<'_>) -> Result<(), StoreError> {
    let connection = transaction.connection;
    match execute_script(connection, COMMIT) {
        Ok(()) => Ok(()),
        Err(error) => {
            rollback_transaction(transaction);
            Err(error)
        }
    }
}

/// Rolls back. A failure here means SQLite already ended the transaction itself, so there is nothing left to undo.
pub(crate) fn rollback_transaction(transaction: SqliteTransaction<'_>) {
    abandon_transaction(transaction.connection);
}

/// False once SQLite has ended the transaction on its own. A full disk, an I/O error, running out of memory or a
/// busy file can roll back the whole transaction, not just the statement that failed; every row written in it
/// since BEGIN is gone, and a later statement would run outside any transaction.
pub(crate) fn transaction_is_open(transaction: &SqliteTransaction<'_>) -> bool {
    !transaction.connection.connection.is_autocommit()
}

/// Runs `work` inside a savepoint of the transaction: its rows are kept when it returns Ok and undone when it
/// returns an error, while the rest of the transaction carries on. This is what lets one bad write in a writer
/// batch fail alone (layout section 6.3). `savepoint_name` is one of the crate's own constants, never input.
///
/// The error returned is always the work's own, so a full disk stays `RRS-S3002`. If its rows can't be undone
/// alone, the whole transaction is rolled back instead, so a write that reports a failure never commits; the
/// caller sees that through `transaction_is_open`.
pub(crate) fn run_in_savepoint(
    transaction: &SqliteTransaction<'_>,
    savepoint_name: &str,
    work: impl FnOnce(&SqliteTransaction<'_>) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    let connection = transaction.connection;
    execute_script(connection, &format!("SAVEPOINT {savepoint_name}"))?;
    match work(transaction) {
        Ok(()) => release_savepoint(transaction, savepoint_name),
        Err(error) => {
            undo_savepoint(transaction, savepoint_name);
            Err(error)
        }
    }
}

/// Runs one statement inside a transaction and returns how many rows it changed.
pub(crate) fn execute(
    transaction: &SqliteTransaction<'_>,
    sql: &str,
    parameters: &[SqlValue],
) -> Result<usize, StoreError> {
    let connection = transaction.connection;
    let mut statement = connection
        .connection
        .prepare_cached(sql)
        .map_err(|error| error_from_sqlite(connection, error))?;
    let values = parameters_to_values(parameters);
    statement
        .execute(params_from_iter(values.iter()))
        .map_err(|error| error_from_sqlite(connection, error))
}

/// Runs a query and returns every row, each as its columns in SELECT order.
pub(crate) fn query_rows(
    connection: &SqliteConnection,
    sql: &str,
    parameters: &[SqlValue],
) -> Result<Vec<Vec<SqlValue>>, StoreError> {
    let mut statement = connection
        .connection
        .prepare_cached(sql)
        .map_err(|error| error_from_sqlite(connection, error))?;
    let column_count = statement.column_count();
    let values = parameters_to_values(parameters);
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .map_err(|error| error_from_sqlite(connection, error))?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error_from_sqlite(connection, error))? {
        let mut columns = Vec::with_capacity(column_count);
        for index in 0..column_count {
            let value = row
                .get_ref(index)
                .map_err(|error| error_from_sqlite(connection, error))?;
            columns.push(value_from_sqlite(value)?);
        }
        result.push(columns);
    }
    Ok(result)
}

/// The highest migration version the file has; 0 for a new, empty file.
pub(crate) fn read_schema_version(connection: &SqliteConnection) -> Result<i64, StoreError> {
    let tables = query_rows(connection, COUNT_SCHEMA_MIGRATIONS_TABLES, &[])?;
    if first_integer(&tables)? == 0 {
        return Ok(0);
    }
    let versions = query_rows(connection, SELECT_SCHEMA_VERSION, &[])?;
    first_integer(&versions)
}

// ---- Parameters and columns: the small conversions every table file uses ----

pub(crate) fn text_value(text: &str) -> SqlValue {
    SqlValue::Text(text.to_string())
}

pub(crate) fn integer_value(number: i64) -> SqlValue {
    SqlValue::Integer(number)
}

/// `None` is stored as NULL: a value not known yet, such as a finish time.
pub(crate) fn optional_integer_value(number: Option<i64>) -> SqlValue {
    match number {
        Some(number) => SqlValue::Integer(number),
        None => SqlValue::Null,
    }
}

/// An event-log sequence or a position. SQLite integers are signed 64-bit, so a number past `i64::MAX` (which no
/// log reaches) is refused rather than stored as a negative one.
pub(crate) fn unsigned_value(number: u64) -> Result<SqlValue, StoreError> {
    match i64::try_from(number) {
        Ok(number) => Ok(SqlValue::Integer(number)),
        Err(_) => Err(StoreError::DatabaseWriteFailed {
            reason: format!("{number} is larger than SQLite's largest integer"),
        }),
    }
}

/// SQLite has no boolean; 0 and 1 in an INTEGER column, as the STRICT tables declare.
pub(crate) fn boolean_value(flag: bool) -> SqlValue {
    SqlValue::Integer(i64::from(flag))
}

/// A TEXT column. NULL reads as "", the crate's way of saying "none" in a row struct.
pub(crate) fn read_text(row: &[SqlValue], index: usize) -> Result<String, StoreError> {
    match row.get(index) {
        Some(SqlValue::Text(text)) => Ok(text.clone()),
        Some(SqlValue::Null) => Ok(String::new()),
        other => Err(unexpected_column(index, "text", other)),
    }
}

pub(crate) fn read_integer(row: &[SqlValue], index: usize) -> Result<i64, StoreError> {
    match row.get(index) {
        Some(SqlValue::Integer(number)) => Ok(*number),
        other => Err(unexpected_column(index, "integer", other)),
    }
}

pub(crate) fn read_optional_integer(row: &[SqlValue], index: usize) -> Result<Option<i64>, StoreError> {
    match row.get(index) {
        Some(SqlValue::Integer(number)) => Ok(Some(*number)),
        Some(SqlValue::Null) => Ok(None),
        other => Err(unexpected_column(index, "integer or null", other)),
    }
}

/// An INTEGER column written by `unsigned_value`; a negative number means the file was changed by something else.
pub(crate) fn read_unsigned(row: &[SqlValue], index: usize) -> Result<u64, StoreError> {
    let number = read_integer(row, index)?;
    match u64::try_from(number) {
        Ok(number) => Ok(number),
        Err(_) => Err(StoreError::IndexDamaged {
            reason: format!("column {index} holds {number} where only numbers from 0 up belong"),
        }),
    }
}

pub(crate) fn read_boolean(row: &[SqlValue], index: usize) -> Result<bool, StoreError> {
    Ok(read_integer(row, index)? != 0)
}

pub(crate) fn read_optional_boolean(row: &[SqlValue], index: usize) -> Result<Option<bool>, StoreError> {
    Ok(read_optional_integer(row, index)?.map(|number| number != 0))
}

// ---- Opening ----

fn open_connection(path: &Path, flags: OpenFlags, read_only: bool) -> Result<SqliteConnection, StoreError> {
    let connection = Connection::open_with_flags(path, flags).map_err(|error| open_failed(path, read_only, error))?;
    let connection = SqliteConnection {
        connection,
        path: path.to_path_buf(),
        read_only,
    };
    connection
        .connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| error_from_sqlite(&connection, error))?;
    Ok(connection)
}

/// A reader that can't open its file failed a read (`RRS-S0006`): it never writes, so "not writable" would point
/// the user at the wrong fix. Only the writer's failure to open is `RRS-S0004`.
fn open_failed(path: &Path, read_only: bool, error: rusqlite::Error) -> StoreError {
    let reason = error.to_string();
    if read_only {
        return StoreError::DatabaseReadFailed { reason };
    }
    StoreError::DataDirectoryNotWritable {
        path: path.to_path_buf(),
        reason,
    }
}

/// Refuses a file whose `schema_migrations` holds a version newer than any migration this build has. It only reads,
/// so the refused file is unchanged.
fn check_schema_version(connection: &SqliteConnection, migrations: &[Migration]) -> Result<(), StoreError> {
    let newest_known_version = newest_migration_version(migrations);
    let found_version = read_schema_version(connection)?;
    if found_version > newest_known_version {
        return Err(StoreError::SchemaTooNew {
            path: connection.path.clone(),
            found_version,
            newest_known_version,
        });
    }
    Ok(())
}

fn newest_migration_version(migrations: &[Migration]) -> i64 {
    let mut newest = 0;
    for migration in migrations {
        newest = newest.max(migration.version);
    }
    newest
}

/// `quick_check` answers one row, "ok", for a healthy file, and one row per problem otherwise.
fn check_integrity(connection: &SqliteConnection) -> Result<(), StoreError> {
    let rows = query_rows(connection, QUICK_CHECK, &[])?;
    let mut problems = Vec::new();
    for row in &rows {
        let line = read_text(row, 0)?;
        if line != QUICK_CHECK_PASSED {
            problems.push(line);
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(StoreError::IndexDamaged {
        reason: problems.join("; "),
    })
}

fn apply_settings(connection: &SqliteConnection, settings: &DatabaseSettings) -> Result<(), StoreError> {
    let journal_mode = query_rows(connection, SET_WAL_JOURNAL_MODE, &[])?;
    let mode = match journal_mode.first() {
        Some(row) => read_text(row, 0)?,
        None => String::new(),
    };
    if !mode.eq_ignore_ascii_case(WAL_JOURNAL_MODE) {
        return Err(StoreError::DatabaseWriteFailed {
            reason: format!("SQLite kept journal mode {mode} instead of WAL"),
        });
    }
    execute_script(connection, &format!("PRAGMA synchronous = {}", settings.synchronous))?;
    execute_script(connection, SET_FOREIGN_KEYS_ON)?;
    let secure_delete = if settings.secure_delete { "ON" } else { "OFF" };
    execute_script(connection, &format!("PRAGMA secure_delete = {secure_delete}"))
}

fn apply_pending_migrations(connection: &SqliteConnection, migrations: &[Migration]) -> Result<(), StoreError> {
    let current_version = read_schema_version(connection)?;
    for migration in migrations {
        if migration.version > current_version {
            apply_migration(connection, migration)?;
        }
    }
    Ok(())
}

/// One migration and its `schema_migrations` row, in one transaction: a crash leaves the file before or after it.
fn apply_migration(connection: &SqliteConnection, migration: &Migration) -> Result<(), StoreError> {
    let transaction = begin_transaction(connection)?;
    let applied_unix_ms = rastai_core::time::now_unix_ms();
    let parameters = [integer_value(migration.version), integer_value(applied_unix_ms)];
    let outcome = execute_script(connection, migration.sql)
        .and_then(|()| execute(&transaction, INSERT_SCHEMA_VERSION, &parameters));
    match outcome {
        Ok(_) => commit_transaction(transaction),
        Err(error) => {
            rollback_transaction(transaction);
            Err(error)
        }
    }
}

// ---- Savepoints ----

/// Keeps the savepoint's rows in the transaction. If RELEASE fails, the rows are in an unknown state, so the whole
/// transaction is rolled back rather than risk committing them under an error.
fn release_savepoint(transaction: &SqliteTransaction<'_>, savepoint_name: &str) -> Result<(), StoreError> {
    let connection = transaction.connection;
    let released = execute_script(connection, &format!("RELEASE {savepoint_name}"));
    if released.is_err() {
        abandon_transaction(connection);
    }
    released
}

/// Undoes a failed savepoint's rows and closes it. When SQLite has already ended the whole transaction, the
/// savepoint went with it and there is nothing to undo.
fn undo_savepoint(transaction: &SqliteTransaction<'_>, savepoint_name: &str) {
    if !transaction_is_open(transaction) {
        return;
    }
    let connection = transaction.connection;
    // ROLLBACK TO undoes the rows but keeps the savepoint open; RELEASE then closes it.
    let undone = execute_script(connection, &format!("ROLLBACK TO {savepoint_name}"))
        .and_then(|()| execute_script(connection, &format!("RELEASE {savepoint_name}")));
    if undone.is_err() {
        // The failed write's rows may still be there; committing them would contradict the error its caller gets.
        abandon_transaction(connection);
    }
}

/// Ends the transaction without committing. A failure means SQLite already ended it, so it is ignored.
fn abandon_transaction(connection: &SqliteConnection) {
    let _ = execute_script(connection, ROLLBACK);
}

// ---- Conversions ----

fn execute_script(connection: &SqliteConnection, sql: &str) -> Result<(), StoreError> {
    connection
        .connection
        .execute_batch(sql)
        .map_err(|error| error_from_sqlite(connection, error))
}

fn parameters_to_values(parameters: &[SqlValue]) -> Vec<Value> {
    let mut values = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        let value = match parameter {
            SqlValue::Null => Value::Null,
            SqlValue::Integer(number) => Value::Integer(*number),
            SqlValue::Real(number) => Value::Real(*number),
            SqlValue::Text(text) => Value::Text(text.clone()),
            SqlValue::Blob(bytes) => Value::Blob(bytes.clone()),
        };
        values.push(value);
    }
    values
}

fn value_from_sqlite(value: ValueRef<'_>) -> Result<SqlValue, StoreError> {
    match value {
        ValueRef::Null => Ok(SqlValue::Null),
        ValueRef::Integer(number) => Ok(SqlValue::Integer(number)),
        ValueRef::Real(number) => Ok(SqlValue::Real(number)),
        ValueRef::Text(bytes) => match String::from_utf8(bytes.to_vec()) {
            Ok(text) => Ok(SqlValue::Text(text)),
            Err(_) => Err(StoreError::IndexDamaged {
                reason: "a text column holds bytes that aren't UTF-8".to_string(),
            }),
        },
        ValueRef::Blob(bytes) => Ok(SqlValue::Blob(bytes.to_vec())),
    }
}

fn first_integer(rows: &[Vec<SqlValue>]) -> Result<i64, StoreError> {
    match rows.first() {
        Some(row) => read_integer(row, 0),
        None => Err(StoreError::IndexDamaged {
            reason: "a count query returned no row".to_string(),
        }),
    }
}

/// A column that doesn't hold what this crate wrote there: the file was changed by something else.
fn unexpected_column(index: usize, expected: &str, found: Option<&SqlValue>) -> StoreError {
    StoreError::IndexDamaged {
        reason: format!("column {index} should hold {expected} but holds {found:?}"),
    }
}

/// Sorts SQLite's error codes into the catalog's: a full disk, a damaged file and an unwritable file each have
/// their own code (contracts 10.3). Everything else keeps SQLite's words: under `RRS-S0006` on a reader's
/// connection, which only reads, and under `RRS-S3005` on the writer's, whose every call is part of a write.
/// A reader that can't open or read its file failed a read too: "data folder not writable" is the writer's alone.
fn error_from_sqlite(connection: &SqliteConnection, error: rusqlite::Error) -> StoreError {
    let reason = error.to_string();
    match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => StoreError::DiskFull { reason },
        Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase) => StoreError::IndexDamaged { reason },
        _ if connection.read_only => StoreError::DatabaseReadFailed { reason },
        Some(ErrorCode::CannotOpen) | Some(ErrorCode::ReadOnly) | Some(ErrorCode::PermissionDenied) => {
            StoreError::DataDirectoryNotWritable {
                path: connection.path.clone(),
                reason,
            }
        }
        _ => StoreError::DatabaseWriteFailed { reason },
    }
}

// ---- Test support: a full disk on demand ----

/// How many pages the file has now, for a test that caps it there (`set_max_page_count`).
#[cfg(test)]
pub(crate) fn read_page_count(connection: &SqliteConnection) -> i64 {
    match query_rows(connection, "PRAGMA page_count", &[]) {
        Ok(rows) => first_integer(&rows).unwrap_or(0),
        Err(_) => 0,
    }
}

/// Caps the file at `page_count` pages. A write that needs more gets SQLITE_FULL, exactly as on a full disk, so
/// the tests can reach the full-disk path without filling one. SQLite never caps below the file's current size.
#[cfg(test)]
pub(crate) fn set_max_page_count(connection: &SqliteConnection, page_count: i64) {
    let _ = execute_script(connection, &format!("PRAGMA max_page_count = {page_count}"));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const TEST_SETTINGS: DatabaseSettings = DatabaseSettings {
        synchronous: "NORMAL",
        secure_delete: false,
    };
    const FIRST_MIGRATION: &str =
        "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_unix_ms INTEGER NOT NULL) STRICT;
         CREATE TABLE notes (note_id TEXT PRIMARY KEY, body TEXT NOT NULL) STRICT;";
    const SECOND_MIGRATION: &str = "ALTER TABLE notes ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;";
    // Far more than one page (4 KiB), so the row needs new pages.
    const LARGE_BODY_BYTES: usize = 1 << 20;

    fn test_folder(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-store-sqlite-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn insert_note(transaction: &SqliteTransaction<'_>, note_id: &str) -> Result<(), StoreError> {
        let sql = "INSERT INTO notes (note_id, body) VALUES (?1, 'text')";
        execute(transaction, sql, &[text_value(note_id)]).map(|_| ())
    }

    fn count_notes(connection: &SqliteConnection) -> i64 {
        first_integer(&query_rows(connection, "SELECT count(*) FROM notes", &[]).unwrap()).unwrap()
    }

    #[test]
    fn migrations_apply_once_and_in_order() {
        let folder = test_folder("migrations");
        let path = folder.join("test.db");
        let first = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        drop(open_database_for_writing(&path, &TEST_SETTINGS, &first).unwrap());
        let both = [
            Migration {
                version: 1,
                sql: FIRST_MIGRATION,
            },
            Migration {
                version: 2,
                sql: SECOND_MIGRATION,
            },
        ];
        // Opening twice with both migrations applies the second once; a second ALTER TABLE would fail.
        drop(open_database_for_writing(&path, &TEST_SETTINGS, &both).unwrap());
        let connection = open_database_for_writing(&path, &TEST_SETTINGS, &both).unwrap();
        assert_eq!(read_schema_version(&connection).unwrap(), 2);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_savepoint_keeps_the_rest_of_the_transaction() {
        let folder = test_folder("savepoint");
        let path = folder.join("test.db");
        let migrations = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        let connection = open_database_for_writing(&path, &TEST_SETTINGS, &migrations).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        run_in_savepoint(&transaction, "request", |transaction| insert_note(transaction, "a")).unwrap();
        // The same key again violates the primary key; only this savepoint is undone.
        let duplicate = run_in_savepoint(&transaction, "request", |transaction| insert_note(transaction, "a"));
        assert_eq!(duplicate.unwrap_err().code(), "RRS-S3005");
        run_in_savepoint(&transaction, "request", |transaction| insert_note(transaction, "b")).unwrap();
        commit_transaction(transaction).unwrap();
        assert_eq!(count_notes(&connection), 2);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_full_disk_keeps_its_own_error_and_ends_the_transaction() {
        let folder = test_folder("full-disk");
        let migrations = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        let connection = open_database_for_writing(&folder.join("test.db"), &TEST_SETTINGS, &migrations).unwrap();
        set_max_page_count(&connection, read_page_count(&connection));
        let transaction = begin_transaction(&connection).unwrap();
        run_in_savepoint(&transaction, "request", |transaction| insert_note(transaction, "a")).unwrap();

        // A body far larger than a page needs new pages, which the cap refuses.
        let too_large = run_in_savepoint(&transaction, "request", |transaction| {
            let sql = "INSERT INTO notes (note_id, body) VALUES ('b', ?1)";
            execute(transaction, sql, &[text_value(&"x".repeat(LARGE_BODY_BYTES))]).map(|_| ())
        });

        // The disk-full code, not the savepoint's "no such savepoint" (RRS-S3005).
        assert_eq!(too_large.unwrap_err().code(), "RRS-S3002");
        assert!(!transaction_is_open(&transaction));
        rollback_transaction(transaction);
        assert_eq!(count_notes(&connection), 0);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_reader_failure_is_a_failed_read_and_a_writer_failure_a_failed_write() {
        let folder = test_folder("read-or-write");
        let path = folder.join("test.db");
        let migrations = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        let writer = open_database_for_writing(&path, &TEST_SETTINGS, &migrations).unwrap();
        let reader = open_database_for_reading(&path).unwrap();
        // A table that isn't there fails the same generic way an I/O error or the busy timeout does.
        let missing_table = "SELECT note_id FROM no_such_table";

        let read_error = query_rows(&reader, missing_table, &[]).unwrap_err();
        let write_error = query_rows(&writer, missing_table, &[]).unwrap_err();

        assert_eq!(read_error.code(), "RRS-S0006");
        assert_eq!(write_error.code(), "RRS-S3005");
        drop(reader);
        drop(writer);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_reader_that_cannot_open_or_write_its_file_failed_a_read() {
        let folder = test_folder("reader-open-or-read-only");
        let path = folder.join("test.db");
        let migrations = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        let writer = open_database_for_writing(&path, &TEST_SETTINGS, &migrations).unwrap();
        let reader = open_database_for_reading(&path).unwrap();
        // SQLITE_READONLY on a reader's connection: the reader failed, the data folder is fine.
        let read_only_error = query_rows(&reader, "DELETE FROM notes", &[]).unwrap_err();
        // SQLITE_CANTOPEN while opening a reader: a failed read too, not "data folder not writable".
        let cannot_open_error = open_database_for_reading(&folder.join("missing.db")).err().unwrap();

        assert_eq!(read_only_error.code(), "RRS-S0006");
        assert!(matches!(read_only_error, StoreError::DatabaseReadFailed { .. }));
        assert_eq!(cannot_open_error.code(), "RRS-S0006");
        drop(reader);
        drop(writer);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_database_is_damaged() {
        let folder = test_folder("not-a-database");
        let path = folder.join("test.db");
        std::fs::write(&path, vec![7u8; 8192]).unwrap();
        let migrations = [Migration {
            version: 1,
            sql: FIRST_MIGRATION,
        }];
        let error = open_database_for_writing(&path, &TEST_SETTINGS, &migrations)
            .err()
            .unwrap();
        assert_eq!(error.code(), "RRS-S0003");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn columns_read_back_with_their_types() {
        let row = vec![
            SqlValue::Text("x".to_string()),
            SqlValue::Null,
            SqlValue::Integer(1),
            SqlValue::Real(0.5),
        ];
        assert_eq!(read_text(&row, 0).unwrap(), "x");
        assert_eq!(read_text(&row, 1).unwrap(), "");
        assert_eq!(read_optional_integer(&row, 1).unwrap(), None);
        assert!(read_boolean(&row, 2).unwrap());
        assert_eq!(read_integer(&row, 3).unwrap_err().code(), "RRS-S0003");
        assert_eq!(read_text(&row, 9).unwrap_err().code(), "RRS-S0003");
    }
}
