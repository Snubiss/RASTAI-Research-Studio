//! The Hugging Face hub cache's layout (contracts 8.1; layout rust-node-gateway-proto.md 7.9), written the way
//! huggingface_hub writes it, so every HF tool reads what the node downloads. A repository's folder
//! `models--<org>--<name>/` holds `blobs/<name>` (a file's content, named by its hash), `snapshots/<commit>/<file>`
//! (relative symlinks into `blobs/`) and `refs/<revision>` (the commit a branch or tag names). Weights are never
//! copied (contracts section 19). `.locks/<repo folder>/<name>.lock` in the same cache is huggingface_hub's per-file
//! lock, which downloads.rs takes before it writes a blob.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The folder-name prefix huggingface_hub gives a model repository.
const MODEL_FOLDER_PREFIX: &str = "models--";
/// Where huggingface_hub puts the "--" in place of a repository id's "/".
const REPOSITORY_SEPARATOR: &str = "--";
const BLOBS_FOLDER: &str = "blobs";
const SNAPSHOTS_FOLDER: &str = "snapshots";
const REFS_FOLDER: &str = "refs";
const LOCKS_FOLDER: &str = ".locks";
/// A blob being downloaded has this suffix until its hash is checked and it is renamed to its name.
const INCOMPLETE_SUFFIX: &str = ".incomplete";
const LOCK_SUFFIX: &str = ".lock";
/// Snapshot links climb from `snapshots/<commit>/` to the repository folder (two levels) and then down into `blobs/`.
const SNAPSHOT_TO_REPOSITORY_LEVELS: usize = 2;

/// `models--<org>--<name>` for a repository id `<org>/<name>`, or `models--<name>` for one without an org.
pub fn repo_folder_name(repo_id: &str) -> String {
    format!("{MODEL_FOLDER_PREFIX}{}", repo_id.replace('/', REPOSITORY_SEPARATOR))
}

/// The repository's folder inside the hub cache.
pub fn repo_dir(hub_root: &Path, repo_id: &str) -> PathBuf {
    hub_root.join(repo_folder_name(repo_id))
}

/// `blobs/<name>`: a finished file.
pub fn blob_path(repo_dir: &Path, blob_name: &str) -> PathBuf {
    repo_dir.join(BLOBS_FOLDER).join(blob_name)
}

/// `blobs/<name>.incomplete`: a file being downloaded. Its length is where a resumed download continues.
pub fn incomplete_path(repo_dir: &Path, blob_name: &str) -> PathBuf {
    let incomplete_name = format!("{blob_name}{INCOMPLETE_SUFFIX}");
    repo_dir.join(BLOBS_FOLDER).join(incomplete_name)
}

/// `snapshots/<commit>/<file>`: how a commit names one of its files.
pub fn snapshot_path(repo_dir: &Path, commit: &str, file_name: &str) -> PathBuf {
    repo_dir.join(SNAPSHOTS_FOLDER).join(commit).join(file_name)
}

/// The relative target of a snapshot link: `../../blobs/<name>`, with one more `../` for each folder the file name
/// has in it (`sub/config.json` sits one folder deeper than `config.json`).
pub fn snapshot_link_target(file_name: &str, blob_name: &str) -> PathBuf {
    let folders_in_name = Path::new(file_name).components().count().saturating_sub(1);
    let mut target = PathBuf::new();
    for _ in 0..(SNAPSHOT_TO_REPOSITORY_LEVELS + folders_in_name) {
        target.push(Component::ParentDir);
    }
    target.push(BLOBS_FOLDER);
    target.push(blob_name);
    target
}

/// `refs/<revision>`: the file that holds the commit a branch or tag names.
pub fn ref_path(repo_dir: &Path, revision: &str) -> PathBuf {
    repo_dir.join(REFS_FOLDER).join(revision)
}

/// `<hub cache>/.locks/<repo folder>/<name>.lock`: the lock file huggingface_hub uses for one blob. The path is under
/// the hub cache, not under Rastai's own cache, so the other HF tools take the same lock.
pub fn lock_path(hub_root: &Path, repo_id: &str, blob_name: &str) -> PathBuf {
    let lock_name = format!("{blob_name}{LOCK_SUFFIX}");
    hub_root
        .join(LOCKS_FOLDER)
        .join(repo_folder_name(repo_id))
        .join(lock_name)
}

/// Whether a name from the Hub, or from a caller, can be joined under a folder: relative, with no `..`, no `.`, no
/// root and no empty part. A name that fails it would write outside the cache.
pub fn is_safe_relative_name(name: &str) -> bool {
    let path = Path::new(name);
    let mut has_part = false;
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return false;
        }
        has_part = true;
    }
    has_part && !name.contains('\\')
}

/// Points `snapshots/<commit>/<file>` at its blob, replacing a link from an earlier download. The link is relative, so
/// the cache can move with its repository folder.
pub fn link_snapshot(repo_dir: &Path, commit: &str, file_name: &str, blob_name: &str) -> io::Result<()> {
    let link = snapshot_path(repo_dir, commit, file_name);
    if let Some(folder) = link.parent() {
        fs::create_dir_all(folder)?;
    }
    remove_existing_link(&link)?;
    create_symlink(&snapshot_link_target(file_name, blob_name), &link)
}

/// Writes `refs/<revision>` holding the commit. The file is replaced, so a branch that moved is read again.
pub fn write_ref(repo_dir: &Path, revision: &str, commit: &str) -> io::Result<()> {
    let ref_file = ref_path(repo_dir, revision);
    if let Some(folder) = ref_file.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::write(ref_file, commit)
}

fn remove_existing_link(link: &Path) -> io::Result<()> {
    match fs::remove_file(link) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows symbolic links need the tier 2 work (ADR-0033 decision 5); until then a download there stops at the link.
#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links aren't supported on this platform yet",
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn repository_folders_follow_huggingface_hub() {
        assert_eq!(repo_folder_name("google/gemma-2b"), "models--google--gemma-2b");
        assert_eq!(repo_folder_name("gpt2"), "models--gpt2");
    }

    #[test]
    fn snapshot_links_climb_one_level_per_folder_of_the_name() {
        assert_eq!(
            snapshot_link_target("config.json", "abc"),
            PathBuf::from("../../blobs/abc")
        );
        assert_eq!(
            snapshot_link_target("layers/0/sae.safetensors", "def"),
            PathBuf::from("../../../../blobs/def")
        );
    }

    #[test]
    fn lock_paths_sit_under_the_hub_cache_where_huggingface_hub_looks() {
        let path = lock_path(Path::new("/cache/hub"), "org/name", "etag1");
        assert_eq!(path, PathBuf::from("/cache/hub/.locks/models--org--name/etag1.lock"));
    }

    #[test]
    fn names_that_could_leave_the_cache_are_refused() {
        assert!(is_safe_relative_name("config.json"));
        assert!(is_safe_relative_name("refs/pr/1"));
        assert!(!is_safe_relative_name("../secret"));
        assert!(!is_safe_relative_name("a/../../b"));
        assert!(!is_safe_relative_name("/etc/passwd"));
        assert!(!is_safe_relative_name("./config.json"));
        assert!(!is_safe_relative_name(""));
        assert!(!is_safe_relative_name("a\\b"));
    }

    #[test]
    fn the_incomplete_and_finished_blob_names_differ_only_by_the_suffix() {
        let repo = Path::new("/cache/hub/models--org--name");
        assert_eq!(
            blob_path(repo, "etag1"),
            PathBuf::from("/cache/hub/models--org--name/blobs/etag1")
        );
        assert_eq!(
            incomplete_path(repo, "etag1"),
            PathBuf::from("/cache/hub/models--org--name/blobs/etag1.incomplete")
        );
    }
}
