//! The Hugging Face hub cache's layout (contracts 8.1; layout rust-node-gateway-proto.md 7.9), written the way
//! huggingface_hub writes it, so every HF tool reads what the node downloads. A repository's folder
//! `models--<org>--<name>/` holds `blobs/<name>` (a file's content, named by its hash), `snapshots/<commit>/<file>`
//! (relative symlinks into `blobs/`) and `refs/<revision>` (the commit a branch or tag names). Weights are never
//! copied (contracts section 19). `.locks/<repo folder>/<name>.lock` in the same cache is huggingface_hub's per-file
//! lock, which downloads.rs takes before it writes a blob; its path is `rastai_core::paths::hf_lock_path`, the one
//! function that builds it.
//!
//! Snapshot links and refs are written under a temporary name and renamed into place, so a reader or a second
//! download never sees the link missing, and two downloads that link at once both succeed.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The folder-name prefix huggingface_hub gives a model repository.
const MODEL_FOLDER_PREFIX: &str = "models--";
/// Where huggingface_hub puts the "--" in place of a repository id's "/".
const REPOSITORY_SEPARATOR: &str = "--";
const BLOBS_FOLDER: &str = "blobs";
const SNAPSHOTS_FOLDER: &str = "snapshots";
const REFS_FOLDER: &str = "refs";
/// A blob being downloaded has this suffix until its hash is checked and it is renamed to its name.
const INCOMPLETE_SUFFIX: &str = ".incomplete";
/// The suffix of a link or ref being written, before it is renamed to its name.
const TEMPORARY_SUFFIX: &str = "tmp";
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
/// the cache can move with its repository folder. It is made under a temporary name and renamed over the old link, so
/// the link is never missing and two downloads of one file, linking at once, both succeed.
pub fn link_snapshot(repo_dir: &Path, commit: &str, file_name: &str, blob_name: &str) -> io::Result<()> {
    let link = snapshot_path(repo_dir, commit, file_name);
    if let Some(folder) = link.parent() {
        fs::create_dir_all(folder)?;
    }
    let temporary = temporary_sibling(&link);
    remove_existing_link(&temporary)?;
    create_symlink(&snapshot_link_target(file_name, blob_name), &temporary)?;
    rename_into_place(&temporary, &link)
}

/// Writes `refs/<revision>` holding the commit. The file is written under a temporary name and renamed over the old one,
/// so a branch that moved is read again and a reader never sees a half-written commit.
pub fn write_ref(repo_dir: &Path, revision: &str, commit: &str) -> io::Result<()> {
    let ref_file = ref_path(repo_dir, revision);
    if let Some(folder) = ref_file.parent() {
        fs::create_dir_all(folder)?;
    }
    let temporary = temporary_sibling(&ref_file);
    fs::write(&temporary, commit)?;
    rename_into_place(&temporary, &ref_file)
}

/// A name in the same folder as `path` that no other write in this process uses, so each link or ref is written
/// beside its final name and the rename stays within one folder.
fn temporary_sibling(path: &Path) -> PathBuf {
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{sequence}.{TEMPORARY_SUFFIX}", std::process::id()))
}

/// Gives a written link or ref its name. A failed rename removes the temporary name, so no half-made file is left.
fn rename_into_place(temporary: &Path, path: &Path) -> io::Result<()> {
    match fs::rename(temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error)
        }
    }
}

/// Counts the temporary names this process has handed out, so two writes at once never share one.
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

    /// Eight threads link the same file of one snapshot 200 times each. Every call must succeed, the link must end at
    /// its blob, and no temporary name may be left behind. Before the rename, one thread could remove a link that
    /// another had just made and then fail to create it again.
    #[cfg(unix)]
    #[test]
    fn concurrent_links_of_one_file_all_succeed_and_leave_no_temporary_name() {
        let folder = std::env::temp_dir().join(format!("rastai-hf-link-race-{}", std::process::id()));
        let repo = folder.join("models--org--name");
        let blob_name = "etag1";
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        link_snapshot(&repo, "commit1", "model.safetensors", blob_name).unwrap();
                    }
                });
            }
        });
        let link = snapshot_path(&repo, "commit1", "model.safetensors");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            snapshot_link_target("model.safetensors", blob_name)
        );
        let folder_entries = fs::read_dir(link.parent().unwrap()).unwrap().count();
        assert_eq!(folder_entries, 1, "no temporary link is left next to the snapshot");
        fs::remove_dir_all(&folder).unwrap();
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
