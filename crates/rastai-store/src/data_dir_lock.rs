//! The daemon lock: `<runtime>/daemon.lock`, held with an exclusive OS file lock for the daemon's whole life, so
//! two daemons never share one data directory (`RRS-S0001`). The offline `rastai storage rebuild-index` takes it too.
//!
//! The OS drops the lock when the process ends, however it ends, so a crashed daemon never leaves a stale one. The
//! file itself stays; deleting it on exit would let a second daemon lock a new file while a first still held the
//! old one.
//!
//! Without `XDG_RUNTIME_DIR` the runtime folder is `/tmp/rastai-<uid>`, in a folder every user can write to, so
//! another user could create it first, or put a symbolic link at `daemon.lock` to make the daemon write over a file
//! of its choosing. An existing runtime folder is used only if it is a real folder (not a link), owned by this user
//! and closed to group and others (0700), and `daemon.lock` only if it is a regular file, never a link
//! (`RRS-S0007`). Once the folder is known to be private, no other user can add or swap a file in it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use rastai_core::paths::{DataPaths, daemon_lock_path};

use crate::error::StoreError;

// The runtime folder holds the sockets too, so only the owner may enter it (contracts 8.1).
#[cfg(unix)]
const PRIVATE_FOLDER_MODE: u32 = 0o700;
// The permission bits for group and others: a private folder has none of them.
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;
// A file this process creates is owned by its user. std has no getuid and libc is disk_space.rs's alone
// (ADR-0033), so the user id is read off a file made for that, and removed straight after.
#[cfg(unix)]
const OWNER_PROBE_PREFIX: &str = ".owner-probe-";

/// The held lock. Dropping it (or ending the process) releases it.
pub struct DaemonLock {
    // Never read: the open, locked file is the lock.
    _file: File,
    pub path: PathBuf,
}

/// Takes the daemon lock, or fails with `RRS-S0001` naming the process that holds it. The holder's process id is
/// written into the file, so the refusal can say who it is. A runtime folder or `daemon.lock` that another user
/// could reach is refused with `RRS-S0007`, before anything is written.
pub fn take_daemon_lock(paths: &DataPaths) -> Result<DaemonLock, StoreError> {
    let path = daemon_lock_path(paths);
    prepare_private_folder(&path)?;
    refuse_unless_regular_file_or_missing(&path)?;
    // Never truncated on open: the file is only written once the lock is held, by write_holder_pid.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| not_writable(&path, &error))?;
    check_opened_file_is_at_path(&file, &path)?;
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

/// Refuses `daemon.lock` when it is a symbolic link or anything but a regular file. Opening a link would lock,
/// and then overwrite, whatever file it points to.
fn refuse_unless_regular_file_or_missing(lock_path: &Path) -> Result<(), StoreError> {
    match std::fs::symlink_metadata(lock_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(not_private(lock_path, "it is a symbolic link")),
        Ok(metadata) if !metadata.is_file() => Err(not_private(lock_path, "it is not a regular file")),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(not_writable(lock_path, &error)),
    }
}

/// Checks that the file just opened is the regular file at `lock_path`, not one a link put there between the check
/// and the open: same device and inode as the path itself, read without following links.
#[cfg(unix)]
fn check_opened_file_is_at_path(file: &File, lock_path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::MetadataExt;

    let opened = file.metadata().map_err(|error| not_writable(lock_path, &error))?;
    let at_path = std::fs::symlink_metadata(lock_path).map_err(|error| not_writable(lock_path, &error))?;
    let same_file = opened.dev() == at_path.dev() && opened.ino() == at_path.ino();
    if !at_path.is_file() || !same_file {
        return Err(not_private(lock_path, "it changed while it was being opened"));
    }
    Ok(())
}

/// Windows has no shared temporary runtime folder (contracts 8.1); the link check before the open is enough.
#[cfg(not(unix))]
fn check_opened_file_is_at_path(_file: &File, _lock_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

/// Creates the runtime folder 0700 if it is missing, then refuses it unless it is private to this user, whether
/// this process made it or found it there.
#[cfg(unix)]
fn prepare_private_folder(lock_path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::DirBuilderExt;

    let Some(folder) = lock_path.parent() else {
        return Ok(());
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_FOLDER_MODE)
        .create(folder)
        .map_err(|error| not_writable(folder, &error))?;
    // The shape first, so nothing is written into a folder that group or others can open.
    check_folder_shape(folder)?;
    let user_id = read_own_user_id(folder)?;
    check_folder_owner(folder, user_id)
}

#[cfg(not(unix))]
fn prepare_private_folder(lock_path: &Path) -> Result<(), StoreError> {
    let Some(folder) = lock_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(folder).map_err(|error| not_writable(folder, &error))
}

/// A real folder, not a link to one, with no permission bits for group or others.
#[cfg(unix)]
fn check_folder_shape(folder: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::symlink_metadata(folder).map_err(|error| not_writable(folder, &error))?;
    if metadata.file_type().is_symlink() {
        return Err(not_private(folder, "it is a symbolic link"));
    }
    if !metadata.is_dir() {
        return Err(not_private(folder, "it is not a folder"));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode & GROUP_AND_OTHER_BITS != 0 {
        let reason = format!("its mode is {mode:o}, not {PRIVATE_FOLDER_MODE:o}");
        return Err(not_private(folder, &reason));
    }
    Ok(())
}

/// Refuses a folder that another user owns: its owner could add or swap files in it.
#[cfg(unix)]
fn check_folder_owner(folder: &Path, user_id: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(folder).map_err(|error| not_writable(folder, &error))?;
    if metadata.uid() != user_id {
        let reason = format!("it belongs to user {}, not to user {user_id}", metadata.uid());
        return Err(not_private(folder, &reason));
    }
    Ok(())
}

/// This process's user id: the owner of a new file it creates in `folder`. `create_new` never follows a link and
/// fails if the name exists, so the probe can't be redirected. A probe left by a crash is removed first.
#[cfg(unix)]
fn read_own_user_id(folder: &Path) -> Result<u32, StoreError> {
    use std::os::unix::fs::MetadataExt;

    let probe_path = folder.join(format!("{OWNER_PROBE_PREFIX}{}", std::process::id()));
    let _ = std::fs::remove_file(&probe_path);
    let probe = File::create_new(&probe_path).map_err(|error| not_writable(folder, &error))?;
    let metadata = probe.metadata();
    drop(probe);
    let _ = std::fs::remove_file(&probe_path);
    let metadata = metadata.map_err(|error| not_writable(folder, &error))?;
    Ok(metadata.uid())
}

fn not_private(path: &Path, reason: &str) -> StoreError {
    StoreError::RuntimeNotPrivate {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
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

    /// Every folder of `test_paths` shares one parent, the test's own folder.
    fn remove_test_folder(paths: &DataPaths) {
        if let Some(root) = paths.runtime.parent() {
            std::fs::remove_dir_all(root).unwrap();
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
        let next = take_daemon_lock(&paths);
        assert!(next.is_ok());
        drop(next);
        remove_test_folder(&paths);
    }

    #[cfg(unix)]
    #[test]
    fn the_runtime_folder_is_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let paths = test_paths("private");
        let lock = take_daemon_lock(&paths).unwrap();
        let mode = std::fs::metadata(&paths.runtime).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, PRIVATE_FOLDER_MODE);
        drop(lock);
        remove_test_folder(&paths);
    }

    /// Creates the runtime folder the way another process might have left it, with `mode`.
    #[cfg(unix)]
    fn create_runtime_folder(paths: &DataPaths, mode: u32) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(&paths.runtime).unwrap();
        std::fs::set_permissions(&paths.runtime, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_daemon_lock_is_refused_and_its_target_left_untouched() {
        let paths = test_paths("symlinked-lock");
        create_runtime_folder(&paths, PRIVATE_FOLDER_MODE);
        // The file a planted link points at: the daemon must neither lock nor overwrite it.
        let target = paths.state.join("precious.txt");
        std::fs::create_dir_all(&paths.state).unwrap();
        std::fs::write(&target, "keep me").unwrap();
        std::os::unix::fs::symlink(&target, daemon_lock_path(&paths)).unwrap();

        let error = take_daemon_lock(&paths).err().unwrap();

        assert_eq!(error.code(), "RRS-S0007");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
        remove_test_folder(&paths);
    }

    #[cfg(unix)]
    #[test]
    fn a_runtime_folder_open_to_others_is_refused() {
        let paths = test_paths("open-folder");
        create_runtime_folder(&paths, 0o777);

        let error = take_daemon_lock(&paths).err().unwrap();

        assert_eq!(error.code(), "RRS-S0007");
        // Refused before anything was written into it.
        assert!(!daemon_lock_path(&paths).exists());
        remove_test_folder(&paths);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_runtime_folder_is_refused() {
        let paths = test_paths("symlinked-folder");
        let real_folder = paths.state.join("elsewhere");
        std::fs::create_dir_all(&paths.state).unwrap();
        std::fs::create_dir(&real_folder).unwrap();
        std::os::unix::fs::symlink(&real_folder, &paths.runtime).unwrap();

        let error = take_daemon_lock(&paths).err().unwrap();

        assert_eq!(error.code(), "RRS-S0007");
        assert!(!real_folder.join("daemon.lock").exists());
        remove_test_folder(&paths);
    }

    #[cfg(unix)]
    #[test]
    fn a_runtime_folder_of_another_user_is_refused() {
        let paths = test_paths("foreign-owner");
        create_runtime_folder(&paths, PRIVATE_FOLDER_MODE);
        let own_user_id = read_own_user_id(&paths.runtime).unwrap();
        // A test can't make a folder another user owns without root, so it asks as if this process were another
        // user: the same check then sees a folder that isn't theirs.
        let other_user_id = own_user_id.wrapping_add(1);

        let error = check_folder_owner(&paths.runtime, other_user_id).err().unwrap();

        assert_eq!(error.code(), "RRS-S0007");
        assert!(check_folder_owner(&paths.runtime, own_user_id).is_ok());
        // The owner probe leaves nothing behind.
        assert_eq!(std::fs::read_dir(&paths.runtime).unwrap().count(), 0);
        remove_test_folder(&paths);
    }
}
