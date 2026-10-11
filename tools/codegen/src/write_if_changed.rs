//! Writing generated files. A file is written only when its bytes would change, so an unchanged output keeps its
//! timestamp and cargo, vite and pytest don't rebuild or re-collect what didn't change.

use std::fs;
use std::path::Path;

use crate::error::{CodegenError, codegen_error};

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
}
