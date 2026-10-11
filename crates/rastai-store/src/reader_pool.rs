//! A few read-only connections per database, each behind a Mutex (layout section 6.1). WAL lets them read while the
//! writer thread writes, so a slow query never holds up a commit, and a reader never sees half a transaction.
//!
//! Readers are handed out round robin; a caller waits only when every reader is busy.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::open_database_for_reading;

// A pool always has at least one reader, so lending one can't fail for want of them.
const MINIMUM_READER_COUNT: usize = 1;

pub struct ReaderPool {
    readers: Vec<Mutex<IndexReader>>,
    // Where the next caller starts looking, so the work spreads over the readers.
    next_reader: AtomicUsize,
}

/// Opens `reader_count` read-only connections to a database its writer has already opened and migrated.
pub(crate) fn open_reader_pool(path: &Path, reader_count: usize) -> Result<ReaderPool, StoreError> {
    let reader_count = reader_count.max(MINIMUM_READER_COUNT);
    let mut readers = Vec::with_capacity(reader_count);
    for _ in 0..reader_count {
        let connection = open_database_for_reading(path)?;
        readers.push(Mutex::new(IndexReader { connection }));
    }
    Ok(ReaderPool {
        readers,
        next_reader: AtomicUsize::new(0),
    })
}

/// Takes the first free reader, starting at the pool's turn; when all are busy, waits for the one whose turn it was.
pub(crate) fn lock_reader(pool: &ReaderPool) -> MutexGuard<'_, IndexReader> {
    let reader_count = pool.readers.len();
    let first = pool.next_reader.fetch_add(1, Ordering::Relaxed) % reader_count;
    for offset in 0..reader_count {
        let reader = &pool.readers[(first + offset) % reader_count];
        match reader.try_lock() {
            Ok(guard) => return guard,
            // A reader whose holder panicked is still a sound read-only connection.
            Err(TryLockError::Poisoned(poisoned)) => return poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => continue,
        }
    }
    match pool.readers[first].lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
