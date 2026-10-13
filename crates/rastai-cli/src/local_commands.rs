//! The commands that work without a daemon (layout: rust-control-plane.md 9.7): `validate`, which is pure
//! rastai-formats, `version`, and `storage rebuild-index`, which M1-33 builds (it holds `daemon.lock`, so it can't
//! be a registry command the daemon runs).

use std::fs::File;
use std::path::Path;

use rastai_core::versions::APP_VERSION;
use rastai_diag::diagnostic::{Diagnostic, diagnostic_at, has_errors};
use rastai_formats::byte_source::{FileSource, SliceSource};
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::read_document;
use serde_json::json;

use crate::error::{CliError, not_available_yet};
use crate::exit_codes::{EXIT_FAILED, EXIT_OK};
use crate::modes::LocalCommand;
use crate::output::{OutputSettings, print_document_report, print_line};

const CODE_SOURCE_UNREADABLE: &str = "RRS-D3001";
const REBUILD_INDEX_WORDS: &str = "storage rebuild-index";

/// Runs a local command and returns its exit code.
pub fn run_local_command(command: &LocalCommand, settings: OutputSettings) -> Result<u8, CliError> {
    match command {
        LocalCommand::Validate(files) => Ok(validate_files(files, settings)),
        LocalCommand::Version => {
            print_line(settings, APP_VERSION, &json!({ "version": APP_VERSION }));
            Ok(EXIT_OK)
        }
        LocalCommand::RebuildIndex { .. } => Err(not_available_yet(REBUILD_INDEX_WORDS)),
    }
}

/// `rastai validate <file>...`: every file's diagnostics, the four layers of `read_document`. `EXIT_FAILED` when
/// any file has an error; warnings alone pass.
pub fn validate_files(files: &[String], settings: OutputSettings) -> u8 {
    let mut any_error = false;
    for file in files {
        let (diagnostics, source_text) = validate_file(file);
        any_error |= has_errors(&diagnostics);
        print_document_report(settings, file, &diagnostics, &source_text);
    }
    if any_error { EXIT_FAILED } else { EXIT_OK }
}

/// One file's diagnostics, and its text for the snippets. The diagnostics' paths start with the file as the user
/// named it. A file larger than the import limit isn't read into memory: `read_document` refuses it from its size
/// (`RRS-E5003`), and its diagnostic has no snippet.
fn validate_file(file: &str) -> (Vec<Diagnostic>, String) {
    let limits = ImportLimits::default();
    let opened = match File::open(Path::new(file)) {
        Ok(opened) => opened,
        Err(error) => return (vec![unreadable(file, &error.to_string())], String::new()),
    };
    let length = opened.metadata().map(|metadata| metadata.len()).unwrap_or(u64::MAX);
    if length > limits.max_file_bytes {
        let outcome = read_document(file, &FileSource { file: opened }, &limits);
        return (outcome.diagnostics, String::new());
    }
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) => return (vec![unreadable(file, &error.to_string())], String::new()),
    };
    let outcome = read_document(file, &SliceSource { bytes: &bytes }, &limits);
    (outcome.diagnostics, String::from_utf8_lossy(&bytes).into_owned())
}

fn unreadable(file: &str, reason: &str) -> Diagnostic {
    diagnostic_at(CODE_SOURCE_UNREADABLE, &[("reason", reason)], file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_unreadable() {
        let (diagnostics, _) = validate_file("no-such-file.rrsmodel");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "RRS-D3001");
        assert_eq!(diagnostics[0].path, "no-such-file.rrsmodel");
    }
}
