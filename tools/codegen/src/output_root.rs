//! The one command-line option every generator takes: `--output <folder>`. Outputs go into the repository unless it
//! names another folder laid out like it; check-generated uses that to compare a fresh run with what is checked in.

use std::path::{Path, PathBuf};

pub const OUTPUT_FLAG: &str = "--output";
/// The usual exit status for wrong arguments, distinct from a generator's own failure (1).
pub const EXIT_USAGE: u8 = 2;

/// The folder to write into: the repository root, or the folder after `--output`. None for any other arguments.
pub fn read_output_root(arguments: &[String], repo_root: &Path) -> Option<PathBuf> {
    match arguments {
        [] => Some(repo_root.to_path_buf()),
        [flag, folder] if flag == OUTPUT_FLAG => Some(PathBuf::from(folder)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_the_repository() {
        let root = Path::new("/repo");
        assert_eq!(read_output_root(&[], root), Some(root.to_path_buf()));
    }

    #[test]
    fn the_output_flag_names_another_folder() {
        let output = read_output_root(&arguments(&["--output", "/tmp/tree"]), Path::new("/repo"));
        assert_eq!(output, Some(PathBuf::from("/tmp/tree")));
    }

    #[test]
    fn anything_else_is_refused() {
        for words in [
            &["--output"][..],
            &["--out", "/tmp"],
            &["/tmp"],
            &["--output", "/a", "/b"],
        ] {
            assert_eq!(
                read_output_root(&arguments(words), Path::new("/repo")),
                None,
                "{words:?}"
            );
        }
    }
}
