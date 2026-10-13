//! The exit code table (layout: rust-control-plane.md 9.5) and the rules that pick a code from an outcome. Scripts
//! and CI read these numbers, so they never change meaning.

use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_proto::common::v1::JobState;

/// Success; a job that `succeeded`.
pub const EXIT_OK: u8 = 0;
/// The command ran and failed: `ok: false`, a job `failed` or `cancelled`, `rastai validate` found errors.
pub const EXIT_FAILED: u8 = 1;
/// An unknown verb, a bad flag, arguments invalid before sending.
pub const EXIT_USAGE: u8 = 2;
/// The daemon can't be reached or started, or it lacks an API package this CLI needs.
pub const EXIT_DAEMON_UNAVAILABLE: u8 = 3;
/// A credential or scope refusal (`RRS-C1xxx`).
pub const EXIT_NOT_PERMITTED: u8 = 4;
/// The work couldn't run at all: a job that failed before it started.
pub const EXIT_ERRORED: u8 = 5;

/// Every auth and scope code is `RRS-C1xxx` (contracts 10.1).
const NOT_PERMITTED_CODE_PREFIX: &str = "RRS-C1";
/// The daemon's own usage refusals: an unknown command and invalid arguments. The CLI checks both before sending,
/// so the daemon only finds what the local check can't see (a plugin command's types, a value's meaning).
const USAGE_CODES: &[&str] = &["RRS-C0001", "RRS-C0002"];

/// The exit code of a command the daemon refused or that failed (`ok: false`). The first error decides: a scope or
/// credential refusal is `EXIT_NOT_PERMITTED`, a usage refusal `EXIT_USAGE`, anything else `EXIT_FAILED`.
pub fn exit_code_for_failure(diagnostics: &[Diagnostic]) -> u8 {
    let first_error = diagnostics.iter().find(|item| item.severity == Severity::Error);
    let Some(error) = first_error else {
        return EXIT_FAILED;
    };
    if error.code.starts_with(NOT_PERMITTED_CODE_PREFIX) {
        return EXIT_NOT_PERMITTED;
    }
    if USAGE_CODES.contains(&error.code.as_str()) {
        return EXIT_USAGE;
    }
    EXIT_FAILED
}

/// The exit code of a job that reached a terminal state. `last_live_state` is the state it left to get there: a job
/// that failed from `queued` or `starting` never ran, so its work errored rather than failed (contracts 9.1).
pub fn exit_code_for_job(final_state: JobState, last_live_state: JobState) -> u8 {
    match final_state {
        JobState::Succeeded => EXIT_OK,
        JobState::Failed if never_started(last_live_state) => EXIT_ERRORED,
        _ => EXIT_FAILED,
    }
}

fn never_started(state: JobState) -> bool {
    matches!(state, JobState::Queued | JobState::Starting)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rastai_diag::diagnostic::diagnostic;

    #[test]
    fn the_table_has_the_contract_numbers() {
        let codes = [
            EXIT_OK,
            EXIT_FAILED,
            EXIT_USAGE,
            EXIT_DAEMON_UNAVAILABLE,
            EXIT_NOT_PERMITTED,
            EXIT_ERRORED,
        ];
        assert_eq!(codes, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_first_error_decides_a_failure() {
        let scope = diagnostic("RRS-C1005", &[]);
        let unknown = diagnostic("RRS-C0001", &[]);
        let other = diagnostic("RRS-S3001", &[]);
        let warning = diagnostic("RRS-C0006", &[]);
        assert_eq!(exit_code_for_failure(&[warning.clone(), scope]), EXIT_NOT_PERMITTED);
        assert_eq!(exit_code_for_failure(&[unknown]), EXIT_USAGE);
        assert_eq!(exit_code_for_failure(&[other]), EXIT_FAILED);
        assert_eq!(exit_code_for_failure(&[warning]), EXIT_FAILED);
        assert_eq!(exit_code_for_failure(&[]), EXIT_FAILED);
    }

    #[test]
    fn a_job_that_never_started_errored() {
        assert_eq!(exit_code_for_job(JobState::Succeeded, JobState::Running), EXIT_OK);
        assert_eq!(exit_code_for_job(JobState::Failed, JobState::Running), EXIT_FAILED);
        assert_eq!(exit_code_for_job(JobState::Failed, JobState::Starting), EXIT_ERRORED);
        assert_eq!(exit_code_for_job(JobState::Failed, JobState::Queued), EXIT_ERRORED);
        assert_eq!(exit_code_for_job(JobState::Cancelled, JobState::Queued), EXIT_FAILED);
        assert_eq!(exit_code_for_job(JobState::Cancelled, JobState::Running), EXIT_FAILED);
    }
}
