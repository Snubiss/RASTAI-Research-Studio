//! `CodegenError`, the one error type of rastai-codegen (contracts 12.2).
//!
//! It has no catalog code, unlike the errors of the shipped crates: it is read by the developer running
//! `just codegen`, never by a user, and the catalog is what this crate generates, so a broken catalog must still be
//! reported in plain words. It names the file at fault and says what is wrong with it.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError {
    /// The file that couldn't be read, parsed or written.
    pub path: PathBuf,
    /// What went wrong, as a sentence for the developer.
    pub problem: String,
}

impl fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.problem)
    }
}

/// Builds a `CodegenError` for a file.
pub fn codegen_error(path: &Path, problem: String) -> CodegenError {
    CodegenError {
        path: path.to_path_buf(),
        problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_file_first() {
        let error = codegen_error(
            Path::new("catalog/errors/RRS-E.json"),
            "entry 2: `code` is missing".to_string(),
        );
        assert_eq!(
            error.to_string(),
            "catalog/errors/RRS-E.json: entry 2: `code` is missing"
        );
    }
}
