//! Where the repository is. Every generator reads its sources from the repository root and, unless told otherwise,
//! writes its outputs there too.

use std::path::{Path, PathBuf};

/// This crate's folder is tools/codegen/, two levels below the repository root.
const LEVELS_BELOW_REPO_ROOT: usize = 2;

/// The repository root, found from this crate's own folder. Cargo fills CARGO_MANIFEST_DIR at compile time, so the
/// answer doesn't depend on the folder `cargo run` was started from. The generators are dev tools that are only
/// ever run from a checkout, through `just codegen` or `just codegen-check`, so a compile-time path is enough.
pub fn find_repo_root() -> PathBuf {
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    for _ in 0..LEVELS_BELOW_REPO_ROOT {
        root.pop();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_root_holds_the_workspace_manifest_and_this_crate() {
        let root = find_repo_root();
        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("tools/codegen/Cargo.toml").is_file());
    }
}
