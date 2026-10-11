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
use crate::sqlite::{SqliteConnection, query_rows, read_schema_version, read_text, text_value};

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

fn list_column_names(reader: &IndexReader, table_name: &str) -> Result<Vec<String>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_COLUMN_NAMES, &[text_value(table_name)])?;
    let mut column_names = Vec::with_capacity(rows.len());
    for row in &rows {
        column_names.push(read_text(row, 0)?);
    }
    Ok(column_names)
}
