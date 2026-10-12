//! The core commands as the formats generator needs them: id, title, kind, stability, the version that added it,
//! and the names of its argument and result types (contracts 6.1, 6.4).
//!
//! The rows come from rastai-core's `COMMANDS` table, and each command's two type names from rastai-formats'
//! `describe_arguments` and `describe_result`, so `commands.js`, `schemas/commands/` and
//! `docs/reference/commands.md` hold exactly the commands the daemon registers.

use std::path::Path;

use rastai_core::commands::descriptor::CommandRow;
use rastai_core::commands::table::COMMANDS;
use rastai_formats::commands::{describe_arguments, describe_result};
use rastai_proto::commands::v1::CommandKind;
use rastai_proto::common::v1::Stability;

use crate::error::{CodegenError, codegen_error};

/// Where a command's argument and result types are listed; a row without them is fixed there.
const COMMAND_TYPES_FILE: &str = "crates/rastai-formats/src/commands/mod.rs";

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

/// Every core command, sorted by id. Fails for a row whose types rastai-formats doesn't describe.
pub fn command_descriptions() -> Result<Vec<CommandDescription>, CodegenError> {
    let mut commands = Vec::new();
    for row in COMMANDS {
        commands.push(command_description(row)?);
    }
    commands.sort_by(|left, right| left.command_id.cmp(&right.command_id));
    Ok(commands)
}

fn command_description(row: &CommandRow) -> Result<CommandDescription, CodegenError> {
    let missing = |what: &str| {
        let problem = format!("the command `{}` has no {what} type in COMMAND_TYPES", row.command_id);
        codegen_error(Path::new(COMMAND_TYPES_FILE), problem)
    };
    let arguments = describe_arguments(row.command_id).ok_or_else(|| missing("argument"))?;
    let result = describe_result(row.command_id).ok_or_else(|| missing("result"))?;
    Ok(CommandDescription {
        command_id: row.command_id.to_string(),
        title: row.title.to_string(),
        kind: kind_word(row.kind).to_string(),
        stability: stability_word(row.stability).to_string(),
        since_version: row.since_version.to_string(),
        arguments_type: arguments.name,
        result_type: result.name,
    })
}

/// The kind as contracts 6.1 writes it. `command_table.rs` refuses an unspecified kind, so it never reaches here.
fn kind_word(kind: CommandKind) -> &'static str {
    match kind {
        CommandKind::Sync => "sync",
        CommandKind::Job => "job",
        CommandKind::Stream => "stream",
        CommandKind::Unspecified => "unspecified",
    }
}

/// The stability as contracts 4.1 and the schema files write it. The event files use it too.
pub fn stability_word(stability: Stability) -> &'static str {
    match stability {
        Stability::Preview => "preview",
        Stability::Stable => "stable",
        Stability::Deprecated => "deprecated",
        Stability::Unspecified => "unspecified",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_here_sorted_by_id() {
        let commands = command_descriptions().unwrap();
        assert_eq!(commands.len(), COMMANDS.len());
        let ids: Vec<&str> = commands.iter().map(|command| command.command_id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn a_row_reads_as_its_words_and_type_names() {
        let commands = command_descriptions().unwrap();
        let train = commands
            .iter()
            .find(|command| command.command_id == "train.start")
            .unwrap();
        assert_eq!(train.kind, "job");
        assert_eq!(train.stability, "preview");
        assert_eq!(train.since_version, "0.1.0");
        assert_eq!(train.arguments_type, "TrainStartArgs");
        assert_eq!(train.result_type, "TrainStartResult");
    }

    #[test]
    fn a_row_without_types_is_an_error_naming_the_type_table() {
        let row = CommandRow {
            command_id: "system.doctor",
            ..COMMANDS[0].clone()
        };
        let error = command_description(&row).unwrap_err();
        assert_eq!(error.path, Path::new(COMMAND_TYPES_FILE));
        assert!(error.problem.contains("`system.doctor`"), "{error}");
    }

    #[test]
    fn kinds_and_stabilities_are_the_contract_words() {
        assert_eq!(kind_word(CommandKind::Stream), "stream");
        assert_eq!(kind_word(CommandKind::Sync), "sync");
        assert_eq!(stability_word(Stability::Stable), "stable");
        assert_eq!(stability_word(Stability::Deprecated), "deprecated");
    }
}
