//! Writing generated files. A file is written only when its bytes would change, so an unchanged output keeps its
//! timestamp and cargo, vite and pytest don't rebuild or re-collect what didn't change. In a folder one generator
//! owns whole, the files it no longer writes are deleted, so a renamed type doesn't leave its old file behind.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CodegenError, codegen_error};

/// Folders no generator writes and none deletes from: importing the generated Python writes `__pycache__/` beside
/// it (check-generated skips the same folder).
const SKIPPED_FOLDER_NAMES: &[&str] = &["__pycache__"];

/// Writes `contents` to `path` unless the file already holds exactly those bytes, creating the parent folders it
/// needs. Returns true when it wrote the file.
pub fn write_if_changed(path: &Path, contents: &str) -> Result<bool, CodegenError> {
    if holds_exactly(path, contents) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| codegen_error(parent, format!("can't create the folder: {error}")))?;
    }
    fs::write(path, contents).map_err(|error| codegen_error(path, format!("can't write the file: {error}")))?;
    Ok(true)
}

/// True when the file exists and holds exactly these bytes. A file that can't be read counts as different, so the
/// write that follows reports the real problem.
fn holds_exactly(path: &Path, contents: &str) -> bool {
    match fs::read(path) {
        Ok(existing) => existing == contents.as_bytes(),
        Err(_) => false,
    }
}

/// Deletes every file below `folder` that isn't one of `written_paths`, and each subfolder that leaves empty. Call
/// it only on a folder the generator owns whole (contracts 15.8), after writing everything it writes there; the
/// paths must be built from the same `folder`. Returns how many files it deleted.
pub fn remove_unwritten_files(folder: &Path, written_paths: &[PathBuf]) -> Result<usize, CodegenError> {
    if !folder.is_dir() {
        return Ok(0);
    }
    let mut removed_count = 0;
    for path in folder_entries(folder)? {
        if path.is_dir() {
            if is_skipped_folder(&path) {
                continue;
            }
            removed_count += remove_unwritten_files(&path, written_paths)?;
            remove_folder_if_empty(&path)?;
        } else if !written_paths.contains(&path) {
            fs::remove_file(&path).map_err(|error| codegen_error(&path, format!("can't delete the file: {error}")))?;
            removed_count += 1;
        }
    }
    Ok(removed_count)
}

fn folder_entries(folder: &Path) -> Result<Vec<PathBuf>, CodegenError> {
    let entries =
        fs::read_dir(folder).map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?;
        paths.push(entry.path());
    }
    Ok(paths)
}

fn is_skipped_folder(path: &Path) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => SKIPPED_FOLDER_NAMES.contains(&name),
        None => false,
    }
}

fn remove_folder_if_empty(folder: &Path) -> Result<(), CodegenError> {
    let is_empty = folder_entries(folder)?.is_empty();
    if is_empty {
        fs::remove_dir(folder).map_err(|error| codegen_error(folder, format!("can't delete the folder: {error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_folder(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-codegen-{}-{test_name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn writes_a_new_file_and_its_folders() {
        let folder = scratch_folder("new-file");
        let path = folder.join("nested/output.txt");
        assert_eq!(write_if_changed(&path, "first\n"), Ok(true));
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn leaves_an_identical_file_alone_and_rewrites_a_different_one() {
        let folder = scratch_folder("same-file");
        let path = folder.join("output.txt");
        assert_eq!(write_if_changed(&path, "same\n"), Ok(true));
        let first_modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(write_if_changed(&path, "same\n"), Ok(false));
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), first_modified);
        assert_eq!(write_if_changed(&path, "changed\n"), Ok(true));
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed\n");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_path_that_cannot_be_written_is_an_error_naming_it() {
        let folder = scratch_folder("blocked");
        // A file where the parent folder should be.
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("blocker"), "a file, not a folder").unwrap();
        let error = write_if_changed(&folder.join("blocker/output.txt"), "text").unwrap_err();
        assert_eq!(error.path, folder.join("blocker"));
        assert!(error.problem.starts_with("can't create the folder"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn files_no_longer_written_are_deleted_and_the_rest_kept() {
        let folder = scratch_folder("prune");
        let kept = folder.join("kept.py");
        let nested_kept = folder.join("commands/rastai.command.kept/1.json");
        let stale = folder.join("renamed.py");
        let nested_stale = folder.join("commands/rastai.command.gone/1.json");
        let cache = folder.join("__pycache__/kept.cpython-312.pyc");
        for path in [&kept, &nested_kept, &stale, &nested_stale, &cache] {
            write_if_changed(path, "text").unwrap();
        }
        let written = vec![kept.clone(), nested_kept.clone()];
        assert_eq!(remove_unwritten_files(&folder, &written), Ok(2));
        assert!(kept.is_file() && nested_kept.is_file() && cache.is_file());
        assert!(!stale.exists());
        assert!(!folder.join("commands/rastai.command.gone").exists());
        assert_eq!(remove_unwritten_files(&folder, &written), Ok(0));
        assert_eq!(remove_unwritten_files(&folder.join("missing"), &written), Ok(0));
        fs::remove_dir_all(&folder).unwrap();
    }
}
