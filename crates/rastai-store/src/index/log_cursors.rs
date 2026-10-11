//! `log_cursors` (contracts 8.2): how far the daemon's own event log is indexed, like `runs.acked_sequence` is for a
//! run's log. The indexer (M1-25) sets it in the same transaction as the rows of the records it acknowledges, so a
//! crash re-reads at most the records whose rows didn't commit.

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{SqliteTransaction, execute, query_rows, read_unsigned, text_value, unsigned_value};

// The ack only moves forward: a late or repeated write for an older sequence leaves the cursor where it is.
const SET_LOG_CURSOR: &str = "INSERT INTO log_cursors (log_name, acked_sequence) VALUES (?1, ?2)
    ON CONFLICT (log_name) DO UPDATE SET acked_sequence = MAX(log_cursors.acked_sequence, excluded.acked_sequence)";
const SELECT_LOG_CURSOR: &str = "SELECT acked_sequence FROM log_cursors WHERE log_name = ?1";

/// Records that every record of `log_name` up to `acked_sequence` is indexed. The indexer (M1-25) calls it inside
/// the transaction of `IndexWrite::ApplyDaemonEvents`.
pub fn set_log_cursor(
    transaction: &SqliteTransaction<'_>,
    log_name: &str,
    acked_sequence: u64,
) -> Result<(), StoreError> {
    execute(
        transaction,
        SET_LOG_CURSOR,
        &[text_value(log_name), unsigned_value(acked_sequence)?],
    )?;
    Ok(())
}

/// The last indexed sequence of `log_name`, or `None` for a log never indexed.
pub fn get_log_cursor(reader: &IndexReader, log_name: &str) -> Result<Option<u64>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_LOG_CURSOR, &[text_value(log_name)])?;
    match rows.first() {
        Some(row) => Ok(Some(read_unsigned(row, 0)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::index::reads::open_index_reader;
    use crate::reader_pool::open_reader_pool;
    use crate::sqlite::{begin_transaction, commit_transaction, open_database_for_writing};
    use crate::store::{INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS};

    #[test]
    fn the_ack_only_moves_forward() {
        let folder = std::env::temp_dir().join(format!("rastai-store-log-cursors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("rastai.db");
        let connection = open_database_for_writing(&path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        set_log_cursor(&transaction, "daemon:events", 5).unwrap();
        // A late write for an older sequence leaves the ack at 5.
        set_log_cursor(&transaction, "daemon:events", 3).unwrap();
        commit_transaction(transaction).unwrap();

        let pool = open_reader_pool(&path, 1).unwrap();
        let reader = open_index_reader(&pool).unwrap();
        assert_eq!(get_log_cursor(&reader, "daemon:events").unwrap(), Some(5));
        assert_eq!(get_log_cursor(&reader, "daemon:other").unwrap(), None);
    }
}
