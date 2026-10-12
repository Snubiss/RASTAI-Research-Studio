//! rastai-formats-codegen: walks the type descriptions of rastai-formats and the core command and event tables, then
//! writes each output in turn (layout rust-control-plane.md 4.9):
//! - `schemas/` (one JSON Schema per document type and per command, one file per event type, and index.json),
//! - `packages/formats/src/generated/` (`@rastai/formats`: one JSDoc `@typedef` module per schema id),
//! - `packages/client/src/generated/commands.js` (one typed wrapper per core command),
//! - `python/rastai_formats/rastai_formats/generated/` (`rastai_formats.generated`: frozen dataclasses),
//! - `docs/reference/commands.md`.
//!
//! Usage: `rastai-formats-codegen [--output <folder>]`. Outputs go into the repository unless `--output` names
//! another folder laid out like it; check-generated uses that to compare a fresh run with what is checked in.
//! The event payloads' fields come from `buf build` run on the repository's proto/, so buf must be on the PATH (the
//! justfile's codegen recipes check its version first).
// The one line on stdout says what changed; the only other output, an error, goes to stderr.
#![allow(clippy::print_stdout)]

use std::path::Path;
use std::process::ExitCode;

use rastai_codegen::command_source::command_descriptions;
use rastai_codegen::descriptor_set::build_descriptor_set;
use rastai_codegen::error::CodegenError;
use rastai_codegen::event_source::event_descriptions;
use rastai_codegen::javascript_commands::write_javascript_commands;
use rastai_codegen::javascript_types::write_javascript_types;
use rastai_codegen::json_schema_files::write_json_schema_files;
use rastai_codegen::output_root::{EXIT_USAGE, read_output_root};
use rastai_codegen::python_dataclasses::write_python_dataclasses;
use rastai_codegen::reference_docs::write_commands_reference;
use rastai_codegen::repo_root::find_repo_root;
use rastai_formats::describe::registry::describe_all_types;
use rastai_formats::documents::DOCUMENT_TYPES;

const PROGRAM_NAME: &str = "rastai-formats-codegen";
const USAGE: &str = "usage: rastai-formats-codegen [--output <folder>]";

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
    match generate_formats(&repo_root, &output_root) {
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

/// Reads the descriptions, commands and event types once and writes each output in turn. Returns how many files
/// changed.
fn generate_formats(repo_root: &Path, output_root: &Path) -> Result<usize, CodegenError> {
    let all_types = describe_all_types();
    let commands = command_descriptions()?;
    let descriptor_set = build_descriptor_set(repo_root)?;
    let events = event_descriptions(&descriptor_set)?;
    let mut changed_count = write_json_schema_files(output_root, DOCUMENT_TYPES, &all_types, &commands, &events)?;
    changed_count += write_javascript_types(output_root, &all_types)?;
    changed_count += write_javascript_commands(output_root, &commands)?;
    changed_count += write_python_dataclasses(output_root, &all_types)?;
    changed_count += write_commands_reference(output_root, &commands)?;
    Ok(changed_count)
}
