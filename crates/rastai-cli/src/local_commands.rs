//! The commands that work without a daemon (layout: rust-control-plane.md 9.7): `validate`, which is pure
//! rastai-formats, `version`, and `storage rebuild-index`, which holds `daemon.lock` while rastai-store rebuilds
//! `rastai.db` (layout 13.2), so it can't be a registry command the daemon runs, and refuses while a daemon runs.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use rastai_core::paths::DataPaths;
use rastai_core::versions::APP_VERSION;
use rastai_diag::diagnostic::{Diagnostic, diagnostic_at, has_errors};
use rastai_formats::byte_source::SliceSource;
use rastai_formats::error::FormatsError;
use rastai_formats::limits::ImportLimits;
use rastai_formats::validate::read_document;
use rastai_formats::validate::syntax::syntax_diagnostic;
use rastai_store::data_dir_lock::take_daemon_lock;
use rastai_store::error::store_error_diagnostic;
use rastai_store::rebuild::{RebuildOptions, RebuildReport, rebuild_index};
use serde_json::json;

use crate::daemon_client::data_paths;
use crate::error::CliError;
use crate::exit_codes::{EXIT_FAILED, EXIT_OK};
use crate::modes::LocalCommand;
use crate::output::{OutputSettings, print_document_report, print_line};

const CODE_SOURCE_UNREADABLE: &str = "RRS-D3001";
/// The kill -9 test's pause point for the rebuild (contracts 15.9). Only a debug build reads it.
const TEST_PAUSE_VARIABLE: &str = "RASTAI_TEST_PAUSE_AT";

/// How a local command ended: its exit code, and the problems it passed over, which the final line reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOutcome {
    pub exit_code: u8,
    pub diagnostics: Vec<Diagnostic>,
}

/// Runs a local command.
pub fn run_local_command(command: &LocalCommand, settings: OutputSettings) -> Result<LocalOutcome, CliError> {
    match command {
        LocalCommand::Validate(files) => Ok(clean_outcome(validate_files(files, settings))),
        LocalCommand::Version => {
            print_line(settings, APP_VERSION, &json!({ "version": APP_VERSION }));
            Ok(clean_outcome(EXIT_OK))
        }
        LocalCommand::RebuildIndex { verify_blobs } => {
            let paths = data_paths()?;
            run_rebuild_index(&paths, *verify_blobs, settings)
        }
    }
}

fn clean_outcome(exit_code: u8) -> LocalOutcome {
    LocalOutcome {
        exit_code,
        diagnostics: Vec::new(),
    }
}

/// `rastai storage rebuild-index [--verify-blobs]`: takes `daemon.lock`, so it refuses with `RRS-S0001` while a
/// daemon runs and no daemon can start until it is done, then rebuilds `rastai.db` and prints what it did. What the
/// rebuild passed over (a damaged log, a missing blob) doesn't fail it: the index holds everything that could be
/// read, and each problem is reported with its code.
pub fn run_rebuild_index(
    paths: &DataPaths,
    verify_blobs: bool,
    settings: OutputSettings,
) -> Result<LocalOutcome, CliError> {
    let lock = take_daemon_lock(paths).map_err(|error| CliError::Failed(vec![store_error_diagnostic(&error)]))?;
    let options = RebuildOptions {
        verify_blobs,
        pause_at: test_pause_point(),
    };
    let rebuilt = rebuild_index(paths, &options);
    drop(lock);
    let report = rebuilt.map_err(|error| CliError::Failed(vec![store_error_diagnostic(&error)]))?;
    print_rebuild_report(settings, &report);
    Ok(LocalOutcome {
        exit_code: EXIT_OK,
        diagnostics: report.diagnostics,
    })
}

/// The test-only pause point, from `RASTAI_TEST_PAUSE_AT` in a debug build; a release build never pauses.
fn test_pause_point() -> String {
    if !cfg!(debug_assertions) {
        return String::new();
    }
    std::env::var(TEST_PAUSE_VARIABLE).unwrap_or_default()
}

fn print_rebuild_report(settings: OutputSettings, report: &RebuildReport) {
    let old_index = report.old_index_path.as_ref().map(|path| path.display().to_string());
    let mut text = format!(
        "rastai.db rebuilt: {} runs, {} records, {} artifacts.",
        report.runs, report.records, report.artifacts
    );
    if let Some(path) = &old_index {
        text.push_str(&format!(" The old index is kept at {path} until you delete it."));
    }
    let value = json!({
        "runs": report.runs,
        "records": report.records,
        "artifacts": report.artifacts,
        "old_index_path": old_index,
    });
    print_line(settings, &text, &value);
}

/// `rastai validate <file>...`: every file's diagnostics, the four layers of `read_document`. `EXIT_FAILED` when
/// any file has an error; warnings alone pass.
pub fn validate_files(files: &[String], settings: OutputSettings) -> u8 {
    let mut any_error = false;
    let limits = ImportLimits::default();
    for file in files {
        let (diagnostics, source_text) = validate_file(file, &limits);
        any_error |= has_errors(&diagnostics);
        print_document_report(settings, file, &diagnostics, &source_text);
    }
    if any_error { EXIT_FAILED } else { EXIT_OK }
}

/// One file's diagnostics, and its text for the snippets. The diagnostics' paths start with the file as the user
/// named it. At most one byte past the import limit is ever read, whatever the file's size says (a pipe or a device
/// says 0, a growing file says too little): a file past the limit is refused with `RRS-E5003`, with no snippet.
fn validate_file(file: &str, limits: &ImportLimits) -> (Vec<Diagnostic>, String) {
    let opened = match File::open(Path::new(file)) {
        Ok(opened) => opened,
        Err(error) => return (vec![unreadable(file, &error.to_string())], String::new()),
    };
    let size_said = opened.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut bytes = Vec::new();
    let mut capped = opened.take(limits.max_file_bytes.saturating_add(1));
    if let Err(error) = capped.read_to_end(&mut bytes) {
        return (vec![unreadable(file, &error.to_string())], String::new());
    }
    let size_read = bytes.len() as u64;
    if size_read > limits.max_file_bytes {
        let too_large = FormatsError::FileTooLarge {
            size_bytes: size_said.max(size_read),
            limit_bytes: limits.max_file_bytes,
        };
        return (vec![syntax_diagnostic(&too_large, file)], String::new());
    }
    let outcome = read_document(file, &SliceSource { bytes: &bytes }, limits);
    (outcome.diagnostics, String::from_utf8_lossy(&bytes).into_owned())
}

fn unreadable(file: &str, reason: &str) -> Diagnostic {
    diagnostic_at(CODE_SOURCE_UNREADABLE, &[("reason", reason)], file)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A small limit, so the pipe test writes little.
    const SMALL_LIMIT_BYTES: u64 = 1_024;
    /// Far past the small limit, and more than a pipe buffers.
    const PIPE_BYTES: usize = 256 * 1_024;
    /// A capped read ends at once; one that waits for the end of the pipe never does.
    const READ_LIMIT_SECONDS: u64 = 20;

    /// A pipe says it holds 0 bytes, and its writer may never close it: only a capped read stops.
    #[cfg(unix)]
    #[test]
    fn a_pipe_is_read_no_further_than_the_limit() {
        let folder = std::env::temp_dir().join(format!("rastai-cli-pipe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let pipe = folder.join("endless.json");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status().unwrap();
        assert!(made.success());

        // The writer writes past the limit, then holds the pipe open until it is released.
        let (release, released) = mpsc::channel::<()>();
        let writer_path = pipe.clone();
        let writer = std::thread::spawn(move || {
            let mut file = std::fs::OpenOptions::new().write(true).open(&writer_path).unwrap();
            // Fails with a broken pipe once the reader stops, which is the point.
            let _ = file.write_all(&vec![b' '; PIPE_BYTES]);
            let _ = released.recv();
        });
        let (done, finished) = mpsc::channel();
        let reader_path = pipe.display().to_string();
        std::thread::spawn(move || {
            let limits = ImportLimits {
                max_file_bytes: SMALL_LIMIT_BYTES,
                ..ImportLimits::default()
            };
            let _ = done.send(validate_file(&reader_path, &limits));
        });
        let outcome = finished.recv_timeout(Duration::from_secs(READ_LIMIT_SECONDS));
        let _ = release.send(());
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);

        let (diagnostics, source_text) = outcome.expect("validate read the pipe past its limit, waiting for its end");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "RRS-E5003");
        assert!(
            diagnostics[0].message.contains("1025 bytes"),
            "{}",
            diagnostics[0].message
        );
        assert!(source_text.is_empty());
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let (diagnostics, _) = validate_file("no-such-file.rrsmodel", &ImportLimits::default());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "RRS-D3001");
        assert_eq!(diagnostics[0].path, "no-such-file.rrsmodel");
    }
}
