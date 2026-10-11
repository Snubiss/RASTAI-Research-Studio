//! The daemon lock: `<runtime>/daemon.lock`, held with an exclusive OS file lock for the daemon's whole life, so
//! two daemons never share one data directory (`RRS-S0001`). The offline `rastai storage rebuild-index` takes it too.
//!
//! The OS drops the lock when the process ends, however it ends, so a crashed daemon never leaves a stale one. The
//! file itself stays; deleting it on exit would let a second daemon lock a new file while a first still held the
//! old one.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use rastai_core::paths::{DataPaths, daemon_lock_path};

use crate::error::StoreError;

// The runtime folder holds the sockets too, so only the owner may enter it (contracts 8.1).
#[cfg(unix)]
const PRIVATE_FOLDER_MODE: u32 = 0o700;

/// The held lock. Dropping it (or ending the process) releases it.
pub struct DaemonLock {
    // Never read: the open, locked file is the lock.
    _file: File,
    pub path: PathBuf,
}

/// Takes the daemon lock, or fails with `RRS-S0001` naming the process that holds it. The holder's process id is
/// written into the file, so the refusal can say who it is.
pub fn take_daemon_lock(paths: &DataPaths) -> Result<DaemonLock, StoreError> {
    let path = daemon_lock_path(paths);
    create_private_folder(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| not_writable(&path, &error))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(StoreError::DaemonLockHeld {
                holder_pid: read_holder_pid(&path),
                path,
            });
        }
        Err(TryLockError::Error(error)) => return Err(not_writable(&path, &error)),
    }
    write_holder_pid(&file).map_err(|error| not_writable(&path, &error))?;
    Ok(DaemonLock { _file: file, path })
}

/// Replaces the file's contents with this process's id. Only the lock holder writes, so no two writers interleave.
fn write_holder_pid(file: &File) -> std::io::Result<()> {
    file.set_len(0)?;
    let mut writer = file;
    writer.write_all(std::process::id().to_string().as_bytes())?;
    writer.flush()
}

/// The holder's process id, when the file can be read: Windows keeps a locked file closed to other processes.
fn read_holder_pid(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    text.trim().parse::<u32>().ok()
}

#[cfg(unix)]
fn create_private_folder(lock_path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::DirBuilderExt;

    let Some(folder) = lock_path.parent() else {
        return Ok(());
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_FOLDER_MODE)
        .create(folder)
        .map_err(|error| not_writable(folder, &error))
}

#[cfg(not(unix))]
fn create_private_folder(lock_path: &Path) -> Result<(), StoreError> {
    let Some(folder) = lock_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(folder).map_err(|error| not_writable(folder, &error))
}

fn not_writable(path: &Path, error: &std::io::Error) -> StoreError {
    StoreError::DataDirectoryNotWritable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn test_paths(test_name: &str) -> DataPaths {
        let root = std::env::temp_dir().join(format!("rastai-store-lock-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        }
    }

    #[test]
    fn a_second_daemon_lock_is_refused_while_the_first_is_held() {
        let paths = test_paths("second");
        let first = take_daemon_lock(&paths).unwrap();
        // A second open of the file is a second lock holder, here as in another process.
        let error = take_daemon_lock(&paths).err().unwrap();
        assert_eq!(error.code(), "RRS-S0001");
        if let StoreError::DaemonLockHeld { path, holder_pid } = error {
            assert_eq!(path, first.path);
            // Windows keeps the locked file closed to readers, so only Unix can name the holder.
            if cfg!(unix) {
                assert_eq!(holder_pid, Some(std::process::id()));
            }
        }
        drop(first);
        // Released with the handle, so the next daemon gets it.
        assert!(take_daemon_lock(&paths).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn the_runtime_folder_is_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let paths = test_paths("private");
        let _lock = take_daemon_lock(&paths).unwrap();
        let mode = std::fs::metadata(&paths.runtime).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, PRIVATE_FOLDER_MODE);
    }
}
