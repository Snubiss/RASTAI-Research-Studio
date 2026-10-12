//! The blob store (contracts 8.4, flows/platform.md 8.2, layout rust-control-plane.md 6.7): blobs are named by the
//! BLAKE3 of their bytes, the same bytes are stored once and their blob touched, a flipped byte is quarantined
//! (`RRS-S1001`), a missing blob is `RRS-S1002`, and a start clears what a crash left in `.incoming/`.

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rastai_core::paths::blobs_dir;
use rastai_formats::hashing::hash_bytes;
use rastai_store::blobs::blob_store::{BlobStore, PutMode, has_blob, open_blob, put_bytes, put_file, verify_blob};
use rastai_store::error::StoreError;
use rastai_store::store::{close_store, open_store};

mod common;
use common::{remove_test_folder, test_paths};

const MEDIA_TYPE: &str = "application/octet-stream";
const BLOB_BYTES: &[u8] = b"the weights of a very small model";
// Old enough that a touch is plainly newer, whatever the file system's time resolution.
const AN_HOUR: Duration = Duration::from_secs(3600);

fn test_store(test_name: &str) -> BlobStore {
    let paths = test_paths(test_name);
    BlobStore {
        root: blobs_dir(&paths),
    }
}

/// test_paths puts the data folder (the blobs' parent) under one test folder. A test that only read may not have
/// made it, so a missing folder is fine.
fn remove_store(store: &BlobStore) {
    if let Some(root) = store.root.parent().and_then(Path::parent) {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// `<root>/ab/<64 hex>`, as contracts 8.4 lays the store out.
fn expected_blob_path(store: &BlobStore, hash: &str) -> PathBuf {
    let hex = hash.strip_prefix("blake3:").unwrap();
    store.root.join(&hex[..2]).join(hex)
}

fn files_in(folder: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(folder) {
        Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    }
}

fn read_blob(store: &BlobStore, hash: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    open_blob(store, hash).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn bytes_are_stored_under_their_hash() {
    let store = test_store("blobs-put-bytes");

    let blob = put_bytes(&store, BLOB_BYTES, MEDIA_TYPE).unwrap();

    assert_eq!(blob.hash, hash_bytes(BLOB_BYTES));
    assert_eq!(blob.size_bytes, BLOB_BYTES.len() as u64);
    assert_eq!(blob.media_type, MEDIA_TYPE);
    assert_eq!(
        std::fs::read(expected_blob_path(&store, &blob.hash)).unwrap(),
        BLOB_BYTES
    );
    assert!(has_blob(&store, &blob.hash));
    assert_eq!(read_blob(&store, &blob.hash), BLOB_BYTES);
    // The write went through .incoming/ and left nothing there.
    assert!(files_in(&store.root.join(".incoming")).is_empty());
    verify_blob(&store, &blob.hash).unwrap();
    remove_store(&store);
}

#[test]
fn the_same_bytes_twice_are_stored_once_and_touch_the_blob() {
    let store = test_store("blobs-dedupe");
    let first = put_bytes(&store, BLOB_BYTES, MEDIA_TYPE).unwrap();
    let path = expected_blob_path(&store, &first.hash);
    let an_hour_ago = SystemTime::now() - AN_HOUR;
    // Write access, because Windows refuses to set the time through a read-only handle.
    let blob_file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    blob_file.set_modified(an_hour_ago).unwrap();
    drop(blob_file);

    let second = put_bytes(&store, BLOB_BYTES, MEDIA_TYPE).unwrap();

    assert_eq!(second, first);
    assert_eq!(files_in(path.parent().unwrap()), vec![path.clone()]);
    assert!(files_in(&store.root.join(".incoming")).is_empty());
    // The touch keeps the GC's grace window honest when a new reference arrives (flows/platform.md 8.2, 8.3).
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(modified > an_hour_ago + AN_HOUR / 2);
    remove_store(&store);
}

#[test]
fn a_flipped_byte_is_quarantined() {
    let store = test_store("blobs-quarantine");
    let blob = put_bytes(&store, BLOB_BYTES, MEDIA_TYPE).unwrap();
    let path = expected_blob_path(&store, &blob.hash);
    let mut damaged = BLOB_BYTES.to_vec();
    damaged[5] ^= 0x01;
    std::fs::write(&path, &damaged).unwrap();

    let error = verify_blob(&store, &blob.hash).unwrap_err();

    assert_eq!(error.code(), "RRS-S1001");
    let StoreError::BlobHashMismatch { hash, quarantine_path } = error else {
        panic!("expected a hash mismatch");
    };
    assert_eq!(hash, blob.hash);
    assert_eq!(quarantine_path.parent().unwrap(), store.root.join(".quarantine"));
    // Set aside with its damaged bytes, and gone from the store.
    assert_eq!(std::fs::read(&quarantine_path).unwrap(), damaged);
    assert!(!path.exists());
    assert!(!has_blob(&store, &blob.hash));
    assert_eq!(open_blob(&store, &blob.hash).unwrap_err().code(), "RRS-S1002");
    assert_eq!(verify_blob(&store, &blob.hash).unwrap_err().code(), "RRS-S1002");
    // Storing the good bytes again brings the blob back.
    put_bytes(&store, BLOB_BYTES, MEDIA_TYPE).unwrap();
    verify_blob(&store, &blob.hash).unwrap();
    remove_store(&store);
}

#[test]
fn a_missing_blob_or_a_malformed_hash_is_missing() {
    let store = test_store("blobs-missing");
    let absent = hash_bytes(b"never stored");
    // Not a blob name: an uppercase digest and a path that tries to leave the store.
    let uppercase = absent.to_uppercase().replace("BLAKE3:", "blake3:");
    let escaping = format!("blake3:../../{}", &absent["blake3:".len() + 6..]);

    for hash in [absent.as_str(), uppercase.as_str(), escaping.as_str(), "", "sha256:00"] {
        assert!(!has_blob(&store, hash));
        assert_eq!(open_blob(&store, hash).unwrap_err().code(), "RRS-S1002", "{hash}");
        assert_eq!(verify_blob(&store, hash).unwrap_err().code(), "RRS-S1002", "{hash}");
    }
    remove_store(&store);
}

#[test]
fn a_file_is_copied_or_linked_and_keeps_its_source() {
    let store = test_store("blobs-put-file");
    std::fs::create_dir_all(&store.root).unwrap();
    let source = store.root.parent().unwrap().join("weights.safetensors");
    std::fs::write(&source, BLOB_BYTES).unwrap();

    let copied = put_file(&store, &source, PutMode::Copy, MEDIA_TYPE).unwrap();
    assert_eq!(copied.hash, hash_bytes(BLOB_BYTES));
    assert_eq!(read_blob(&store, &copied.hash), BLOB_BYTES);
    assert!(source.exists());

    // The same bytes by hard link: the blob is already there, so the link is dropped and nothing changes.
    let linked = put_file(&store, &source, PutMode::HardLink, MEDIA_TYPE).unwrap();
    assert_eq!(linked, copied);
    assert!(files_in(&store.root.join(".incoming")).is_empty());

    // New bytes by hard link become a blob that is the source file itself.
    let other_source = store.root.parent().unwrap().join("adapter.safetensors");
    std::fs::write(&other_source, b"adapter bytes").unwrap();
    let other = put_file(&store, &other_source, PutMode::HardLink, MEDIA_TYPE).unwrap();
    assert_eq!(other.hash, hash_bytes(b"adapter bytes"));
    assert!(other_source.exists());
    assert_eq!(read_blob(&store, &other.hash), b"adapter bytes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let blob_metadata = std::fs::metadata(expected_blob_path(&store, &other.hash)).unwrap();
        assert_eq!(blob_metadata.ino(), std::fs::metadata(&other_source).unwrap().ino());
    }
    remove_store(&store);
}

/// A symbolic link is followed: the blob is the file it names, never the link itself, so it stays inside the store
/// and outlives the link's target. Both an absolute link and a relative one (the Hugging Face cache's layout).
#[cfg(unix)]
#[test]
fn a_symlinked_source_is_stored_as_the_file_it_names() {
    use std::os::unix::fs::{MetadataExt, symlink};

    let store = test_store("blobs-symlink-source");
    let outside = store.root.parent().unwrap().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let absolute_target = outside.join("absolute-target.bin");
    let relative_target = outside.join("relative-target.bin");
    std::fs::write(&absolute_target, b"absolute target bytes").unwrap();
    std::fs::write(&relative_target, b"relative target bytes").unwrap();
    let absolute_link = outside.join("absolute-link.bin");
    let relative_link = outside.join("relative-link.bin");
    symlink(&absolute_target, &absolute_link).unwrap();
    symlink("relative-target.bin", &relative_link).unwrap();

    for (link, target) in [(&absolute_link, &absolute_target), (&relative_link, &relative_target)] {
        let bytes = std::fs::read(target).unwrap();
        let blob = put_file(&store, link, PutMode::HardLink, MEDIA_TYPE).unwrap();
        let path = expected_blob_path(&store, &blob.hash);

        assert_eq!(blob.hash, hash_bytes(&bytes));
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_file());
        assert_eq!(
            std::fs::metadata(&path).unwrap().ino(),
            std::fs::metadata(target).unwrap().ino()
        );
        // Copy follows the same link to the same blob.
        assert_eq!(put_file(&store, link, PutMode::Copy, MEDIA_TYPE).unwrap(), blob);
        assert!(files_in(&store.root.join(".incoming")).is_empty());

        // Deleting the target (a run folder removed) leaves the blob whole.
        std::fs::remove_file(target).unwrap();
        assert!(has_blob(&store, &blob.hash));
        assert_eq!(read_blob(&store, &blob.hash), bytes);
        verify_blob(&store, &blob.hash).unwrap();
    }
    remove_store(&store);
}

#[test]
fn a_missing_source_file_is_a_failed_read() {
    let store = test_store("blobs-missing-source");
    let source = store.root.join("not-here.bin");

    for mode in [PutMode::Copy, PutMode::HardLink] {
        let error = put_file(&store, &source, mode, MEDIA_TYPE).unwrap_err();
        assert_eq!(error.code(), "RRS-S0006");
    }
    remove_test_folder(&test_paths("blobs-missing-source"));
}

#[test]
fn opening_the_store_clears_what_a_crash_left_in_incoming() {
    let paths = test_paths("blobs-startup-cleanup");
    let incoming = blobs_dir(&paths).join(".incoming");
    std::fs::create_dir_all(&incoming).unwrap();
    std::fs::write(incoming.join("0123456789abcdef0123456789abcdef"), b"half a blob").unwrap();
    // A transfer's resumable partial file survives a restart (contracts 8.4).
    let partial = incoming.join(format!("{}.partial", hash_bytes(b"x").replace(':', "-")));
    std::fs::write(&partial, b"half a transfer").unwrap();

    let store = open_store(&paths).unwrap();

    assert_eq!(store.blobs.root, blobs_dir(&paths));
    assert_eq!(files_in(&incoming), vec![partial]);
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(close_store(store));
    remove_test_folder(&paths);
}
