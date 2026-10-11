//! The blob store's public functions (contracts 8.4, flows/platform.md 8.2): put bytes or a file, open, ask for and
//! verify a blob by its hash. Each blob is `<root>/<first two hex digits>/<64 hex digits>` of the BLAKE3 hash of
//! its bytes (`blake3:<64 hex>`), so the same bytes are stored once.
//!
//! - Writes go through `.incoming/` and a rename (incoming.rs). When the blob already exists, the new copy is dropped
//!   and the existing file is touched: its fresh modification time keeps the GC's grace window honest (8.3).
//! - `verify_blob` re-hashes a blob and quarantines it on a mismatch (`RRS-S1001`, quarantine.rs).
//! - A blob that isn't there is `RRS-S1002`.
//!
//! The caller checks free space (`check_free_space`) before a large write: the margin is a setting, and settings are
//! the server's (no policy here).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rastai_formats::hashing::{finish_stream_hasher, new_stream_hasher, update_stream_hasher};
use rastai_proto::common::v1::BlobRef;

use crate::blobs::incoming::{
    IncomingFile, copy_into_incoming, discard_incoming_file, link_into_incoming, move_into_place, open_existing_file,
    write_incoming_bytes,
};
use crate::blobs::quarantine::quarantine_blob;
use crate::error::{StoreError, blob_read_error, blob_write_error};

const HASH_PREFIX: &str = "blake3:";
const HASH_HEX_DIGITS: usize = 64;
// Blobs are spread over 256 folders by their first two hex digits, so no folder grows to millions of names.
const FOLDER_HEX_DIGITS: usize = 2;
const READ_BUFFER_BYTES: usize = 1 << 20;

/// Where the blobs live: `<data>/blobs/` (contracts 8.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStore {
    pub root: PathBuf,
}

/// How `put_file` brings a file in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutMode {
    /// Copy the bytes: the source stays the caller's to change or delete.
    Copy,
    /// Hard-link the file, as when a checkpoint is pinned: no second copy on disk. Falls back to a copy when the
    /// file is on another filesystem. The source must not be changed afterwards, since it is the blob.
    HardLink,
}

/// Stores `bytes` and returns their reference. Storing bytes the store already has changes nothing but the
/// existing blob's modification time.
pub fn put_bytes(store: &BlobStore, bytes: &[u8], media_type: &str) -> Result<BlobRef, StoreError> {
    let incoming = write_incoming_bytes(&store.root, bytes)?;
    finish_put(store, incoming, media_type)
}

/// Stores the file at `source` and returns its reference. The source itself is left where it is.
pub fn put_file(store: &BlobStore, source: &Path, mode: PutMode, media_type: &str) -> Result<BlobRef, StoreError> {
    let incoming = match mode {
        PutMode::Copy => copy_into_incoming(&store.root, source)?,
        PutMode::HardLink => link_or_copy_into_incoming(store, source)?,
    };
    finish_put(store, incoming, media_type)
}

/// Opens a blob for reading. `RRS-S1002` when the store has no blob by that hash.
pub fn open_blob(store: &BlobStore, hash: &str) -> Result<File, StoreError> {
    let path = existing_blob_path(store, hash)?;
    File::open(&path).map_err(|error| missing_or_read_error(hash, &path, &error))
}

/// Whether the store has a blob by that hash. A hash that isn't `blake3:<64 lowercase hex>` names no blob.
pub fn has_blob(store: &BlobStore, hash: &str) -> bool {
    match blob_path(store, hash) {
        Some(path) => path.is_file(),
        None => false,
    }
}

/// Re-hashes a blob. On a mismatch the blob is moved to `.quarantine/` and the error is `RRS-S1001`; from then on
/// the store doesn't have it. `RRS-S1002` when there is no such blob.
pub fn verify_blob(store: &BlobStore, hash: &str) -> Result<(), StoreError> {
    let path = existing_blob_path(store, hash)?;
    let mut file = File::open(&path).map_err(|error| missing_or_read_error(hash, &path, &error))?;
    let (actual_hash, _) = hash_open_file(&mut file, &path)?;
    if actual_hash == hash {
        return Ok(());
    }
    // Close it before the move: Windows refuses to rename an open file.
    drop(file);
    let quarantine_path = quarantine_blob(&store.root, &path)?;
    Err(StoreError::BlobHashMismatch {
        hash: hash.to_string(),
        quarantine_path,
    })
}

/// Where the blob with this hash lives, or `None` when the text isn't `blake3:<64 lowercase hex>`. Checking the form
/// first means a hash can never name a path outside the store.
pub(crate) fn blob_path(store: &BlobStore, hash: &str) -> Option<PathBuf> {
    let hex = hash.strip_prefix(HASH_PREFIX)?;
    let is_lowercase_hex = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if hex.len() != HASH_HEX_DIGITS || !is_lowercase_hex {
        return None;
    }
    Some(store.root.join(&hex[..FOLDER_HEX_DIGITS]).join(hex))
}

/// Puts a fsynced `.incoming/` file in its place, or drops it when the blob is already there.
fn finish_put(store: &BlobStore, incoming: IncomingFile, media_type: &str) -> Result<BlobRef, StoreError> {
    let Some(path) = blob_path(store, &incoming.hash) else {
        discard_incoming_file(&incoming.path);
        return Err(StoreError::BlobWriteFailed {
            path: incoming.path.clone(),
            reason: format!("the hasher returned {}, which isn't a blob name", incoming.hash),
        });
    };
    if path.is_file() {
        discard_incoming_file(&incoming.path);
        touch_blob(&path)?;
    } else {
        move_into_place(&incoming.path, &path)?;
    }
    Ok(BlobRef {
        hash: incoming.hash,
        size_bytes: incoming.size_bytes,
        media_type: media_type.to_string(),
    })
}

/// Hashes the file where it is, then hard-links it into `.incoming/`; copies it when no link can be made.
fn link_or_copy_into_incoming(store: &BlobStore, source: &Path) -> Result<IncomingFile, StoreError> {
    let mut file = File::open(source).map_err(|error| blob_read_error(source, &error))?;
    let (hash, size_bytes) = hash_open_file(&mut file, source)?;
    match link_into_incoming(&store.root, source, &hash, size_bytes)? {
        Some(incoming) => Ok(incoming),
        None => copy_into_incoming(&store.root, source),
    }
}

/// Refreshes an existing blob's modification time, so a sweep running now sees it as just referenced (8.3).
fn touch_blob(path: &Path) -> Result<(), StoreError> {
    let file = open_existing_file(path).map_err(|error| blob_write_error(path, &error))?;
    file.set_modified(SystemTime::now())
        .map_err(|error| blob_write_error(path, &error))
}

fn existing_blob_path(store: &BlobStore, hash: &str) -> Result<PathBuf, StoreError> {
    match blob_path(store, hash) {
        Some(path) => Ok(path),
        None => Err(StoreError::BlobMissing { hash: hash.to_string() }),
    }
}

fn missing_or_read_error(hash: &str, path: &Path, error: &std::io::Error) -> StoreError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return StoreError::BlobMissing { hash: hash.to_string() };
    }
    blob_read_error(path, error)
}

/// The hash and size of everything left to read in `file`, read a piece at a time.
fn hash_open_file(file: &mut File, path: &Path) -> Result<(String, u64), StoreError> {
    let mut hasher = new_stream_hasher();
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    let mut size_bytes = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|error| blob_read_error(path, &error))?;
        if count == 0 {
            break;
        }
        update_stream_hasher(&mut hasher, &buffer[..count]);
        size_bytes += count as u64;
    }
    Ok((finish_stream_hasher(hasher), size_bytes))
}
