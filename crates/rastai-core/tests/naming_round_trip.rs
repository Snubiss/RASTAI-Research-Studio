//! The naming rules of the generated surfaces (layout rust-control-plane.md 5.5, 10.3) on every command id there
//! is: the core table and every id contracts 6.4 registers, core and plugin alike. Each id must have the contract
//! form, and survive `cli_words` -> `command_id_from_cli_words` and `mcp_tool_name` -> `command_id_from_mcp_tool_name`;
//! no two ids may share a JavaScript wrapper; no CLI verb may shadow a fixed CLI word. Then the CLI and MCP columns
//! of `fixtures/command-golden/copy-as.json` must follow the same rules.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rastai_core::commands::naming::{
    ALIASES, CLI_MODE_WORDS, check_command_id, cli_words, command_id_from_cli_words, command_id_from_mcp_tool_name,
    flag_name, javascript_wrapper_name, mcp_tool_name,
};
use rastai_core::commands::registry::{CommandRegistry, load_core_commands};
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::Value;

const CONTRACTS: &str = "docs/architecture/contracts.md";
const CATALOG_HEADING: &str = "### 6.4 Initial command catalog";
const NEXT_HEADING: &str = "### 6.5";
const COPY_AS_GOLDEN: &str = "fixtures/command-golden/copy-as.json";
/// The first-party plugins whose commands contracts 6.4 lists; every other id is a core command.
const PLUGIN_AREAS: &[&str] = &["psychology", "eval"];
/// 6.4 lists about 200 ids; far fewer means the table reader lost its place, not that commands went away.
const MIN_CONTRACT_IDS: usize = 150;
const CLI_PROGRAM: &str = "rastai";

fn repository_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

/// The command ids in the first column of the contracts 6.4 tables (`model.load`, and both ids of a
/// `system.update.check` / `system.update.apply` cell).
fn contract_command_ids() -> Vec<String> {
    let text = fs::read_to_string(repository_file(CONTRACTS)).unwrap();
    let start = text.find(CATALOG_HEADING).expect("contracts 6.4 heading");
    let end = start + text[start..].find(NEXT_HEADING).expect("contracts 6.5 heading");
    let mut ids = Vec::new();
    for line in text[start..end].lines() {
        let Some(first_cell) = line.strip_prefix("| `").and_then(|rest| rest.split('|').next()) else {
            continue;
        };
        let cell = format!("`{first_cell}");
        for (index, piece) in cell.split('`').enumerate() {
            // The odd pieces are inside backticks.
            let is_command_id = piece.contains('.') && !piece.contains(' ');
            if index % 2 == 1 && is_command_id {
                ids.push(piece.to_string());
            }
        }
    }
    ids
}

/// The core table's ids and every id contracts 6.4 registers, each once.
fn every_command_id() -> Vec<String> {
    let mut ids: BTreeSet<String> = contract_command_ids().into_iter().collect();
    for command in load_core_commands().commands {
        ids.insert(command.command_id);
    }
    ids.into_iter().collect()
}

fn plugin_id_of(command_id: &str) -> &str {
    let area = command_id.split('.').next().unwrap();
    if PLUGIN_AREAS.contains(&area) { area } else { "" }
}

fn registry_of(command_ids: &[String]) -> CommandRegistry {
    let commands = command_ids
        .iter()
        .map(|command_id| CommandDescriptor {
            command_id: command_id.clone(),
            plugin_id: plugin_id_of(command_id).to_string(),
            ..CommandDescriptor::default()
        })
        .collect();
    CommandRegistry { commands }
}

#[test]
fn the_contract_tables_are_read() {
    let ids = contract_command_ids();
    assert!(
        ids.len() >= MIN_CONTRACT_IDS,
        "only {} ids read from {CONTRACTS}",
        ids.len()
    );
    for expected in [
        "system.status",
        "system.update.apply",
        "eval.suite.add_conversation",
        "dataset.from_conversations",
    ] {
        assert!(ids.iter().any(|id| id == expected), "{expected} missing");
    }
}

#[test]
fn every_id_has_the_contract_form_and_area() {
    for command_id in every_command_id() {
        let plugin_id = plugin_id_of(&command_id);
        if let Err(refusal) = check_command_id(&command_id, plugin_id) {
            panic!("{command_id} (plugin {plugin_id:?}): {}", refusal.code);
        }
    }
}

#[test]
fn every_id_survives_the_cli_words() {
    let ids = every_command_id();
    let registry = registry_of(&ids);
    for command_id in &ids {
        let words = cli_words(command_id);
        let found = command_id_from_cli_words(&registry, &words);
        assert_eq!(found, Some((command_id.clone(), words.len())), "{command_id}");
        // Flags after the verb are left for the argument parser.
        let mut with_flag = words.clone();
        with_flag.push(flag_name("fail_fast"));
        assert_eq!(command_id_from_cli_words(&registry, &with_flag), found, "{command_id}");
    }
}

#[test]
fn every_id_survives_the_mcp_tool_name() {
    for command_id in every_command_id() {
        let tool_name = mcp_tool_name(&command_id);
        assert!(!tool_name.contains('.'), "{tool_name}");
        assert_eq!(command_id_from_mcp_tool_name(&tool_name), command_id);
    }
}

#[test]
fn every_id_has_its_own_javascript_wrapper() {
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    for command_id in every_command_id() {
        let name = javascript_wrapper_name(&command_id);
        assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric()), "{name}");
        if let Some(other) = owners.insert(name.clone(), command_id.clone()) {
            panic!("{command_id} and {other} share the wrapper {name}");
        }
    }
}

#[test]
fn no_verb_shadows_a_fixed_cli_word() {
    let fixed: BTreeSet<String> = CLI_MODE_WORDS
        .iter()
        .map(|words| words.to_string())
        .chain(ALIASES.iter().map(|(alias, _)| alias.to_string()))
        .collect();
    for command_id in every_command_id() {
        let verb = cli_words(&command_id).join(" ");
        assert!(!fixed.contains(&verb), "{command_id} is the fixed CLI word {verb:?}");
    }
}

#[test]
fn aliases_name_registered_commands() {
    let ids = every_command_id();
    let registry = registry_of(&ids);
    for (alias, command_id) in ALIASES {
        assert!(ids.iter().any(|id| id == command_id), "{alias} -> {command_id}");
        let words = vec![alias.to_string(), flag_name("json")];
        assert_eq!(
            command_id_from_cli_words(&registry, &words),
            Some((command_id.to_string(), 1))
        );
    }
    // Without its command in the registry, an alias names nothing.
    let empty = CommandRegistry::default();
    assert_eq!(command_id_from_cli_words(&empty, &["doctor".to_string()]), None);
}

#[test]
fn only_the_exact_spelling_finds_a_command() {
    let registry = registry_of(&["eval.suite.add_conversation".to_string(), "eval.test".to_string()]);
    let words = |text: &str| -> Vec<String> { text.split(' ').map(str::to_string).collect() };
    assert_eq!(
        command_id_from_cli_words(&registry, &words("eval suite add_conversation")),
        None
    );
    assert_eq!(command_id_from_cli_words(&registry, &words("eval suite")), None);
    assert_eq!(command_id_from_cli_words(&registry, &words("eval")), None);
    assert_eq!(command_id_from_cli_words(&registry, &[]), None);
    assert_eq!(
        command_id_from_cli_words(&registry, &words("eval test --suite x")),
        Some(("eval.test".to_string(), 2))
    );
}

/// The `(flag, value)` pairs a copy-as CLI line should end with: `flag_name` of each field, the value as text.
fn expected_flags(arguments: &Value) -> Vec<(String, String)> {
    let mut flags = Vec::new();
    for (field, value) in arguments.as_object().unwrap() {
        let text = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        flags.push((flag_name(field), text));
    }
    flags.sort();
    flags
}

/// The `(flag, value)` pairs after the verb of a CLI line, sorted.
fn written_flags(flag_words: &[String]) -> Vec<(String, String)> {
    assert_eq!(flag_words.len() % 2, 0, "every flag has one value: {flag_words:?}");
    let mut flags: Vec<(String, String)> = flag_words
        .chunks(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    flags.sort();
    flags
}

#[test]
fn copy_as_cli_and_mcp_columns_follow_the_rules() {
    let golden: Value = serde_json::from_str(&fs::read_to_string(repository_file(COPY_AS_GOLDEN)).unwrap()).unwrap();
    let examples = golden["examples"].as_array().unwrap();
    assert!(!examples.is_empty());
    let ids: Vec<String> = examples
        .iter()
        .map(|example| example["command_id"].as_str().unwrap().to_string())
        .collect();
    let registry = registry_of(&ids);
    for (example, command_id) in examples.iter().zip(&ids) {
        check_command_id(command_id, plugin_id_of(command_id)).unwrap();
        assert_eq!(example["mcp"]["name"], mcp_tool_name(command_id), "{command_id}");
        assert_eq!(example["mcp"]["arguments"], example["arguments"], "{command_id}");

        let words: Vec<String> = example["cli"]
            .as_str()
            .unwrap()
            .split(' ')
            .map(str::to_string)
            .collect();
        assert_eq!(words[0], CLI_PROGRAM);
        let (found, word_count) = command_id_from_cli_words(&registry, &words[1..]).unwrap();
        assert_eq!(&found, command_id);
        assert_eq!(words[1..=word_count], cli_words(command_id));
        let flags = written_flags(&words[1 + word_count..]);
        assert_eq!(flags, expected_flags(&example["arguments"]), "{command_id}");
    }
}
