//! `rastai help [words]` and `--help` (layout: rust-control-plane.md 9.1): help text from the command descriptors
//! and their argument schemas, plus one usage line per fixed mode. Help never starts a daemon: it asks a running
//! one for its commands, and otherwise reads this version's command cache, so the verbs are listed while the daemon
//! is down. With `--json` each answer is one JSON line, like any other result.

use rastai_core::commands::naming::{GLOBAL_FLAGS, cli_words};
use rastai_core::commands::registry::CommandRegistry;
use rastai_core::paths::{DataPaths, daemon_socket_path};
use rastai_core::versions::APP_VERSION;
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::{Value, json};

use crate::command_cache::load_command_cache;
use crate::daemon_client::{DaemonClient, list_commands, read_health};
use crate::error::CliError;
use crate::modes::fixed_words_of;
use crate::output::{OutputSettings, print_line, print_result, print_text};
use crate::verb_arguments::{ArgumentField, argument_fields, kind_words};
use crate::verbs::find_verb;

const PROGRAM_NAME: &str = "rastai";
/// Each fixed mode's usage line (contracts 3.1 and 15.1).
const MODE_USAGE: &[(&str, &str)] = &[
    (
        "open",
        "rastai open [--print-url] [--replace-older-daemon]   start the daemon if needed and open the UI",
    ),
    (
        "serve",
        "rastai serve [--headless] [--port N] [--set key=value]   run the daemon",
    ),
    ("node serve", "rastai node serve   run a standalone node (M9)"),
    (
        "node init",
        "rastai node init --daemon-key <key>   create this node's identity (M9)",
    ),
    (
        "node check-update",
        "rastai node check-update   the node's update check (L2)",
    ),
    (
        "tunnel",
        "rastai tunnel <user@host>   open the UI of a headless box over SSH (M9)",
    ),
    ("mcp", "rastai mcp   the MCP stdio bridge, with RASTAI_TOKEN"),
    (
        "validate",
        "rastai validate <file>...   check documents, no daemon needed",
    ),
    ("version", "rastai version   print this CLI's version"),
    (
        "storage rebuild-index",
        "rastai storage rebuild-index [--verify-blobs]   rebuild rastai.db offline, with the daemon stopped",
    ),
    ("help", "rastai help [words]   this help"),
];
const COMMANDS_HEADING: &str = "Commands:";
const MODES_HEADING: &str = "Modes:";
const FLAGS_HEADING: &str = "Flags:";
const GLOBAL_FLAGS_HEADING: &str = "Global flags:";
const REQUIRED_NOTE: &str = "(required)";
const INDENT: &str = "  ";
/// The one control character a descriptor's text keeps: a summary runs over several lines.
const LINE_BREAK: char = '\n';

/// Prints help for the words: everything when there are none, a mode's usage, or a command's flags.
pub async fn run_help(paths: &DataPaths, words: &[String], settings: OutputSettings) -> Result<(), CliError> {
    if let Some(mode_words) = fixed_words_of(words)
        && words.len() == mode_words.split(' ').count()
    {
        let usage = mode_usage(mode_words);
        print_line(settings, &usage, &json!({ "mode": mode_words, "usage": usage }));
        return Ok(());
    }
    let registry = CommandRegistry {
        commands: known_commands(paths).await,
    };
    if words.is_empty() && settings.json {
        print_result(settings, &overview_json(&registry));
        return Ok(());
    }
    if words.is_empty() {
        print_text(settings, &overview(&registry));
        return Ok(());
    }
    let command = find_verb(&registry, words)?;
    if settings.json {
        print_result(settings, &command_help_json(command));
    } else {
        print_text(settings, &command_help(command));
    }
    Ok(())
}

/// The running daemon's commands, else the cache of this version's: the commands of the daemon a verb would start
/// (verbs.rs keeps the cache only from a daemon of this version).
async fn known_commands(paths: &DataPaths) -> Vec<CommandDescriptor> {
    let socket_path = daemon_socket_path(paths);
    if let Ok(health) = read_health(&socket_path).await {
        let client = DaemonClient {
            socket_path,
            daemon_version: health.version,
            warnings: Vec::new(),
        };
        if let Ok(commands) = list_commands(&client).await {
            return commands;
        }
    }
    load_command_cache(paths, APP_VERSION)
}

fn mode_usage(mode_words: &str) -> String {
    let found = MODE_USAGE.iter().find(|(words, _)| *words == mode_words);
    match found {
        Some((_, usage)) => usage.to_string(),
        None => format!("{PROGRAM_NAME} {mode_words}"),
    }
}

/// Every mode, then every command with its title.
fn overview(registry: &CommandRegistry) -> String {
    let mut lines = vec![MODES_HEADING.to_string()];
    for (_, usage) in MODE_USAGE {
        lines.push(format!("{INDENT}{usage}"));
    }
    if !registry.commands.is_empty() {
        lines.push(String::new());
        lines.push(COMMANDS_HEADING.to_string());
    }
    for command in &registry.commands {
        let verb = printable_text(&cli_words(&command.command_id).join(" "));
        lines.push(format!(
            "{INDENT}{PROGRAM_NAME} {verb}   {}",
            printable_text(&command.title)
        ));
    }
    lines.push(String::new());
    lines.push(format!("{GLOBAL_FLAGS_HEADING} {}", GLOBAL_FLAGS.join(" ")));
    lines.join("\n")
}

/// The overview as one JSON object: the modes with their usage, the commands with their verbs, the global flags.
fn overview_json(registry: &CommandRegistry) -> Value {
    let modes: Vec<Value> = MODE_USAGE
        .iter()
        .map(|(words, usage)| json!({ "mode": words, "usage": usage }))
        .collect();
    let commands: Vec<Value> = registry
        .commands
        .iter()
        .map(|command| {
            json!({
                "command_id": command.command_id,
                "verb": cli_words(&command.command_id).join(" "),
                "title": command.title,
            })
        })
        .collect();
    json!({ "modes": modes, "commands": commands, "global_flags": GLOBAL_FLAGS })
}

/// One command: its usage line with positionals, its title and summary, then its flags.
fn command_help(command: &CommandDescriptor) -> String {
    let fields = command_fields(command);
    let mut lines = vec![
        printable_text(&usage_line(command, &fields)),
        String::new(),
        printable_text(&command.title),
    ];
    if !command.summary.is_empty() {
        lines.push(printable_text(&command.summary));
    }
    if !fields.is_empty() {
        lines.push(String::new());
        lines.push(FLAGS_HEADING.to_string());
    }
    for field in &fields {
        lines.push(flag_line(field));
    }
    lines.push(String::new());
    lines.push(format!("{GLOBAL_FLAGS_HEADING} {}", GLOBAL_FLAGS.join(" ")));
    lines.join("\n")
}

fn command_help_json(command: &CommandDescriptor) -> Value {
    let flags: Vec<Value> = command_fields(command)
        .iter()
        .map(|field| {
            json!({
                "flag": field.flag,
                "takes": kind_words(&field.kind),
                "required": field.required,
                "position": field.cli_position,
                "description": field.description,
            })
        })
        .collect();
    json!({
        "command_id": command.command_id,
        "title": command.title,
        "summary": command.summary,
        "kind": command.kind().as_str_name(),
        "flags": flags,
    })
}

fn command_fields(command: &CommandDescriptor) -> Vec<ArgumentField> {
    let schema: Value = serde_json::from_str(&command.arguments_schema_json).unwrap_or(Value::Null);
    argument_fields(&schema)
}

/// `rastai job cancel <job_id> [flags]`: the positional fields in order.
fn usage_line(command: &CommandDescriptor, fields: &[ArgumentField]) -> String {
    let mut positional: Vec<&ArgumentField> = fields.iter().filter(|field| field.cli_position.is_some()).collect();
    positional.sort_by_key(|field| field.cli_position);
    let mut line = format!("{PROGRAM_NAME} {}", cli_words(&command.command_id).join(" "));
    for field in positional {
        line.push_str(&format!(" <{}>", field.name));
    }
    if !fields.is_empty() {
        line.push_str(" [flags]");
    }
    line
}

fn flag_line(field: &ArgumentField) -> String {
    let mut line = format!(
        "{INDENT}{} <{}>   {}",
        field.flag,
        kind_words(&field.kind),
        field.description
    );
    if field.required {
        line.push(' ');
        line.push_str(REQUIRED_NOTE);
    }
    printable_text(&line)
}

/// Descriptor text as the terminal may show it. A plugin's title or summary could hold an escape sequence (ESC, or
/// the C1 range) that rewrites the screen, or a carriage return that overwrites the line, so every control character
/// goes except the line break. Nothing else in the text changes.
fn printable_text(text: &str) -> String {
    text.chars()
        .filter(|character| *character == LINE_BREAK || !character.is_control())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::commands::naming::CLI_MODE_WORDS;

    #[test]
    fn every_mode_has_a_usage_line() {
        for mode_words in CLI_MODE_WORDS {
            let usage = mode_usage(mode_words);
            assert!(usage.starts_with(&format!("{PROGRAM_NAME} {mode_words}")), "{usage}");
        }
    }

    #[test]
    fn the_json_overview_lists_modes_commands_and_global_flags() {
        let registry = CommandRegistry {
            commands: vec![CommandDescriptor {
                command_id: "job.cancel".to_string(),
                title: "Cancel a job".to_string(),
                ..CommandDescriptor::default()
            }],
        };
        let overview = overview_json(&registry);
        assert_eq!(overview["modes"].as_array().unwrap().len(), MODE_USAGE.len());
        assert_eq!(
            overview["commands"][0],
            json!({"command_id": "job.cancel", "verb": "job cancel", "title": "Cancel a job"})
        );
        assert_eq!(overview["global_flags"][0], "--json");
    }

    #[test]
    fn a_plugin_title_cannot_write_control_characters_to_the_terminal() {
        let command = CommandDescriptor {
            command_id: "plugin.demo.run".to_string(),
            title: "Run\u{1b}[2J now".to_string(),
            summary: "First line\rover\nSecond \u{9b}line".to_string(),
            ..CommandDescriptor::default()
        };
        let text = command_help(&command);
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(!text.contains('\r'), "{text:?}");
        assert!(!text.contains('\u{9b}'), "{text:?}");
        assert!(text.contains("Run[2J now"), "{text:?}");
        assert!(text.contains("First lineover\nSecond line"), "{text:?}");
    }

    #[test]
    fn a_command_shows_its_positionals_and_flags() {
        let schema = json!({
            "properties": {"job_id": {"type": "string", "x-rastai-cli-position": 0, "description": "The job."}},
            "required": ["job_id"]
        });
        let command = CommandDescriptor {
            command_id: "job.cancel".to_string(),
            title: "Cancel a job".to_string(),
            arguments_schema_json: schema.to_string(),
            ..CommandDescriptor::default()
        };
        let text = command_help(&command);
        assert!(text.starts_with("rastai job cancel <job_id> [flags]"), "{text}");
        assert!(text.contains("--job-id <text>   The job. (required)"), "{text}");
    }
}
