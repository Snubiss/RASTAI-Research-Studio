//! The core commands as the formats generator needs them: id, title, kind, stability, the version that added it,
//! and the names of its argument and result types (contracts 6.1, 6.4).
//!
//! The rows come from `rastai_core::commands::COMMANDS` once rastai-core's command table exists (M1-21 builds it,
//! M1-22 adds the first rows). Until then the list is empty, so `commands.js`, `schemas/commands/` and
//! `docs/reference/commands.md` hold no commands yet, and nothing here guesses at rows that aren't in code.

/// One core command, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescription {
    /// `model.load` (contracts 6.1).
    pub command_id: String,
    /// The palette title, such as "Load model".
    pub title: String,
    /// `sync`, `job` or `stream`.
    pub kind: String,
    /// `preview`, `stable` or `deprecated`. Every command is `preview` until Phase 2 (owner decision N3).
    pub stability: String,
    /// The app version that added the command, written as `x-rastai-since-version`.
    pub since_version: String,
    /// The described type of its arguments, such as `ModelLoadArgs`.
    pub arguments_type: String,
    /// The described type of its result, such as `ModelLoadResult`.
    pub result_type: String,
}

/// Every core command, sorted by id. Empty until rastai-core's `COMMANDS` table lands (M1-21, M1-22).
pub fn command_descriptions() -> Vec<CommandDescription> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // When rastai-core's table arrives, this becomes "every row is here, sorted by id".
    #[test]
    fn there_are_no_core_commands_before_the_command_table() {
        assert!(command_descriptions().is_empty());
    }
}
