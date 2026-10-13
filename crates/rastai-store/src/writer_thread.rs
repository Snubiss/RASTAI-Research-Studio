//! One std thread per database file, owning its only write connection (layout section 6.3).
//!
//! The spike's lesson (07): a `Mutex<Connection>` locked from tokio's blocking pool convoys at 256 clients, because
//! every caller pays for its own transaction while the others queue behind the lock. Here callers only send a
//! request down a channel and await a oneshot. The thread takes every request already waiting (up to
//! `WRITER_BATCH_MAX`) and applies them in one transaction, one commit for the lot, with a savepoint per request so
//! one bad write fails only its own caller. A caller's await returns once its rows are committed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use tokio::sync::oneshot;

use crate::error::StoreError;
use crate::index::writes::{IndexWrite, apply_index_write};
use crate::sqlite::{SqliteConnection, begin_transaction, commit_transaction, run_in_savepoint, transaction_is_open};

/// The most requests one transaction takes; equal to flows/platform.md's `INDEX_BATCH_RECORDS`, so an indexing
/// batch of records and a burst of commands cost the same one commit.
pub const WRITER_BATCH_MAX: usize = 256;

const INDEX_WRITER_THREAD_NAME: &str = "rastai-index-writer";
// The savepoint around each request of a batch. Savepoints nest by name, so a fixed name is enough.
const INDEX_REQUEST_SAVEPOINT: &str = "index_request";

/// The handle callers write `rastai.db` through. Clones share the one thread; the thread ends when the last clone
/// is dropped (`close_store` drops the store's and waits for it).
#[derive(Clone)]
pub struct IndexWriter {
    requests: mpsc::Sender<IndexRequest>,
    // How many batches (one transaction each) the thread has applied: the measure of batching that
    // `writer_batch_count` reports.
    batches_applied: Arc<AtomicU64>,
}

/// One write and the oneshot its caller awaits.
struct IndexRequest {
    write: IndexWrite,
    committed: oneshot::Sender<Result<(), StoreError>>,
}

/// Starts the writer thread on the connection `open_database_for_writing` returned. The receiver fires once the
/// thread has drained every request and closed the connection.
pub(crate) fn start_index_writer(
    connection: SqliteConnection,
) -> Result<(IndexWriter, oneshot::Receiver<()>), StoreError> {
    let (requests, received_requests) = mpsc::channel();
    let (finished, writer_finished) = oneshot::channel();
    let batches_applied = Arc::new(AtomicU64::new(0));
    let thread_batches_applied = Arc::clone(&batches_applied);
    let spawned = std::thread::Builder::new()
        .name(INDEX_WRITER_THREAD_NAME.to_string())
        .spawn(move || run_index_writer(connection, received_requests, thread_batches_applied, finished));
    match spawned {
        Ok(_) => Ok((
            IndexWriter {
                requests,
                batches_applied,
            },
            writer_finished,
        )),
        Err(error) => Err(StoreError::DatabaseWriteFailed {
            reason: format!("the index writer thread didn't start: {error}"),
        }),
    }
}

/// Queues one write and waits until it is committed (or refused). Never blocks a tokio worker: sending on the
/// channel doesn't wait, and the commit is awaited.
pub async fn write_index(writer: &IndexWriter, write: IndexWrite) -> Result<(), StoreError> {
    let (committed, commit_finished) = oneshot::channel();
    let request = IndexRequest { write, committed };
    if writer.requests.send(request).is_err() {
        return Err(StoreError::WriterStopped);
    }
    match commit_finished.await {
        Ok(outcome) => outcome,
        // The thread ended without answering, so the write was never committed.
        Err(_) => Err(StoreError::WriterStopped),
    }
}

/// How many batches the writer thread has applied since it started, each in one transaction. Under load it is far
/// below the number of writes; equal to it would mean every write paid for its own transaction, the convoy.
pub fn writer_batch_count(writer: &IndexWriter) -> u64 {
    writer.batches_applied.load(Ordering::Relaxed)
}

/// The thread's whole life: batch, apply, answer, until every `IndexWriter` is gone.
fn run_index_writer(
    connection: SqliteConnection,
    requests: mpsc::Receiver<IndexRequest>,
    batches_applied: Arc<AtomicU64>,
    finished: oneshot::Sender<()>,
) {
    while let Ok(first) = requests.recv() {
        let batch = drain_more_requests(first, &requests, WRITER_BATCH_MAX);
        let writes: Vec<&IndexWrite> = batch.iter().map(|request| &request.write).collect();
        let outcomes = apply_batch_in_one_transaction(&connection, &writes);
        // Counted before the replies, so a caller that sees its commit also sees its batch counted.
        batches_applied.fetch_add(1, Ordering::Relaxed);
        reply_to_each(batch, outcomes);
    }
    // Close the connection before saying so: the last connection to close checkpoints the WAL into the file.
    drop(connection);
    let _ = finished.send(());
}

/// The first request plus whatever is already waiting, without waiting for more.
fn drain_more_requests(
    first: IndexRequest,
    requests: &mpsc::Receiver<IndexRequest>,
    batch_max: usize,
) -> Vec<IndexRequest> {
    let mut batch = vec![first];
    while batch.len() < batch_max {
        match requests.try_recv() {
            Ok(request) => batch.push(request),
            Err(_) => break,
        }
    }
    batch
}

/// One transaction for the whole batch and one savepoint per write; returns each write's outcome in order. If the
/// transaction can't start or commit, every write in it gets that error, since none of them was committed.
///
/// Some errors (a full disk, an I/O error) make SQLite roll back the whole transaction, not just the write that hit
/// them. The batch stops there: the earlier writes' rows are gone too, and a later write would run outside any
/// transaction and commit on its own. Every write in the batch gets that error, since none of them was committed.
fn apply_batch_in_one_transaction(
    connection: &SqliteConnection,
    writes: &[&IndexWrite],
) -> Vec<Result<(), StoreError>> {
    let transaction = match begin_transaction(connection) {
        Ok(transaction) => transaction,
        Err(error) => return vec![Err(error); writes.len()],
    };
    let mut outcomes = Vec::with_capacity(writes.len());
    for write in writes {
        let outcome = run_in_savepoint(&transaction, INDEX_REQUEST_SAVEPOINT, |transaction| {
            apply_index_write(transaction, write)
        });
        if !transaction_is_open(&transaction) {
            return vec![Err(error_that_ended_the_transaction(outcome)); writes.len()];
        }
        outcomes.push(outcome);
    }
    match commit_transaction(transaction) {
        Ok(()) => outcomes,
        Err(error) => vec![Err(error); writes.len()],
    }
}

/// The error a write got when SQLite ended the transaction under it. A write that succeeded can't end it, but if
/// one ever seemed to, its batch still failed, and the callers are told so.
fn error_that_ended_the_transaction(outcome: Result<(), StoreError>) -> StoreError {
    match outcome {
        Err(error) => error,
        Ok(()) => StoreError::DatabaseWriteFailed {
            reason: "SQLite ended the transaction, so no write in the batch was committed".to_string(),
        },
    }
}

/// Answers every caller. A caller that stopped waiting has dropped its receiver; its rows are committed anyway.
fn reply_to_each(batch: Vec<IndexRequest>, outcomes: Vec<Result<(), StoreError>>) {
    for (request, outcome) in batch.into_iter().zip(outcomes) {
        let _ = request.committed.send(outcome);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::index::kept_tables::{TokenRow, find_token_by_hash};
    use crate::index::reads::open_index_reader;
    use crate::reader_pool::open_reader_pool;
    use crate::sqlite::{open_database_for_writing, read_page_count, set_max_page_count};
    use crate::store::{INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS};

    // Far more than one page (4 KiB), so a token with a name this long needs new pages.
    const LARGE_NAME_BYTES: usize = 1 << 20;
    // Larger than any test file, so the cap no longer bites.
    const NO_PAGE_LIMIT: i64 = 1 << 30;

    fn test_folder(test_name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-store-writer-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn token(token_id: &str) -> TokenRow {
        TokenRow {
            token_id: token_id.to_string(),
            name: "test".to_string(),
            token_hash: format!("blake3:{token_id}"),
            scopes: vec!["read".to_string()],
            created_unix_ms: 1,
            revoked_unix_ms: None,
            created_by: "owner".to_string(),
        }
    }

    #[test]
    fn one_bad_write_in_a_batch_fails_alone() {
        let folder = test_folder("batch");
        let path = folder.join("rastai.db");
        let connection = open_database_for_writing(&path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        // The middle write repeats the first token's id: a token is inserted once, so it is refused.
        let first = IndexWrite::InsertToken(token("tok_a"));
        let duplicate = IndexWrite::InsertToken(TokenRow {
            token_hash: "blake3:other".to_string(),
            ..token("tok_a")
        });
        let last = IndexWrite::InsertToken(token("tok_b"));
        let outcomes = apply_batch_in_one_transaction(&connection, &[&first, &duplicate, &last]);
        assert!(outcomes[0].is_ok());
        assert_eq!(outcomes[1].as_ref().unwrap_err().code(), "RRS-S3005");
        assert!(outcomes[2].is_ok());

        let pool = open_reader_pool(&path, 1).unwrap();
        let reader = open_index_reader(&pool).unwrap();
        assert_eq!(
            find_token_by_hash(&reader, "blake3:tok_a").unwrap(),
            Some(token("tok_a"))
        );
        assert_eq!(
            find_token_by_hash(&reader, "blake3:tok_b").unwrap(),
            Some(token("tok_b"))
        );
        assert_eq!(find_token_by_hash(&reader, "blake3:other").unwrap(), None);
        drop(reader);
        drop(pool);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_full_disk_fails_every_write_of_its_batch() {
        let folder = test_folder("full-disk");
        let path = folder.join("rastai.db");
        let connection = open_database_for_writing(&path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        // Capped at its current size, the file takes a small token in the pages it has, but not a large one.
        set_max_page_count(&connection, read_page_count(&connection));
        let first = IndexWrite::InsertToken(token("tok_a"));
        let too_large = IndexWrite::InsertToken(TokenRow {
            name: "x".repeat(LARGE_NAME_BYTES),
            ..token("tok_large")
        });
        let last = IndexWrite::InsertToken(token("tok_c"));

        let outcomes = apply_batch_in_one_transaction(&connection, &[&first, &too_large, &last]);

        // SQLite ended the transaction at the full disk, so no write was committed and every caller hears so.
        assert_eq!(outcomes.len(), 3);
        for outcome in &outcomes {
            assert_eq!(outcome.as_ref().unwrap_err().code(), "RRS-S3002");
        }
        let pool = open_reader_pool(&path, 1).unwrap();
        let reader = open_index_reader(&pool).unwrap();
        for token_hash in ["blake3:tok_a", "blake3:tok_large", "blake3:tok_c"] {
            assert_eq!(find_token_by_hash(&reader, token_hash).unwrap(), None);
        }

        // With room again, the next batch commits: the connection was left ready for it.
        set_max_page_count(&connection, NO_PAGE_LIMIT);
        let outcomes = apply_batch_in_one_transaction(&connection, &[&last]);
        assert!(outcomes[0].is_ok());
        assert_eq!(
            find_token_by_hash(&reader, "blake3:tok_c").unwrap(),
            Some(token("tok_c"))
        );
        drop(reader);
        drop(pool);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
