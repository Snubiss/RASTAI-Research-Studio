//! `run_verb` (layout: rust-control-plane.md 9.3): words to a registry command, flags to arguments JSON, `Execute`,
//! then the result printed or the job followed until it ends. No verb is written here: every one comes from the
//! daemon's `ListCommands`, so a new command or an enabled plugin's is a verb at once.
//!
//! (M3) A `stream` command's result is printed like a `sync` one's: it is the session id. Printing the session's
//! ephemeral events as they arrive (layout 9.3 step 6) lands with the first core stream command, `chat.send`,
//! whose events (`chat.token`) and end are defined there.

use rastai_core::commands::naming::{cli_words, command_id_from_cli_words};
use rastai_core::commands::registry::{CommandRegistry, find_command};
use rastai_core::error::CoreError;
use rastai_core::paths::DataPaths;
use rastai_core::random::random_bytes;
use rastai_core::versions::APP_VERSION;
use rastai_core::wire::diagnostic_from_proto;
use rastai_diag::diagnostic::{Diagnostic, diagnostic};
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind, ExecuteRequest, ExecuteResponse};
use rastai_proto::common::v1::JobState;
use serde_json::{Value, json};

use crate::arguments::ParsedArguments;
use crate::command_cache::save_command_cache;
use crate::daemon_client::{DaemonClient, execute, list_commands};
use crate::error::CliError;
use crate::exit_codes::{EXIT_OK, exit_code_for_failure, exit_code_for_job};
use crate::follow::{CHUNK_TIMEOUT_MS, EventStream, follow_job, open_job_events};
use crate::output::{OutputSettings, output_settings, print_line, print_result};
use crate::verb_arguments::build_arguments_json;

const CODE_UNKNOWN_COMMAND: &str = "RRS-C0001";
const CODE_RANDOM_SOURCE_UNREADABLE: &str = "RRS-C0502";
/// How many close verbs an unknown one suggests (layout 9.3 step 3).
const SUGGESTION_COUNT: usize = 3;
/// A command id has at most three segments, so at most three words name a verb.
const MAX_VERB_WORDS: usize = 3;
const PROGRAM_NAME: &str = "rastai";
/// Random bytes in an idempotency key, written as hex: enough that two invocations never share one.
const IDEMPOTENCY_KEY_BYTES: usize = 16;
const JOB_ID_KEY: &str = "job_id";

/// How a verb ended (layout 9.2).
#[derive(Debug, Clone, PartialEq)]
pub enum VerbOutcome {
    /// A `sync` command's answer, a `stream` command's session id, or any command the daemon refused or that
    /// failed (`ok: false`).
    Finished(ExecuteResponse),
    /// A followed job reached a terminal state. `from_state` is the state it left, which tells a job that failed
    /// before it ran from one that failed while running.
    JobEnded {
        job_id: String,
        final_state: JobState,
        from_state: JobState,
        diagnostics: Vec<Diagnostic>,
    },
    /// `--detach`: the job runs on, unfollowed.
    Detached { job_id: String },
}

/// Runs the registry command the words name, against a running daemon.
pub async fn run_verb(
    client: &DaemonClient,
    paths: &DataPaths,
    words: &[String],
    arguments: &ParsedArguments,
) -> Result<VerbOutcome, CliError> {
    let settings = output_settings(&arguments.global);
    let commands = list_commands(client).await?;
    // The cache is this version's commands (contracts 8.1), which help reads while no daemon runs; a daemon of
    // another version has other commands, so its list isn't kept.
    if client.daemon_version == APP_VERSION {
        save_command_cache(paths, APP_VERSION, &commands);
    }
    let registry = CommandRegistry { commands };
    let command = find_verb(&registry, words)?;
    let request = execute_request(command, arguments)?;
    // Subscribed before Execute, so no event of the job can fall between its start and the subscription.
    let events = if follows_job(command, arguments) {
        Some(open_job_events(client, CHUNK_TIMEOUT_MS).await?)
    } else {
        None
    };
    let response = execute(client, &request).await?;
    finish_verb(response, events, settings).await
}

/// The command the leading words name. An unknown verb is `RRS-C0001`, with the closest verbs as its fix.
pub fn find_verb<'a>(registry: &'a CommandRegistry, words: &[String]) -> Result<&'a CommandDescriptor, CliError> {
    let found =
        command_id_from_cli_words(registry, words).and_then(|(command_id, _)| find_command(registry, &command_id));
    match found {
        Some(command) => Ok(command),
        None => Err(unknown_verb(registry, words)),
    }
}

fn execute_request(command: &CommandDescriptor, arguments: &ParsedArguments) -> Result<ExecuteRequest, CliError> {
    // `--node` and `--set` were refused when the mode was chosen (modes.rs), before any daemon was started.
    let global = &arguments.global;
    let schema: Value = serde_json::from_str(&command.arguments_schema_json).unwrap_or(Value::Null);
    let arguments_json = build_arguments_json(command, &schema, arguments).map_err(CliError::Usage)?;
    let idempotency_key = match &global.idempotency_key {
        Some(key) => key.clone(),
        None => new_idempotency_key()?,
    };
    Ok(ExecuteRequest {
        command_id: command.command_id.clone(),
        arguments_json,
        project_id: global.project.clone().unwrap_or_default(),
        idempotency_key,
        dry_run: global.dry_run,
        acting_plugin_id: String::new(),
    })
}

/// A job is followed unless `--detach`; a dry run starts nothing to follow. A stream isn't followed yet (M3).
fn follows_job(command: &CommandDescriptor, arguments: &ParsedArguments) -> bool {
    command.kind() == CommandKind::Job && !arguments.global.detach && !arguments.global.dry_run
}

async fn finish_verb(
    response: ExecuteResponse,
    events: Option<EventStream>,
    settings: OutputSettings,
) -> Result<VerbOutcome, CliError> {
    if !response.ok || response.job_id.is_empty() {
        print_command_result(&response, settings);
        return Ok(VerbOutcome::Finished(response));
    }
    let job_id = response.job_id.clone();
    let job_line = json!({ JOB_ID_KEY: job_id });
    print_line(settings, &job_id, &job_line);
    let Some(events) = events else {
        return Ok(VerbOutcome::Detached { job_id });
    };
    follow_job(events, &job_id, settings).await
}

fn print_command_result(response: &ExecuteResponse, settings: OutputSettings) {
    if !response.ok || response.result_json.is_empty() {
        return;
    }
    let result: Value =
        serde_json::from_str(&response.result_json).unwrap_or(Value::String(response.result_json.clone()));
    print_result(settings, &result);
}

/// The exit code of a verb's outcome (layout 9.5).
pub fn outcome_exit_code(outcome: &VerbOutcome) -> u8 {
    match outcome {
        VerbOutcome::Finished(response) if response.ok => EXIT_OK,
        VerbOutcome::Finished(response) => exit_code_for_failure(&response_diagnostics(response)),
        VerbOutcome::JobEnded {
            final_state,
            from_state,
            ..
        } => exit_code_for_job(*final_state, *from_state),
        VerbOutcome::Detached { .. } => EXIT_OK,
    }
}

/// The diagnostics a verb's outcome reports at its end.
pub fn outcome_diagnostics(outcome: &VerbOutcome) -> Vec<Diagnostic> {
    match outcome {
        VerbOutcome::Finished(response) => response_diagnostics(response),
        VerbOutcome::JobEnded { diagnostics, .. } => diagnostics.clone(),
        VerbOutcome::Detached { .. } => Vec::new(),
    }
}

fn response_diagnostics(response: &ExecuteResponse) -> Vec<Diagnostic> {
    response.diagnostics.iter().map(diagnostic_from_proto).collect()
}

/// `RRS-C0001` for the words as typed, its fix naming the closest verbs by edit distance.
fn unknown_verb(registry: &CommandRegistry, words: &[String]) -> CliError {
    let typed_words = &words[..words.len().min(MAX_VERB_WORDS)];
    let typed = typed_words.join(" ");
    let mut refusal = diagnostic(CODE_UNKNOWN_COMMAND, &[("command_id", &typed)]);
    let suggestions = closest_verbs(registry, &typed);
    if !suggestions.is_empty() {
        let shown: Vec<String> = suggestions
            .iter()
            .map(|verb| format!("{PROGRAM_NAME} {verb}"))
            .collect();
        refusal.fix_text = format!("{} ({})", refusal.fix_text, shown.join(", "));
    }
    CliError::Usage(vec![refusal])
}

/// The registered verbs closest to the typed words, closest first, ties in registry order.
pub fn closest_verbs(registry: &CommandRegistry, typed: &str) -> Vec<String> {
    let mut scored: Vec<(usize, String)> = Vec::new();
    for command in &registry.commands {
        let verb = cli_words(&command.command_id).join(" ");
        scored.push((edit_distance(typed, &verb), verb));
    }
    // A stable sort keeps registry order among equal distances.
    scored.sort_by_key(|(distance, _)| *distance);
    scored
        .into_iter()
        .take(SUGGESTION_COUNT)
        .map(|(_, verb)| verb)
        .collect()
}

/// The Levenshtein distance: the fewest single-character insertions, deletions and substitutions from one text
/// to the other.
pub fn edit_distance(from: &str, to: &str) -> usize {
    let to_characters: Vec<char> = to.chars().collect();
    let mut previous_row: Vec<usize> = (0..=to_characters.len()).collect();
    for (row, from_character) in from.chars().enumerate() {
        let mut current_row = vec![row + 1];
        for (column, to_character) in to_characters.iter().enumerate() {
            let substitution = previous_row[column] + usize::from(from_character != *to_character);
            let deletion = previous_row[column + 1] + 1;
            let insertion = current_row[column] + 1;
            current_row.push(substitution.min(deletion).min(insertion));
        }
        previous_row = current_row;
    }
    previous_row[to_characters.len()]
}

fn new_idempotency_key() -> Result<String, CliError> {
    let bytes = random_bytes(IDEMPOTENCY_KEY_BYTES).map_err(|error| {
        let reason = match error {
            CoreError::RandomUnavailable { reason } => reason,
            other => format!("{other:?}"),
        };
        CliError::Failed(vec![diagnostic(CODE_RANDOM_SOURCE_UNREADABLE, &[("reason", &reason)])])
    })?;
    let mut key = String::with_capacity(IDEMPOTENCY_KEY_BYTES * 2);
    for byte in bytes {
        key.push_str(&format!("{byte:02x}"));
    }
    Ok(key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn registry_of(ids: &[&str]) -> CommandRegistry {
        let commands = ids
            .iter()
            .map(|id| CommandDescriptor {
                command_id: id.to_string(),
                ..CommandDescriptor::default()
            })
            .collect();
        CommandRegistry { commands }
    }

    #[test]
    fn edit_distance_counts_single_changes() {
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("system status", "system status"), 0);
    }

    #[test]
    fn an_unknown_verb_suggests_the_closest_three() {
        let registry = registry_of(&["system.status", "system.shutdown", "job.list", "token.list"]);
        // "token list" is 9 edits from "system statu", "job list" 10.
        assert_eq!(
            closest_verbs(&registry, "system statu"),
            ["system status", "system shutdown", "token list"]
        );
        let words: Vec<String> = ["system", "statu"].iter().map(|word| word.to_string()).collect();
        let refusal = find_verb(&registry, &words).unwrap_err();
        assert_eq!(refusal.exit_code(), 2);
        assert_eq!(refusal.diagnostics()[0].code, "RRS-C0001");
        assert!(refusal.diagnostics()[0].fix_text.contains("rastai system status"));
    }

    #[test]
    fn idempotency_keys_are_new_each_time() {
        let first = new_idempotency_key().unwrap();
        assert_eq!(first.len(), 32);
        assert_ne!(first, new_idempotency_key().unwrap());
    }
}
