//! The naming rules of the generated command surfaces (layout rust-control-plane.md 10.3, contracts 6.1), one
//! implementation each: CLI words and flags, MCP tool names, typed JavaScript wrapper names, and the check that a
//! command id has the right form and sits in the right area.
//!
//! A command id is `<area>.<verb>` or `<area>.<noun>.<verb>`. Every segment is snake_case with no leading, trailing
//! or doubled underscore, which is what makes each rule here lossless: `-` in a CLI word and `__` in an MCP tool
//! name can only come from one place in the id. A plugin id is the first segment of its commands, so it follows the
//! same segment rule (`is_plugin_id`), and scopes.rs checks `plugin:<plugin_id>` with that same function.

use rastai_diag::diagnostic::{Diagnostic, diagnostic};

use crate::commands::registry::{CommandRegistry, find_command};

const ID_SEPARATOR: char = '.';
const SEGMENT_WORD_SEPARATOR: char = '_';
/// CLI words and flags write `_` as `-` (`add_conversation` -> `add-conversation`).
const CLI_WORD_SEPARATOR: char = '-';
const MCP_SEPARATOR: &str = "__";
const FLAG_PREFIX: &str = "--";
const MIN_SEGMENTS: usize = 2;
const MAX_SEGMENTS: usize = 3;

const CODE_VALUE_NOT_ALLOWED: &str = "RRS-E1004";
const CODE_PLUGIN_COMMAND_OUTSIDE_NAMESPACE: &str = "RRS-P0004";
const EXPECTED_ID_FORM: &str = "<area>.<verb> or <area>.<noun>.<verb>, in lowercase snake_case";
const EXPECTED_PLUGIN_ID_FORM: &str =
    "a plugin id: lowercase letters and digits in words joined by single underscores, starting with a letter";

/// The reserved core areas (contracts 6.1). No plugin may use one as its id, and every core command sits in one.
/// `view` holds UI-only actions and `core` is the pseudo plugin id of core sections and panels.
const CORE_AREAS: &[&str] = &[
    "system",
    "settings",
    "token",
    "secret",
    "storage",
    "archive",
    "artifact",
    "project",
    "file",
    "document",
    "workspace",
    "command",
    "job",
    "hf",
    "model",
    "chat",
    "conversation",
    "adapter",
    "gateway",
    "train",
    "teach",
    "rlhf",
    "sweep",
    "recipe",
    "run",
    "checkpoint",
    "design",
    "harness",
    "gym",
    "sandbox",
    "plugin",
    "node",
    "transfer",
    "dataset",
    "converter",
    "tutorial",
    "view",
    "core",
];

/// Short CLI words for commands people type often: the word, then the command id it runs (contracts 15.1).
pub const ALIASES: &[(&str, &str)] = &[("doctor", "system.doctor"), ("rerun", "run.rerun")];

/// The CLI's fixed words (contracts 15.1): process modes and local commands, none of them registry commands.
/// `rastai-cli`'s modes.rs dispatches them, and rastai-names-codegen lists them as CLI verbs. `node check-update`
/// is (L2).
pub const CLI_MODE_WORDS: &[&str] = &[
    "open",
    "serve",
    "node serve",
    "node init",
    "node check-update",
    "tunnel",
    "mcp",
    "validate",
    "version",
    "storage rebuild-index",
    "help",
];

/// The CLI words of a command: its segments, with `_` written as `-`.
/// `eval.suite.add_conversation` -> `["eval", "suite", "add-conversation"]`.
pub fn cli_words(command_id: &str) -> Vec<String> {
    command_id.split(ID_SEPARATOR).map(cli_word).collect()
}

/// The registered command the leading CLI words name, and how many words it used; the rest are its flags.
/// The longest match wins (three words before two), so a command id never hides a longer one. A single alias
/// word (`doctor`) also counts, when the command it stands for is registered. A word must be written exactly as
/// `cli_words` writes it, so every command has one spelling.
pub fn command_id_from_cli_words(registry: &CommandRegistry, words: &[String]) -> Option<(String, usize)> {
    let longest = MAX_SEGMENTS.min(words.len());
    for word_count in (MIN_SEGMENTS..=longest).rev() {
        let candidate = command_id_from_words(&words[..word_count]);
        if find_command(registry, &candidate).is_some() && cli_words(&candidate) == words[..word_count] {
            return Some((candidate, word_count));
        }
    }
    let first_word = words.first()?;
    let (_, command_id) = ALIASES.iter().find(|(alias, _)| alias == first_word)?;
    find_command(registry, command_id)?;
    Some((command_id.to_string(), 1))
}

/// The CLI flag of an argument field: `fail_fast` -> `--fail-fast`.
pub fn flag_name(field_name: &str) -> String {
    format!("{FLAG_PREFIX}{}", cli_word(field_name))
}

/// The MCP tool name of a command: dots as `__` (`eval.suite.edit` -> `eval__suite__edit`), because MCP clients
/// refuse dots in tool names.
pub fn mcp_tool_name(command_id: &str) -> String {
    command_id.replace(ID_SEPARATOR, MCP_SEPARATOR)
}

/// The command id of an MCP tool name, the inverse of `mcp_tool_name`. It doesn't check that the command exists;
/// the caller looks it up in the registry.
pub fn command_id_from_mcp_tool_name(tool_name: &str) -> String {
    tool_name.replace(MCP_SEPARATOR, &ID_SEPARATOR.to_string())
}

/// The typed JavaScript wrapper of a command: camelCase of its segments
/// (`eval.suite.add_conversation` -> `evalSuiteAddConversation`, contracts 6.1). rastai-codegen writes one per
/// command into the generated `commands.js`.
pub fn javascript_wrapper_name(command_id: &str) -> String {
    let mut name = String::new();
    let words = command_id.split([ID_SEPARATOR, SEGMENT_WORD_SEPARATOR]);
    for (index, word) in words.enumerate() {
        if index == 0 {
            name.push_str(word);
        } else {
            name.push_str(&capitalized(word));
        }
    }
    name
}

/// Checks a command id's form (contracts 6.1) and its area. A core command (`plugin_id` empty) must sit in a core
/// area; a plugin's command must start with the plugin's id, and that id can't be a core area (`RRS-P0004`).
/// A malformed command id or plugin id is `RRS-E1004`.
// The layout's shape for every check that refuses with one diagnostic (5.2), as in scopes.rs.
#[allow(clippy::result_large_err)]
pub fn check_command_id(command_id: &str, plugin_id: &str) -> Result<(), Diagnostic> {
    if !plugin_id.is_empty() {
        check_plugin_id(plugin_id)?;
    }
    let segments: Vec<&str> = command_id.split(ID_SEPARATOR).collect();
    let segment_count_ok = (MIN_SEGMENTS..=MAX_SEGMENTS).contains(&segments.len());
    if !segment_count_ok || !segments.iter().all(|segment| is_snake_case_segment(segment)) {
        return Err(diagnostic(
            CODE_VALUE_NOT_ALLOWED,
            &[("found", command_id), ("expected", EXPECTED_ID_FORM)],
        ));
    }
    let area = segments[0];
    if plugin_id.is_empty() {
        return check_core_area(command_id, area);
    }
    if area != plugin_id || is_core_area(plugin_id) {
        return Err(diagnostic(
            CODE_PLUGIN_COMMAND_OUTSIDE_NAMESPACE,
            &[("command_id", command_id), ("plugin_id", plugin_id)],
        ));
    }
    Ok(())
}

/// True for a plugin id as contracts 6.1 defines it: one command-id segment (`acme`, `acme_tools2`), so no
/// trailing or doubled underscore (`acme_`, `acme__tools`). The empty id of core commands is not a plugin id.
pub(crate) fn is_plugin_id(text: &str) -> bool {
    is_snake_case_segment(text)
}

/// Refuses a text that isn't a plugin id (`RRS-E1004`), including the empty id that marks core commands.
#[allow(clippy::result_large_err)]
pub(crate) fn check_plugin_id(plugin_id: &str) -> Result<(), Diagnostic> {
    if is_plugin_id(plugin_id) {
        return Ok(());
    }
    Err(diagnostic(
        CODE_VALUE_NOT_ALLOWED,
        &[("found", plugin_id), ("expected", EXPECTED_PLUGIN_ID_FORM)],
    ))
}

/// True for one of the reserved core areas of contracts 6.1.
fn is_core_area(area: &str) -> bool {
    CORE_AREAS.contains(&area)
}

#[allow(clippy::result_large_err)]
fn check_core_area(command_id: &str, area: &str) -> Result<(), Diagnostic> {
    if is_core_area(area) {
        return Ok(());
    }
    let expected = format!("a core area: {}", CORE_AREAS.join(", "));
    Err(diagnostic(
        CODE_VALUE_NOT_ALLOWED,
        &[("found", command_id), ("expected", &expected)],
    ))
}

/// `[a-z][a-z0-9]*` words joined by single underscores: `add_conversation`, `top_k2`; not `_x`, `x_`, `a__b`.
fn is_snake_case_segment(segment: &str) -> bool {
    let starts_with_letter = segment.bytes().next().is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && segment.split(SEGMENT_WORD_SEPARATOR).all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// One segment or field name as a CLI word: `_` written as `-`.
fn cli_word(segment: &str) -> String {
    segment.replace(SEGMENT_WORD_SEPARATOR, &CLI_WORD_SEPARATOR.to_string())
}

/// The command id CLI words would spell, before checking it is registered.
fn command_id_from_words(words: &[String]) -> String {
    let segments: Vec<String> = words
        .iter()
        .map(|word| word.replace(CLI_WORD_SEPARATOR, &SEGMENT_WORD_SEPARATOR.to_string()))
        .collect();
    segments.join(&ID_SEPARATOR.to_string())
}

/// `conversation` -> `Conversation`. Segments are ASCII (checked by `check_command_id`).
fn capitalized(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cli_words_turn_underscores_into_dashes() {
        assert_eq!(
            cli_words("eval.suite.add_conversation"),
            ["eval", "suite", "add-conversation"]
        );
        assert_eq!(cli_words("token.login_code.create"), ["token", "login-code", "create"]);
        assert_eq!(cli_words("model.load"), ["model", "load"]);
    }

    #[test]
    fn flags_are_dashed_field_names() {
        assert_eq!(flag_name("fail_fast"), "--fail-fast");
        assert_eq!(flag_name("suite"), "--suite");
        assert_eq!(flag_name("max_new_tokens"), "--max-new-tokens");
    }

    #[test]
    fn mcp_tool_names_round_trip() {
        assert_eq!(mcp_tool_name("eval.suite.edit"), "eval__suite__edit");
        assert_eq!(
            mcp_tool_name("eval.suite.add_conversation"),
            "eval__suite__add_conversation"
        );
        assert_eq!(
            command_id_from_mcp_tool_name("eval__suite__add_conversation"),
            "eval.suite.add_conversation"
        );
    }

    #[test]
    fn javascript_wrappers_are_camel_case() {
        assert_eq!(
            javascript_wrapper_name("eval.suite.add_conversation"),
            "evalSuiteAddConversation"
        );
        assert_eq!(javascript_wrapper_name("system.status"), "systemStatus");
        assert_eq!(
            javascript_wrapper_name("token.login_code.create"),
            "tokenLoginCodeCreate"
        );
    }

    #[test]
    fn well_formed_ids_pass() {
        check_command_id("system.status", "").unwrap();
        check_command_id("system.crash_report.list", "").unwrap();
        check_command_id("psychology.replay.open", "psychology").unwrap();
        check_command_id("acme2.tool_x.run", "acme2").unwrap();
    }

    #[test]
    fn malformed_ids_are_refused() {
        for command_id in [
            "",
            "system",
            "system.",
            ".status",
            "System.status",
            "system.status.now.please",
            "system.bad-verb",
            "system._status",
            "system.status_",
            "system.add__x",
            "system.2fast",
            "system.st atus",
        ] {
            let refusal = check_command_id(command_id, "").unwrap_err();
            assert_eq!(refusal.code, CODE_VALUE_NOT_ALLOWED, "{command_id:?}");
        }
    }

    #[test]
    fn core_ids_must_sit_in_a_core_area() {
        let refusal = check_command_id("psychology.replay.open", "").unwrap_err();
        assert_eq!(refusal.code, CODE_VALUE_NOT_ALLOWED);
    }

    #[test]
    fn plugin_ids_stay_in_their_namespace() {
        let outside = check_command_id("eval.test", "psychology").unwrap_err();
        assert_eq!(outside.code, CODE_PLUGIN_COMMAND_OUTSIDE_NAMESPACE);
        let reserved = check_command_id("model.load", "model").unwrap_err();
        assert_eq!(reserved.code, CODE_PLUGIN_COMMAND_OUTSIDE_NAMESPACE);
    }

    #[test]
    fn plugin_ids_are_one_segment() {
        for plugin_id in ["acme", "acme_tools2", "a1_2b"] {
            assert!(is_plugin_id(plugin_id), "{plugin_id:?}");
            check_plugin_id(plugin_id).unwrap();
        }
        for plugin_id in ["", "acme_", "_acme", "my__tools", "Acme", "1acme", "a.b", "a-b"] {
            assert!(!is_plugin_id(plugin_id), "{plugin_id:?}");
            assert_eq!(check_plugin_id(plugin_id).unwrap_err().code, CODE_VALUE_NOT_ALLOWED);
        }
    }

    #[test]
    fn a_malformed_plugin_id_is_named_in_the_refusal() {
        // The refusal names the plugin id, not the command id: "acme_.run" is only wrong because "acme_" is.
        let refusal = check_command_id("acme_.run", "acme_").unwrap_err();
        assert_eq!(refusal.code, CODE_VALUE_NOT_ALLOWED);
        let refusal = check_command_id("acme.run", "acme_").unwrap_err();
        assert_eq!(refusal.code, CODE_VALUE_NOT_ALLOWED);
    }

    #[test]
    fn aliases_and_mode_words_are_the_contract_ones() {
        assert_eq!(ALIASES, [("doctor", "system.doctor"), ("rerun", "run.rerun")]);
        assert!(CLI_MODE_WORDS.contains(&"storage rebuild-index"));
        assert_eq!(CLI_MODE_WORDS.len(), 11);
    }
}
