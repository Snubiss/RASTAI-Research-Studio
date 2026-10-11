//! The core command table (contracts 6.2, 6.4; layout rust-control-plane.md 10.1): one `CommandRow` per core
//! command, and the commands that were retired (layout 11.2). Plugin commands come from manifests at runtime, never
//! from this file.
//!
//! Every row is a contract: it lands in a contract PR with its argument and result types and catalog codes, before
//! the handler (plan 3.2). The first rows, the skeleton's commands, arrive with M1-22. `tests/command_table.rs`
//! checks every row: id form and core area, no duplicates, scope, kind, section, `since_version`, `preview`.

use crate::commands::descriptor::CommandRow;

/// One row per core command (contracts 6.4).
pub const COMMANDS: &[CommandRow] = &[];

/// A command that was replaced. Its id is never reused, `ListCommands` doesn't list it, and `Execute` on it answers
/// `RRS-C0007` with a fix button that runs the replacement (layout 11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredCommand {
    pub id: &'static str,
    pub replaced_by: &'static str,
    /// The app version that retired it.
    pub retired_in: &'static str,
}

/// Commands that were replaced; none yet.
pub const RETIRED_COMMANDS: &[RetiredCommand] = &[];
