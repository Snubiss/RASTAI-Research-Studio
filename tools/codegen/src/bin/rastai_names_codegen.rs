//! rastai-names-codegen: collects every name the code has, then writes `schemas/names.json` (contracts 12.6,
//! layout rust-control-plane.md 4.9). `just codegen` runs it last, the order layout 4.9 gives: it reads the catalog,
//! the core tables, the migrations and the type descriptions, so it sees each of them after the other steps.
//!
//! Usage: `rastai-names-codegen [--output <folder>]`. The names are read from the repository either way; `--output`
//! only says where names.json goes, so check-generated can compare a fresh run with what is checked in.
// The one line on stdout says what changed; the only other output, an error, goes to stderr.
#![allow(clippy::print_stdout)]

use std::path::Path;
use std::process::ExitCode;

use rastai_codegen::error::CodegenError;
use rastai_codegen::names_registry::{collect_names, compiled_names, write_names_registry};
use rastai_codegen::output_root::{EXIT_USAGE, read_output_root};
use rastai_codegen::repo_root::find_repo_root;

const PROGRAM_NAME: &str = "rastai-names-codegen";
const USAGE: &str = "usage: rastai-names-codegen [--output <folder>]";

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
    match generate_names(&repo_root, &output_root) {
        Ok(changed) => {
            let changed_count = usize::from(changed);
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

/// Collects the names from the repository and writes names.json under `output_root`. Returns whether it changed.
fn generate_names(repo_root: &Path, output_root: &Path) -> Result<bool, CodegenError> {
    let registry = collect_names(repo_root, &compiled_names())?;
    write_names_registry(output_root, &registry)
}
