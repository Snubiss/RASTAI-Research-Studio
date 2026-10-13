//! `CliError`, the one error type of this crate (contracts 12.2). Every variant carries catalog diagnostics, which
//! output.rs prints, and maps to one row of the exit code table (exit_codes.rs).

use rastai_diag::diagnostic::{Diagnostic, diagnostic};

use crate::exit_codes::{EXIT_DAEMON_UNAVAILABLE, EXIT_FAILED, EXIT_NOT_PERMITTED, EXIT_USAGE};

const CODE_INVALID_ARGUMENTS: &str = "RRS-C0002";
const CODE_NOT_AVAILABLE_YET: &str = "RRS-C0010";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// An unknown verb, a bad flag, or arguments found invalid before sending (`EXIT_USAGE`).
    Usage(Vec<Diagnostic>),
    /// The daemon can't be reached or started, or it lacks an API package (`EXIT_DAEMON_UNAVAILABLE`).
    DaemonUnavailable(Vec<Diagnostic>),
    /// The daemon's gate refused the credential or the scope (`EXIT_NOT_PERMITTED`).
    NotPermitted(Vec<Diagnostic>),
    /// Anything else that stopped the invocation (`EXIT_FAILED`).
    Failed(Vec<Diagnostic>),
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Usage(_) => EXIT_USAGE,
            CliError::DaemonUnavailable(_) => EXIT_DAEMON_UNAVAILABLE,
            CliError::NotPermitted(_) => EXIT_NOT_PERMITTED,
            CliError::Failed(_) => EXIT_FAILED,
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            CliError::Usage(diagnostics)
            | CliError::DaemonUnavailable(diagnostics)
            | CliError::NotPermitted(diagnostics)
            | CliError::Failed(diagnostics) => diagnostics,
        }
    }
}

/// A command line the CLI can't use: `RRS-C0002` naming what it found and what it expected there.
pub fn usage_error(found: &str, expected: &str) -> CliError {
    CliError::Usage(vec![diagnostic(
        CODE_INVALID_ARGUMENTS,
        &[("found", found), ("expected", expected)],
    )])
}

/// A mode whose task hasn't landed yet (`node serve`, `tunnel`, ...): `RRS-C0010` naming its words.
pub fn not_available_yet(words: &str) -> CliError {
    CliError::Failed(vec![diagnostic(CODE_NOT_AVAILABLE_YET, &[("command_id", words)])])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_has_its_exit_code() {
        assert_eq!(CliError::Usage(Vec::new()).exit_code(), 2);
        assert_eq!(CliError::DaemonUnavailable(Vec::new()).exit_code(), 3);
        assert_eq!(CliError::NotPermitted(Vec::new()).exit_code(), 4);
        assert_eq!(CliError::Failed(Vec::new()).exit_code(), 1);
    }

    #[test]
    fn a_usage_error_names_what_was_found() {
        let error = usage_error("--colour", "a flag of system.status");
        assert_eq!(error.diagnostics()[0].code, "RRS-C0002");
        assert!(error.diagnostics()[0].message.contains("--colour"));
    }
}
