//! How bytes enter the blob store (contracts 8.4, flows/platform.md 8.2): written to `.incoming/<random>`, fsynced,
//! then renamed to `ab/<hash>` and the folder fsynced. A blob is visible only after the rename, so a reader never
//! sees half of one, and a crash leaves at most an unreferenced file in `.incoming/`, which the next start deletes.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rastai_formats::hashing::{finish_stream_hasher, hash_bytes, new_stream_hasher, update_stream_hasher};

use crate::error::{StoreError, blob_read_error, blob_write_error};

const INCOMING_FOLDER: &str = ".incoming";
// Transfers keep resumable partial files in `.incoming/` too (contracts 8.4), so a restart must leave them.
const PARTIAL_SUFFIX: &str = ".partial";
// 128 random bits: two writers never pick the same temporary name.
const TEMP_NAME_RANDOM_BYTES: usize = 16;
// Files are copied and hashed a piece at a time, so a large checkpoint never sits in memory whole.
const COPY_BUFFER_BYTES: usize = 1 << 20;

/// A file in `.incoming/`, fsynced, and the hash and size of its bytes.
pub(crate) struct IncomingFile {
    pub(crate) path: PathBuf,
    pub(crate) hash: String,
    pub(crate) size_bytes: u64,
}

/// Writes `bytes` to a new file in `.incoming/` and fsyncs it.
pub(crate) fn write_incoming_bytes(store_root: &Path, bytes: &[u8]) -> Result<IncomingFile, StoreError> {
    let (path, mut file) = create_incoming_file(store_root)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if let Err(error) = written {
        discard_incoming_file(&path);
        return Err(blob_write_error(&path, &error));
    }
    Ok(IncomingFile {
        path,
        hash: hash_bytes(bytes),
        size_bytes: bytes.len() as u64,
    })
}

/// Copies the file at `source` into a new file in `.incoming/`, hashing it on the way, and fsyncs the copy.
pub(crate) fn copy_into_incoming(store_root: &Path, source: &Path) -> Result<IncomingFile, StoreError> {
    let mut reader = File::open(source).map_err(|error| blob_read_error(source, &error))?;
    let (path, mut file) = create_incoming_file(store_root)?;
    match copy_and_hash(&mut reader, source, &mut file, &path) {
        Ok((hash, size_bytes)) => Ok(IncomingFile { path, hash, size_bytes }),
        Err(error) => {
            discard_incoming_file(&path);
            Err(error)
        }
    }
}

/// Hard-links the file at `source` into `.incoming/`, for a file whose hash the caller already has. `None` when the
/// two are on different filesystems, where no hard link can exist: the caller copies instead.
pub(crate) fn link_into_incoming(
    store_root: &Path,
    source: &Path,
    hash: &str,
    size_bytes: u64,
) -> Result<Option<IncomingFile>, StoreError> {
    let folder = incoming_folder(store_root)?;
    let path = folder.join(random_file_name(&folder)?);
    match std::fs::hard_link(source, &path) {
        Ok(()) => Ok(Some(IncomingFile {
            path,
            hash: hash.to_string(),
            size_bytes,
        })),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => Ok(None),
        Err(error) => Err(blob_write_error(&path, &error)),
    }
}

/// Renames a fsynced `.incoming/` file to its place in the store, creating its `ab/` folder when needed, and fsyncs
/// the folder so the new name survives a power cut.
pub(crate) fn move_into_place(incoming_path: &Path, blob_path: &Path) -> Result<(), StoreError> {
    let Some(folder) = blob_path.parent() else {
        return Err(blob_write_error(
            blob_path,
            &std::io::Error::from(std::io::ErrorKind::NotFound),
        ));
    };
    let new_folder = !folder.is_dir();
    std::fs::create_dir_all(folder).map_err(|error| blob_write_error(folder, &error))?;
    if new_folder && let Some(store_root) = folder.parent() {
        // The new `ab/` folder is itself a name in the store's root, which must survive the power cut too.
        sync_folder(store_root)?;
    }
    if let Err(error) = std::fs::rename(incoming_path, blob_path) {
        discard_incoming_file(incoming_path);
        return Err(blob_write_error(blob_path, &error));
    }
    sync_folder(folder)
}

/// Deletes the temporary files a crash or a failed write left in `.incoming/`, keeping the transfers' `.partial`
/// files. Called once at startup, before anything writes a blob. A file that can't be deleted is left for the next
/// start: nothing refers to it, so it does no harm meanwhile.
pub(crate) fn remove_stale_incoming_files(store_root: &Path) {
    let Ok(entries) = std::fs::read_dir(store_root.join(INCOMING_FOLDER)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(PARTIAL_SUFFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Removes an `.incoming/` file that won't become a blob. Failing to is harmless: the next start deletes it.
pub(crate) fn discard_incoming_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn create_incoming_file(store_root: &Path) -> Result<(PathBuf, File), StoreError> {
    let folder = incoming_folder(store_root)?;
    let path = folder.join(random_file_name(&folder)?);
    // create_new: never write into a file that is already there, however unlikely the name.
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| blob_write_error(&path, &error))?;
    Ok((path, file))
}

fn incoming_folder(store_root: &Path) -> Result<PathBuf, StoreError> {
    let folder = store_root.join(INCOMING_FOLDER);
    std::fs::create_dir_all(&folder).map_err(|error| blob_write_error(&folder, &error))?;
    Ok(folder)
}

/// 32 hex digits from the operating system's random source.
fn random_file_name(folder: &Path) -> Result<String, StoreError> {
    let bytes = match rastai_core::random::random_bytes(TEMP_NAME_RANDOM_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(StoreError::BlobWriteFailed {
                path: folder.to_path_buf(),
                reason: format!("no random name for a temporary file: {error:?}"),
            });
        }
    };
    let mut name = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        name.push_str(&format!("{byte:02x}"));
    }
    Ok(name)
}

/// Copies everything from `reader` to `file`, hashing it, then fsyncs `file`. Returns the hash and the size.
fn copy_and_hash(reader: &mut File, source: &Path, file: &mut File, path: &Path) -> Result<(String, u64), StoreError> {
    let mut hasher = new_stream_hasher();
    let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
    let mut size_bytes = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| blob_read_error(source, &error))?;
        if count == 0 {
            break;
        }
        update_stream_hasher(&mut hasher, &buffer[..count]);
        file.write_all(&buffer[..count])
            .map_err(|error| blob_write_error(path, &error))?;
        size_bytes += count as u64;
    }
    file.sync_all().map_err(|error| blob_write_error(path, &error))?;
    Ok((finish_stream_hasher(hasher), size_bytes))
}

#[cfg(unix)]
fn sync_folder(folder: &Path) -> Result<(), StoreError> {
    let handle = File::open(folder).map_err(|error| blob_write_error(folder, &error))?;
    handle.sync_all().map_err(|error| blob_write_error(folder, &error))
}

/// Windows can't open a folder to fsync it; NTFS makes a rename durable by its own journal.
#[cfg(not(unix))]
fn sync_folder(_folder: &Path) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn test_folder(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-store-incoming-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn incoming_names(store_root: &Path) -> Vec<String> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(store_root.join(INCOMING_FOLDER)).unwrap() {
            names.push(entry.unwrap().file_name().to_string_lossy().to_string());
        }
        names.sort();
        names
    }

    #[test]
    fn bytes_land_in_incoming_then_move_into_place() {
        let root = test_folder("write-then-move");
        let incoming = write_incoming_bytes(&root, b"blob bytes").unwrap();
        assert_eq!(incoming.hash, hash_bytes(b"blob bytes"));
        assert_eq!(incoming.size_bytes, 10);
        assert_eq!(std::fs::read(&incoming.path).unwrap(), b"blob bytes");
        assert_eq!(incoming_names(&root).len(), 1);

        let blob_path = root.join("ab").join("abcdef");
        move_into_place(&incoming.path, &blob_path).unwrap();

        assert_eq!(std::fs::read(&blob_path).unwrap(), b"blob bytes");
        assert!(incoming_names(&root).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_copied_file_has_the_same_hash_as_its_bytes() {
        let root = test_folder("copy");
        let source = root.join("source.bin");
        // Larger than the copy buffer, so the hash is built from several pieces.
        let bytes: Vec<u8> = (0..COPY_BUFFER_BYTES + 7).map(|index| (index % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();

        let incoming = copy_into_incoming(&root, &source).unwrap();

        assert_eq!(incoming.hash, hash_bytes(&bytes));
        assert_eq!(incoming.size_bytes, bytes.len() as u64);
        assert_eq!(std::fs::read(&incoming.path).unwrap(), bytes);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn startup_cleanup_keeps_only_the_partial_transfers() {
        let root = test_folder("cleanup");
        write_incoming_bytes(&root, b"left by a crash").unwrap();
        std::fs::write(
            root.join(INCOMING_FOLDER).join("blake3-0011.partial"),
            b"half a transfer",
        )
        .unwrap();

        remove_stale_incoming_files(&root);

        assert_eq!(incoming_names(&root), vec!["blake3-0011.partial".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
