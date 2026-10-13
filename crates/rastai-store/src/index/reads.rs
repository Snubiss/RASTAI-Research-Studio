//! Reading `rastai.db`: `IndexReader` (one read-only connection), the guard that lends one out of the pool, and the
//! schema listing. The table files (`runs.rs`, `journal.rs`, ...) hold one read function per query; each takes an
//! `&IndexReader` and a filter or id, never SQL.
//!
//! Reads are blocking: async callers run them in `spawn_blocking` (layout section 6.6).
//!
//! ```text
//! let reader = open_index_reader(&store.index_readers)?;
//! let runs = list_runs(&reader, &filter)?;
//! ```

use std::ops::Deref;
use std::sync::MutexGuard;

use crate::error::StoreError;
use crate::reader_pool::{ReaderPool, lock_reader};
use crate::sqlite::{SqlValue, SqliteConnection, query_rows, read_schema_version, read_text, text_value};

// SQLite's own tables (sqlite_sequence, sqlite_stat1, ...) all start with this prefix and are never ours.
const SQLITE_INTERNAL_TABLE_PATTERN: &str = "sqlite\\_%";
const SELECT_TABLE_NAMES: &str =
    "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE ?1 ESCAPE '\\' ORDER BY name";
const SELECT_COLUMN_NAMES: &str = "SELECT name FROM pragma_table_info(?1) ORDER BY cid";

/// One read-only connection to `rastai.db`. Only the pool makes them.
pub struct IndexReader {
    pub(crate) connection: SqliteConnection,
}

/// One reader lent out of the pool. It derefs to the `IndexReader` and goes back to the pool when dropped.
pub struct IndexReaderGuard<'a> {
    reader: MutexGuard<'a, IndexReader>,
}

impl Deref for IndexReaderGuard<'_> {
    type Target = IndexReader;

    fn deref(&self) -> &IndexReader {
        &self.reader
    }
}

/// A table of the migrated database and its columns in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumns {
    pub table_name: String,
    pub column_names: Vec<String>,
}

/// Every row of one table, each column written as text, sorted by all its columns, so two databases with the same
/// rows give the same list. For comparing whole tables: the indexer's idempotence test and the rebuild's row by row
/// check (contracts 8.2). NULL is `NULL`, text is quoted, a blob is `x'<hex>'`.
///
/// `table_name` must be a table of the database (`list_table_columns` lists them); any other name is refused before
/// it reaches SQL, so the name can't carry SQL of its own.
pub fn list_table_rows(reader: &IndexReader, table_name: &str) -> Result<Vec<Vec<String>>, StoreError> {
    let column_names = list_column_names(reader, table_name)?;
    // No table of ours has a quote in its name, and one would end the quoted name in the SQL below.
    if column_names.is_empty() || table_name.contains('"') {
        return Err(StoreError::DatabaseReadFailed {
            reason: format!("{table_name} is not a table of the database"),
        });
    }
    let mut column_numbers = Vec::with_capacity(column_names.len());
    for number in 1..=column_names.len() {
        column_numbers.push(number.to_string());
    }
    let sql = format!("SELECT * FROM \"{table_name}\" ORDER BY {}", column_numbers.join(", "));
    let rows = query_rows(&reader.connection, &sql, &[])?;
    let mut table_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        table_rows.push(row.iter().map(value_as_text).collect());
    }
    Ok(table_rows)
}

/// One whole table as text: its name, its columns in declaration order and every row as `list_table_rows` writes
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRows {
    pub table_name: String,
    pub column_names: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Every table of the database with every row, sorted by table name: a whole index as text, so two indexes can be
/// compared table by table (the rebuild's row by row check, `rebuilt_table_differences`).
pub fn list_every_table_rows(reader: &IndexReader) -> Result<Vec<TableRows>, StoreError> {
    let mut tables = Vec::new();
    for table in list_table_columns(reader)? {
        let rows = list_table_rows(reader, &table.table_name)?;
        tables.push(TableRows {
            table_name: table.table_name,
            column_names: table.column_names,
            rows,
        });
    }
    Ok(tables)
}

/// Lends one reader from the pool, waiting only when every reader is in use.
pub fn open_index_reader(pool: &ReaderPool) -> Result<IndexReaderGuard<'_>, StoreError> {
    Ok(IndexReaderGuard {
        reader: lock_reader(pool),
    })
}

/// The newest migration the file has (`schema_migrations`), for `system.status` and the tests.
pub fn read_index_schema_version(reader: &IndexReader) -> Result<i64, StoreError> {
    read_schema_version(&reader.connection)
}

/// Every table of the database with its columns, from SQLite itself (`PRAGMA table_info`), sorted by table name.
/// `tests/index_migrations.rs` compares it with contracts 8.2, which `check-names` reads too (contracts 12.6).
pub fn list_table_columns(reader: &IndexReader) -> Result<Vec<TableColumns>, StoreError> {
    let table_rows = query_rows(
        &reader.connection,
        SELECT_TABLE_NAMES,
        &[text_value(SQLITE_INTERNAL_TABLE_PATTERN)],
    )?;
    let mut tables = Vec::with_capacity(table_rows.len());
    for table_row in &table_rows {
        let table_name = read_text(table_row, 0)?;
        let column_names = list_column_names(reader, &table_name)?;
        tables.push(TableColumns {
            table_name,
            column_names,
        });
    }
    Ok(tables)
}

fn value_as_text(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "NULL".to_string(),
        SqlValue::Integer(number) => number.to_string(),
        SqlValue::Real(number) => format!("{number:?}"),
        SqlValue::Text(text) => format!("{text:?}"),
        SqlValue::Blob(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                hex.push_str(&format!("{byte:02x}"));
            }
            format!("x'{hex}'")
        }
    }
}

fn list_column_names(reader: &IndexReader, table_name: &str) -> Result<Vec<String>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_COLUMN_NAMES, &[text_value(table_name)])?;
    let mut column_names = Vec::with_capacity(rows.len());
    for row in &rows {
        column_names.push(read_text(row, 0)?);
    }
    Ok(column_names)
}
