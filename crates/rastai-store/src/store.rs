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
pub(crate) const INDEX_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("index/migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("index/migrations/0002_journal_arguments_hash.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("index/migrations/0003_token_creator.sql"),
    },
];

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

/// Read-only connections to the `rastai.db` a running daemon has open, for a process that isn't that daemon: the
/// kill -9 tests (`tests/crash/`) watch a daemon process's index this way. It never creates, migrates or writes the
/// file, so it can't be a second writer; while the daemon runs, WAL lets these read beside its writer thread.
pub fn open_index_readers(paths: &DataPaths) -> Result<ReaderPool, StoreError> {
    open_reader_pool(&rastai_db_path(paths), INDEX_READER_COUNT)
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
    use crate::sqlite::{
        SqliteTransaction, begin_transaction, commit_transaction, execute, integer_value, optional_integer_value,
        text_value,
    };

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
        // A newer app is this one plus a migration: it migrates the file one version further and closes it cleanly.
        let mut newer_migrations: Vec<Migration> = INDEX_MIGRATIONS
            .iter()
            .map(|migration| Migration {
                version: migration.version,
                sql: migration.sql,
            })
            .collect();
        newer_migrations.push(Migration {
            version: i64::try_from(INDEX_MIGRATIONS.len()).unwrap() + 1,
            sql: NEWER_MIGRATION_SQL,
        });
        drop(open_database_for_writing(&database_path, &INDEX_DATABASE_SETTINGS, &newer_migrations).unwrap());
        let bytes_before = std::fs::read(&database_path).unwrap();

        let error = open_store(&paths).err().unwrap();

        assert_eq!(error.code(), "RRS-S0002");
        assert_eq!(std::fs::read(&database_path).unwrap(), bytes_before);
        if let Some(root) = paths.data.parent() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    // ---- Migration 0003 on a file that already has tokens (contracts 8.2 "Token creators and revocation") ----
    // Here too, not in tests/index_migrations.rs: a file at version 2 is made with the crate's own migration functions.

    /// The version before `0003_token_creator.sql`.
    const VERSION_BEFORE_TOKEN_CREATOR: usize = 2;
    const INSERT_OLD_TOKEN: &str = "INSERT INTO tokens (token_id, name, token_hash, scopes, created_unix_ms,
            revoked_unix_ms)
        VALUES (?1, ?1, 'blake3:' || ?1, '[\"system.admin\"]', 1, ?2)";
    const INSERT_JOURNAL_ROW: &str = "INSERT INTO command_journal (journal_id, command_id, principal, arguments_json,
            ok, result_json, started_unix_ms, finished_unix_ms)
        VALUES (?1, ?2, ?3, '{}', ?4, ?5, ?6, ?6)";
    const INSERT_OLD_PLUGIN: &str = "INSERT INTO plugins (plugin_id, version, state, content_hash,
            granted_permissions, source)
        VALUES (?1, '1.0.0', ?2, 'blake3:00', '[]', 'installed')";
    const MIGRATION_TIME_FLOOR_MS: i64 = 1_000_000_000_000;
    const JOURNAL_STARTED_UNIX_MS: i64 = 1;

    /// One journal row of a `token.create` as `execute_command` writes it: the result keeps `token_id` and redacts
    /// the token's text.
    fn journal_create(
        transaction: &SqliteTransaction<'_>,
        journal_id: &str,
        principal: &str,
        ok: bool,
        result_json: &str,
    ) {
        let parameters = [
            text_value(journal_id),
            text_value("token.create"),
            text_value(principal),
            integer_value(i64::from(ok)),
            text_value(result_json),
            integer_value(JOURNAL_STARTED_UNIX_MS),
        ];
        execute(transaction, INSERT_JOURNAL_ROW, &parameters).unwrap();
    }

    fn created_result(token_id: &str) -> String {
        format!("{{\"token_id\":\"{token_id}\",\"name\":\"x\",\"scopes\":[],\"token\":\"<redacted>\"}}")
    }

    /// A version-2 file as a daemon before SEC-01 left it: tokens with no creator column, the journal rows that
    /// made them, and two plugins.
    fn write_version_two_file(database_path: &Path) {
        let connection = open_database_for_writing(
            database_path,
            &INDEX_DATABASE_SETTINGS,
            &INDEX_MIGRATIONS[..VERSION_BEFORE_TOKEN_CREATOR],
        )
        .unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        // (token, revoked at, principal of its token.create)
        let tokens: [(&str, Option<i64>, &str); 7] = [
            ("tok_a", None, "owner"),
            ("tok_b", None, "token:tok_a"),
            ("tok_r", Some(5), "owner"),
            ("tok_s", None, "token:tok_r"),
            ("tok_t", None, "token:tok_s"),
            ("tok_p", None, "plugin:acme_gone"),
            ("tok_q", None, "plugin:acme_live"),
        ];
        for (token_id, revoked_unix_ms, principal) in tokens {
            let parameters = [text_value(token_id), optional_integer_value(revoked_unix_ms)];
            execute(&transaction, INSERT_OLD_TOKEN, &parameters).unwrap();
            journal_create(
                &transaction,
                &format!("jrn_{token_id}"),
                principal,
                true,
                &created_result(token_id),
            );
        }
        // tok_n has no successful row: a failed one names it, and a result that isn't JSON is passed over.
        execute(
            &transaction,
            INSERT_OLD_TOKEN,
            &[text_value("tok_n"), optional_integer_value(None)],
        )
        .unwrap();
        journal_create(
            &transaction,
            "jrn_failed",
            "token:tok_r",
            false,
            &created_result("tok_n"),
        );
        journal_create(&transaction, "jrn_not_json", "token:tok_r", true, "not json");
        for (plugin_id, state) in [("acme_gone", "disabled"), ("acme_live", "enabled")] {
            execute(
                &transaction,
                INSERT_OLD_PLUGIN,
                &[text_value(plugin_id), text_value(state)],
            )
            .unwrap();
        }
        commit_transaction(transaction).unwrap();
    }

    #[test]
    fn migration_0003_fills_created_by_from_the_journal_and_revokes_orphaned_successors() {
        let paths = test_paths("token-creator-migration");
        std::fs::create_dir_all(&paths.data).unwrap();
        write_version_two_file(&rastai_db_path(&paths));

        let store = open_store(&paths).unwrap();
        let tokens = {
            let reader = crate::index::reads::open_index_reader(&store.index_readers).unwrap();
            crate::index::kept_tables::list_tokens(&reader).unwrap()
        };
        close(store);

        let found: Vec<(&str, &str, Option<i64>)> = tokens
            .iter()
            .map(|row| (row.token_id.as_str(), row.created_by.as_str(), row.revoked_unix_ms))
            .collect();
        let migration_time = found
            .iter()
            .find(|(token_id, _, _)| *token_id == "tok_s")
            .and_then(|(_, _, revoked)| *revoked)
            .unwrap();
        assert!(migration_time > MIGRATION_TIME_FLOOR_MS, "{migration_time}");
        let expected: Vec<(&str, &str, Option<i64>)> = vec![
            // Valid tokens stay valid, their creators filled in.
            ("tok_a", "owner", None),
            ("tok_b", "token:tok_a", None),
            // A revoked token keeps its time; what it made, and what that made, is revoked at the migration's time.
            ("tok_n", "", None),
            ("tok_p", "plugin:acme_gone", Some(migration_time)),
            ("tok_q", "plugin:acme_live", None),
            ("tok_r", "owner", Some(5)),
            ("tok_s", "token:tok_r", Some(migration_time)),
            ("tok_t", "token:tok_s", Some(migration_time)),
        ];
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(sorted, expected);
        if let Some(root) = paths.data.parent() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    // The kill -9 tests read a live daemon's index from another process: the readers read the file its writer made,
    // and a missing file is an error, never a new empty database.
    #[test]
    fn outside_readers_read_the_live_index_and_never_create_one() {
        let paths = test_paths("outside-readers");
        assert!(open_index_readers(&paths).is_err());
        assert!(!rastai_db_path(&paths).exists());
        let store = open_store(&paths).unwrap();
        let readers = open_index_readers(&paths).unwrap();
        let reader = crate::index::reads::open_index_reader(&readers).unwrap();
        let newest_version = INDEX_MIGRATIONS.last().unwrap().version;
        assert_eq!(
            crate::index::reads::read_index_schema_version(&reader).unwrap(),
            newest_version
        );
        drop(reader);
        drop(readers);
        close(store);
        if let Some(root) = paths.data.parent() {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
