//! rastai-catalog-codegen: reads catalog/ and the first-party plugin catalogs, then writes the catalog for every
//! language that needs it (layout rust-control-plane.md 4.9):
//! - `crates/rastai-diag/src/generated/` (the Rust tables),
//! - `packages/catalog/src/generated/catalog.js` (`@rastai/catalog`),
//! - `python/rastai_plugin/rastai_plugin/catalog/generated.py` (`rastai_plugin.catalog`),
//! - `docs/reference/errors.md`.
//!
//! Usage: `rastai-catalog-codegen [--output <folder>]`. Outputs go into the repository unless `--output` names
//! another folder laid out like it; check-generated uses that to compare a fresh run with what is checked in.
// The one line on stdout says what changed; the only other output, an error, goes to stderr.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rastai_codegen::catalog_javascript::write_catalog_javascript;
use rastai_codegen::catalog_python::write_catalog_python;
use rastai_codegen::catalog_rust::write_catalog_rust;
use rastai_codegen::catalog_source::read_catalog;
use rastai_codegen::error::CodegenError;
use rastai_codegen::reference_docs::write_errors_reference;
use rastai_codegen::repo_root::find_repo_root;

const PROGRAM_NAME: &str = "rastai-catalog-codegen";
const OUTPUT_FLAG: &str = "--output";
const USAGE: &str = "usage: rastai-catalog-codegen [--output <folder>]";
/// The usual exit status for wrong arguments, distinct from a catalog problem (1).
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let repo_root = match find_repo_root() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("{PROGRAM_NAME}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(output_root) = read_output_root(&arguments, &repo_root) else {
        eprintln!("{USAGE}");
        return ExitCode::from(EXIT_USAGE);
    };
    match generate_catalog(&repo_root, &output_root) {
        Ok(changed_count) => {
            println!(
                "{PROGRAM_NAME}: {changed_count} file(s) changed in {}",
                output_root.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{PROGRAM_NAME}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The folder to write into: the repository root, or the folder after `--output`. None for any other arguments.
fn read_output_root(arguments: &[String], repo_root: &Path) -> Option<PathBuf> {
    match arguments {
        [] => Some(repo_root.to_path_buf()),
        [flag, folder] if flag == OUTPUT_FLAG => Some(PathBuf::from(folder)),
        _ => None,
    }
}

/// Reads the catalog once and writes each output in turn. Returns how many files changed.
fn generate_catalog(repo_root: &Path, output_root: &Path) -> Result<usize, CodegenError> {
    let catalog = read_catalog(repo_root)?;
    let mut changed_count = write_catalog_rust(output_root, &catalog)?;
    changed_count += write_catalog_javascript(output_root, &catalog)?;
    changed_count += write_catalog_python(output_root, &catalog)?;
    changed_count += write_errors_reference(output_root, &catalog)?;
    Ok(changed_count)
}
