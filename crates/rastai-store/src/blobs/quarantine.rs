//! Setting a damaged blob aside (contracts 8.4, flows/platform.md 8.2). A blob whose bytes don't hash to its name is
//! moved to `<data>/blobs/.quarantine/`, out of the store's `ab/<hash>` layout, so it is never served again and
//! `has_blob` answers false. The bytes are kept, not deleted, so the damage can still be looked at.

use std::path::{Path, PathBuf};

use crate::error::{StoreError, blob_write_error};

const QUARANTINE_FOLDER: &str = ".quarantine";

/// Moves the blob file at `blob_path` to `.quarantine/<file name>-<unix_ms>` under `store_root` and returns where it
/// went. The time in the name keeps a second damaged copy of the same blob from replacing the first.
pub(crate) fn quarantine_blob(store_root: &Path, blob_path: &Path) -> Result<PathBuf, StoreError> {
    let folder = store_root.join(QUARANTINE_FOLDER);
    std::fs::create_dir_all(&folder).map_err(|error| blob_write_error(&folder, &error))?;
    let file_name = match blob_path.file_name() {
        Some(name) => name.to_string_lossy().to_string(),
        None => String::new(),
    };
    let target = folder.join(format!("{file_name}-{}", rastai_core::time::now_unix_ms()));
    std::fs::rename(blob_path, &target).map_err(|error| blob_write_error(blob_path, &error))?;
    Ok(target)
}
