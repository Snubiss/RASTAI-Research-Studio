//! `Store`, the handles the daemon keeps for its whole life, and `open_store` / `close_store` (layout section 6.2).
//!
//! M1-23 opens `rastai.db`: the writer connection first (it creates and migrates the file), then the read-only pool,
//! then the writer thread. M1-25 adds the blob store and its startup cleanup of `.incoming/`. The vault, its readers
//! and the daemon log join the struct with their tasks. The caller takes the daemon lock (`take_daemon_lock`) before
//! opening the store.

use std::path::Path;

use rastai_core::paths::{DataPaths, blobs_dir, rastai_db_path};
use tokio::sync::oneshot;

use crate::blobs::blob_store::BlobStore;
use crate::blobs::incoming::remove_stale_incoming_files;
use crate::error::StoreError;
use crate::reader_pool::{ReaderPool, open_reader_pool};
use crate::sqlite::{DatabaseSettings, Migration, open_database_for_writing};
use crate::writer_thread::{IndexWriter, start_index_writer};

/// `rastai.db` is an index: WAL with synchronous NORMAL never corrupts the file, and a power cut loses only commits
/// the event logs restore. Deleted rows hold nothing private, so no secure_delete (layout section 6.3).
pub(crate) const INDEX_DATABASE_SETTINGS: DatabaseSettings = DatabaseSettings {
    synchronous: "NORMAL",
    secure_delete: false,
};

/// The migration chain of `rastai.db`, oldest first. A migration is never edited once released; a change is a new
/// one at the end.
pub(crate) const INDEX_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("index/migrations/0001_initial.sql"),
}];

// Reads are short and the daemon serves a handful of clients, so a few readers keep them from queueing.
const INDEX_READER_COUNT: usize = 4;

pub struct Store {
    pub index: IndexWriter,
    pub index_readers: ReaderPool,
    pub blobs: BlobStore,
    // Fires when the index writer thread has committed its last batch and closed the file.
    index_writer_finished: oneshot::Receiver<()>,
}

/// Opens `rastai.db` (WAL, quick_check, migrations), starts its writer thread and deletes the temporary files a
/// crash left in the blob store's `.incoming/`. Synchronous: called once at startup, before anything is served.
pub fn open_store(paths: &DataPaths) -> Result<Store, StoreError> {
    create_data_folder(&paths.data)?;
    let index_path = rastai_db_path(paths);
    let writer_connection = open_database_for_writing(&index_path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS)?;
    let index_readers = open_reader_pool(&index_path, INDEX_READER_COUNT)?;
    let (index, index_writer_finished) = start_index_writer(writer_connection)?;
    let blobs = BlobStore { root: blobs_dir(paths) };
    remove_stale_incoming_files(&blobs.root);
    Ok(Store {
        index,
        index_readers,
        blobs,
        index_writer_finished,
    })
}

/// Waits until every write already queued is committed and the file is closed. Clones of `store.index` held
/// elsewhere keep the writer thread alive, so drop them first.
pub async fn close_store(store: Store) {
    let Store {
        index,
        index_readers,
        blobs: _,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // The migration a newer app would have added; this build doesn't know it.
    const NEWER_MIGRATION_SQL: &str = "CREATE TABLE added_by_a_newer_app (note TEXT NOT NULL) STRICT;";

    fn test_paths(test_name: &str) -> DataPaths {
        let root = std::env::temp_dir().join(format!("rastai-store-store-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        }
    }

    fn close(store: Store) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(close_store(store));
    }

    // Here rather than in tests/index_migrations.rs: a newer app is modelled with the crate's own migration
    // functions, which are crate-private so that no other crate can run SQL or open a second write connection.
    #[test]
    fn a_newer_schema_version_is_refused_and_the_file_is_unchanged() {
        let paths = test_paths("newer-schema");
        close(open_store(&paths).unwrap());
        let database_path = rastai_db_path(&paths);
        // A newer app is this one plus a migration: it migrates the file to version 2 and closes it cleanly.
        let newer_migrations = [
            Migration {
                version: 1,
                sql: INDEX_MIGRATIONS[0].sql,
            },
            Migration {
                version: 2,
                sql: NEWER_MIGRATION_SQL,
            },
        ];
        drop(open_database_for_writing(&database_path, &INDEX_DATABASE_SETTINGS, &newer_migrations).unwrap());
        let bytes_before = std::fs::read(&database_path).unwrap();

        let error = open_store(&paths).err().unwrap();

        assert_eq!(error.code(), "RRS-S0002");
        assert_eq!(std::fs::read(&database_path).unwrap(), bytes_before);
        if let Some(root) = paths.data.parent() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
