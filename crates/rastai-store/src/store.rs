//! `Store`, the handles the daemon keeps for its whole life, and `open_store` / `close_store` (layout section 6.2).
//!
//! M1-23 opens `rastai.db`: the writer connection first (it creates and migrates the file), then the read-only pool,
//! then the writer thread. The vault, its readers, the blob store and the daemon log join the struct with their
//! tasks (M1-24, M1-25, M3). The caller takes the daemon lock (`take_daemon_lock`) before opening the store.

use std::path::Path;

use rastai_core::paths::{DataPaths, rastai_db_path};
use tokio::sync::oneshot;

use crate::error::StoreError;
use crate::reader_pool::{ReaderPool, open_reader_pool};
use crate::sqlite::{DatabaseSettings, Migration, open_database_for_writing};
use crate::writer_thread::{IndexWriter, start_index_writer};

/// `rastai.db` is an index: WAL with synchronous NORMAL never corrupts the file, and a power cut loses only commits
/// the event logs restore. Deleted rows hold nothing private, so no secure_delete (layout section 6.3).
pub const INDEX_DATABASE_SETTINGS: DatabaseSettings = DatabaseSettings {
    synchronous: "NORMAL",
    secure_delete: false,
};

/// The migration chain of `rastai.db`, oldest first. A migration is never edited once released; a change is a new
/// one at the end.
pub const INDEX_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("index/migrations/0001_initial.sql"),
}];

// Reads are short and the daemon serves a handful of clients, so a few readers keep them from queueing.
const INDEX_READER_COUNT: usize = 4;

pub struct Store {
    pub index: IndexWriter,
    pub index_readers: ReaderPool,
    // Fires when the index writer thread has committed its last batch and closed the file.
    index_writer_finished: oneshot::Receiver<()>,
}

/// Opens `rastai.db` (WAL, quick_check, migrations) and starts its writer thread. Synchronous: called once at
/// startup, before anything is served.
pub fn open_store(paths: &DataPaths) -> Result<Store, StoreError> {
    create_data_folder(&paths.data)?;
    let index_path = rastai_db_path(paths);
    let writer_connection = open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS)?;
    let index_readers = open_reader_pool(&index_path, INDEX_READER_COUNT)?;
    let (index, index_writer_finished) = start_index_writer(writer_connection)?;
    Ok(Store {
        index,
        index_readers,
        index_writer_finished,
    })
}

/// Waits until every write already queued is committed and the file is closed. Clones of `store.index` held
/// elsewhere keep the writer thread alive, so drop them first.
pub async fn close_store(store: Store) {
    let Store {
        index,
        index_readers,
        index_writer_finished,
    } = store;
    // Readers close first, so the writer's connection is the last one and checkpoints the WAL into the file.
    drop(index_readers);
    drop(index);
    let _ = index_writer_finished.await;
}

fn create_data_folder(folder: &Path) -> Result<(), StoreError> {
    std::fs::create_dir_all(folder).map_err(|error| StoreError::DataDirectoryNotWritable {
        path: folder.to_path_buf(),
        reason: error.to_string(),
    })
}
