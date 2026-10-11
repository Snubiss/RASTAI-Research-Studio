//! What a command handler returns (layout rust-control-plane.md 5.2). Handlers return
//! `Result<CommandOutput, Vec<Diagnostic>>`: failures are always catalog diagnostics, so there is no separate
//! command error type (contracts 6.2, flows/platform.md 3.1).

use rastai_diag::diagnostic::Diagnostic;

/// A command's successful outcome. `Execute` copies it into `ExecuteResponse`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// The result as JSON text; empty for a `job` or `stream` command, which answers with `job_id`.
    pub result_json: String,
    /// The job a `job` command started; empty otherwise.
    pub job_id: String,
    /// Problems that didn't stop the command (warnings and info only).
    pub warnings: Vec<Diagnostic>,
}
