//! `CommandContext`, what every handler is given besides its arguments (layout: rust-control-plane.md 8.3), and the
//! few helpers handlers share to answer: a sync result, and the failures of the crates they call as diagnostics.
//!
//! Handlers never see HTTP: they take `(&CommandContext, Args)` and return `Result<CommandOutput, Vec<Diagnostic>>`,
//! so the CLI, MCP, plugin forwards and the UI all get the same command.

use rastai_core::commands::output::CommandOutput;
use rastai_core::error::CoreError;
use rastai_core::principal::Principal;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_store::error::StoreError;

use crate::server_state::Server;

/// One running command.
pub struct CommandContext<'a> {
    pub server: &'a Server,
    /// Who runs it, already narrowed by `acting_plugin_id` (flows/platform.md 3.2).
    pub principal: &'a Principal,
    /// The registry's descriptor of the command.
    pub command: &'a CommandDescriptor,
    /// The journal row of this run; empty for a dry run and for `read` commands, which aren't journaled.
    pub journal_id: String,
    /// The project the caller has open; empty outside one.
    pub project_id: String,
    pub idempotency_key: String,
    /// Check and plan only: a handler with side effects makes none (flows/platform.md: "plan, then do").
    pub dry_run: bool,
}

/// A sync command's answer: its result as JSON text, no job.
pub(crate) fn sync_result(result_json: String) -> CommandOutput {
    CommandOutput {
        result_json,
        ..CommandOutput::default()
    }
}

/// A store error as the command's failure. The store's own code says what failed; its words go in the reason.
pub(crate) fn store_failure(error: &StoreError) -> Vec<Diagnostic> {
    let reason = format!("{error:?}");
    vec![diagnostic(error.code(), &[("reason", &reason), ("path", "")])]
}

/// A rastai-core error (no random source for an id or a secret) as the command's failure.
pub(crate) fn core_failure(error: &CoreError) -> Vec<Diagnostic> {
    let reason = format!("{error:?}");
    vec![diagnostic(error.code(), &[("reason", &reason)])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_keep_the_code_of_the_crate_that_failed() {
        let store = store_failure(&StoreError::WriterStopped);
        assert_eq!(store[0].code, "RRS-S3005");
        let core = core_failure(&CoreError::RandomUnavailable {
            reason: "no /dev/urandom".to_string(),
        });
        assert_eq!(core[0].code, "RRS-C0502");
        assert!(core[0].message.contains("no /dev/urandom"), "{}", core[0].message);
    }

    #[test]
    fn a_sync_result_has_no_job() {
        let output = sync_result("{}".to_string());
        assert_eq!(output.result_json, "{}");
        assert!(output.job_id.is_empty());
        assert!(output.warnings.is_empty());
    }
}
