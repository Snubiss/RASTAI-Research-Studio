//! `<cache>/commands-<version>.json` (contracts 8.1, layout 9.1): the last `ListCommands` answer, so `rastai help`
//! can list the verbs while no daemon runs. It is only a help cache: verbs always ask the running daemon, whose
//! registry changes as plugins load. A cache that is missing or unreadable is the same as an empty one.

use std::fs;
use std::path::Path;

use rastai_core::paths::{DataPaths, command_help_cache_path};
use rastai_proto::commands::v1::{CommandDescriptor, CommandKind};
use serde_json::{Map, Value, json};

const FORMAT_VERSION: u64 = 1;
const FORMAT_VERSION_KEY: &str = "format_version";
const COMMANDS_KEY: &str = "commands";
/// Written beside the cache and renamed over it, so a reader never sees half a file.
const TEMPORARY_SUFFIX: &str = ".tmp";

// The descriptor fields help reads; the rest of a descriptor isn't kept.
const COMMAND_ID_KEY: &str = "command_id";
const TITLE_KEY: &str = "title";
const SUMMARY_KEY: &str = "summary";
const SECTION_KEY: &str = "section";
const KIND_KEY: &str = "kind";
const PLUGIN_ID_KEY: &str = "plugin_id";
const ARGUMENTS_SCHEMA_KEY: &str = "arguments_schema_json";

/// Saves the commands of a daemon of `daemon_version`. A cache that can't be written is skipped: help falls back to
/// the mode words.
pub fn save_command_cache(paths: &DataPaths, daemon_version: &str, commands: &[CommandDescriptor]) {
    let cache_path = command_help_cache_path(paths, daemon_version);
    let listed: Vec<Value> = commands.iter().map(command_to_json).collect();
    let text = json!({ FORMAT_VERSION_KEY: FORMAT_VERSION, COMMANDS_KEY: listed }).to_string();
    let _ = write_atomically(&cache_path, &text);
}

/// The cached commands of a daemon of `version`, or none.
pub fn load_command_cache(paths: &DataPaths, version: &str) -> Vec<CommandDescriptor> {
    let Ok(text) = fs::read_to_string(command_help_cache_path(paths, version)) else {
        return Vec::new();
    };
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    if value[FORMAT_VERSION_KEY].as_u64() != Some(FORMAT_VERSION) {
        return Vec::new();
    }
    let listed = value[COMMANDS_KEY].as_array().cloned().unwrap_or_default();
    listed.iter().filter_map(command_from_json).collect()
}

fn command_to_json(command: &CommandDescriptor) -> Value {
    let mut object = Map::new();
    object.insert(COMMAND_ID_KEY.to_string(), json!(command.command_id));
    object.insert(TITLE_KEY.to_string(), json!(command.title));
    object.insert(SUMMARY_KEY.to_string(), json!(command.summary));
    object.insert(SECTION_KEY.to_string(), json!(command.section));
    object.insert(KIND_KEY.to_string(), json!(command.kind().as_str_name()));
    object.insert(PLUGIN_ID_KEY.to_string(), json!(command.plugin_id));
    object.insert(ARGUMENTS_SCHEMA_KEY.to_string(), json!(command.arguments_schema_json));
    Value::Object(object)
}

fn command_from_json(value: &Value) -> Option<CommandDescriptor> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    let command_id = value[COMMAND_ID_KEY].as_str()?.to_string();
    let kind = CommandKind::from_str_name(value[KIND_KEY].as_str().unwrap_or_default()).unwrap_or_default();
    Some(CommandDescriptor {
        command_id,
        title: text(TITLE_KEY),
        summary: text(SUMMARY_KEY),
        section: text(SECTION_KEY),
        kind: kind as i32,
        plugin_id: text(PLUGIN_ID_KEY),
        arguments_schema_json: text(ARGUMENTS_SCHEMA_KEY),
        ..CommandDescriptor::default()
    })
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(TEMPORARY_SUFFIX);
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn saved_commands_load_back() {
        let root = std::env::temp_dir().join(format!("rcc-{}", std::process::id()));
        let paths = DataPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            state: root.join("state"),
            runtime: root.join("runtime"),
        };
        let command = CommandDescriptor {
            command_id: "job.cancel".to_string(),
            title: "Cancel a job".to_string(),
            kind: CommandKind::Sync as i32,
            arguments_schema_json: "{\"type\":\"object\"}".to_string(),
            ..CommandDescriptor::default()
        };
        save_command_cache(&paths, "0.1.0", std::slice::from_ref(&command));
        assert_eq!(load_command_cache(&paths, "0.1.0"), vec![command]);
        assert!(load_command_cache(&paths, "9.9.9").is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
