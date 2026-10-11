//! Where the repository is. Every generator reads its sources from the repository root and, unless told otherwise,
//! writes its outputs there too.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{CodegenError, codegen_error};

/// The variable cargo sets to this crate's folder, tools/codegen/, when `cargo run` or `cargo test` starts a program.
const MANIFEST_FOLDER_VARIABLE: &str = "CARGO_MANIFEST_DIR";
/// The same folder as it was when this binary was compiled. Only a fallback: see `find_repo_root`.
const COMPILED_MANIFEST_FOLDER: &str = env!("CARGO_MANIFEST_DIR");
/// This crate's folder is tools/codegen/, two levels below the repository root.
const LEVELS_BELOW_REPO_ROOT: usize = 2;
/// Files every repository root holds, relative to it. A folder without them isn't this repository.
const WORKSPACE_MANIFEST: &str = "Cargo.toml";
const CODEGEN_MANIFEST: &str = "tools/codegen/Cargo.toml";

/// The repository root, two folders above this crate's folder.
///
/// The folder cargo passes at run time comes first. The compile-time path can't be trusted: cargo's freshness check
/// ignores where the checkout is, so two checkouts sharing one target folder share one binary, and it would carry the
/// other checkout's path (read and write the wrong repository, or one since deleted). `just codegen`,
/// `just codegen-check` and the tests all start the generators through cargo, so the run-time folder is always set
/// there; the compile-time one is only for a binary started by hand.
pub fn find_repo_root() -> Result<PathBuf, CodegenError> {
    let run_time_folder = std::env::var_os(MANIFEST_FOLDER_VARIABLE);
    repo_root_from(run_time_folder.as_deref())
}

/// The repository root for a given crate folder (the run-time CARGO_MANIFEST_DIR), or for the compile-time one when
/// there is none. Refuses a root that doesn't hold the workspace manifest and this crate's manifest.
pub fn repo_root_from(manifest_folder: Option<&OsStr>) -> Result<PathBuf, CodegenError> {
    let manifest_folder = match manifest_folder {
        Some(folder) => Path::new(folder),
        None => Path::new(COMPILED_MANIFEST_FOLDER),
    };
    let mut root = manifest_folder.to_path_buf();
    for _ in 0..LEVELS_BELOW_REPO_ROOT {
        root.pop();
    }
    check_repo_root(&root, manifest_folder)?;
    Ok(root)
}

fn check_repo_root(root: &Path, manifest_folder: &Path) -> Result<(), CodegenError> {
    for required_file in [WORKSPACE_MANIFEST, CODEGEN_MANIFEST] {
        if !root.join(required_file).is_file() {
            let problem = format!(
                "not the repository root: {required_file} is missing (found from {MANIFEST_FOLDER_VARIABLE} {}); \
                 run the generators through `just codegen` or `cargo run -p rastai-codegen` in a checkout",
                manifest_folder.display()
            );
            return Err(codegen_error(root, problem));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn scratch_folder(test_name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-codegen-root-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn repo_root_holds_the_workspace_manifest_and_this_crate() {
        let root = find_repo_root().unwrap();
        assert!(root.join(WORKSPACE_MANIFEST).is_file());
        assert!(root.join(CODEGEN_MANIFEST).is_file());
    }

    #[test]
    fn run_time_folder_wins_over_the_compiled_one() {
        // A second checkout: the compiled path points at this one, so only the run-time folder can find it.
        let other_checkout = scratch_folder("other-checkout");
        let other_crate_folder = other_checkout.join("tools/codegen");
        std::fs::create_dir_all(&other_crate_folder).unwrap();
        std::fs::write(other_checkout.join(WORKSPACE_MANIFEST), "").unwrap();
        std::fs::write(other_checkout.join(CODEGEN_MANIFEST), "").unwrap();
        let root = repo_root_from(Some(other_crate_folder.as_os_str())).unwrap();
        assert_eq!(root, other_checkout);
        std::fs::remove_dir_all(&other_checkout).unwrap();
    }

    #[test]
    fn compiled_folder_is_the_fallback() {
        let root = repo_root_from(None).unwrap();
        assert_eq!(root.join("tools/codegen"), Path::new(COMPILED_MANIFEST_FOLDER));
    }

    #[test]
    fn a_folder_that_is_no_checkout_is_refused() {
        // Like a deleted scratch checkout whose path a stale binary still carries.
        let scratch = scratch_folder("missing-checkout");
        let missing_crate_folder = scratch.join("gone/tools/codegen");
        let error = repo_root_from(Some(missing_crate_folder.as_os_str())).unwrap_err();
        assert!(error.problem.contains("Cargo.toml is missing"), "{error}");
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn a_root_without_this_crate_is_refused() {
        let other_workspace = scratch_folder("other-workspace");
        std::fs::write(other_workspace.join(WORKSPACE_MANIFEST), "").unwrap();
        let crate_folder = other_workspace.join("tools/codegen");
        let error = repo_root_from(Some(crate_folder.as_os_str())).unwrap_err();
        assert!(error.problem.contains(CODEGEN_MANIFEST), "{error}");
        std::fs::remove_dir_all(&other_workspace).unwrap();
    }
}
