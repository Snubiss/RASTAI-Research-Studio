//! `write_config_file`, the one way a config file is replaced (contracts 11.1): `settings.json` (`settings.set`),
//! keybindings and layouts. The files are the only store of what they hold, so a crash or a full disk must never
//! leave one half written: the new bytes go to a temporary file in the same folder, which is fsynced, renamed over
//! the target and the folder fsynced. A reader sees the old file or the new one, never a mix.
//!
//! The store never reads these files; the caller does (rastai-server parses `settings.json` with rastai-formats).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{StoreError, config_file_write_error};

// The temporary file is `.<file name>.<32 hex digits>.tmp` beside the target: hidden, never taken for the real file,
// and a rename within one folder is atomic. 128 random bits: two writers never pick the same name.
const TEMP_NAME_PREFIX: &str = ".";
const TEMP_NAME_SUFFIX: &str = ".tmp";
const TEMP_NAME_RANDOM_BYTES: usize = 16;

/// Replaces `path` with `bytes` atomically, creating its folder when needed. On any error the old file (or its
/// absence) is left as it was, and the temporary file is removed.
pub fn write_config_file(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let folder = parent_folder(path)?;
    std::fs::create_dir_all(&folder).map_err(|error| config_file_write_error(&folder, &error))?;
    let temporary_path = folder.join(temporary_name(path)?);
    if let Err(error) = write_and_sync(&temporary_path, bytes) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary_path, path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(config_file_write_error(path, &error));
    }
    sync_folder(&folder)
}

fn parent_folder(path: &Path) -> Result<PathBuf, StoreError> {
    match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => Ok(folder.to_path_buf()),
        _ => Ok(PathBuf::from(".")),
    }
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    // create_new: never write into a file that is already there, however unlikely the name.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| config_file_write_error(path, &error))?;
    file.write_all(bytes)
        .map_err(|error| config_file_write_error(path, &error))?;
    file.sync_all().map_err(|error| config_file_write_error(path, &error))
}

/// `.settings.json.<32 hex digits>.tmp` for `settings.json`.
fn temporary_name(path: &Path) -> Result<String, StoreError> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let random = match rastai_core::random::random_bytes(TEMP_NAME_RANDOM_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(StoreError::ConfigFileWriteFailed {
                path: path.to_path_buf(),
                reason: format!("no random name for a temporary file: {error:?}"),
            });
        }
    };
    let mut name = format!("{TEMP_NAME_PREFIX}{file_name}.");
    for byte in random {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(TEMP_NAME_SUFFIX);
    Ok(name)
}

#[cfg(unix)]
fn sync_folder(folder: &Path) -> Result<(), StoreError> {
    let handle = File::open(folder).map_err(|error| config_file_write_error(folder, &error))?;
    handle
        .sync_all()
        .map_err(|error| config_file_write_error(folder, &error))
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
        let folder = std::env::temp_dir().join(format!("rastai-store-config-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    fn file_names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_new_file_and_its_folder_are_created() {
        let folder = test_folder("new");
        let path = folder.join("config").join("settings.json");
        write_config_file(&path, b"{\"a\": 1}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\": 1}");
        assert_eq!(file_names(&folder.join("config")), ["settings.json"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn an_existing_file_is_replaced_whole_and_no_temporary_file_stays() {
        let folder = test_folder("replace");
        let path = folder.join("settings.json");
        write_config_file(&path, b"a much longer first version").unwrap();
        write_config_file(&path, b"short").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"short");
        assert_eq!(file_names(&folder), ["settings.json"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_target_that_is_a_folder_is_refused_and_left_alone() {
        let folder = test_folder("folder-target");
        let path = folder.join("settings.json");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        let error = write_config_file(&path, b"{}").unwrap_err();
        assert_eq!(error.code(), "RRS-S3005");
        assert!(path.join("inside").is_dir());
        assert_eq!(file_names(&folder), ["settings.json"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_temporary_name_is_hidden_and_random() {
        let path = Path::new("/config/settings.json");
        let first = temporary_name(path).unwrap();
        let second = temporary_name(path).unwrap();
        assert!(first.starts_with(".settings.json."), "{first}");
        assert!(first.ends_with(".tmp"), "{first}");
        assert_eq!(first.len(), ".settings.json.".len() + 32 + ".tmp".len());
        assert_ne!(first, second);
    }
}
