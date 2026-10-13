//! The in-house argument parser (layout: rust-control-plane.md 9.1, 9.4; contracts 2.1: no CLI-parsing crate). It
//! knows only the global flags, which mean the same for every verb; everything else stays a plain list of words and
//! flags in order, which the mode (modes.rs) or the command's argument schema (verb_arguments.rs) reads. Only the
//! schema knows whether `--name` takes a value, so this parser never guesses.
//!
//! - `--name value`, `--name=value`; `--` ends the flags, and every word after it is a positional word.
//! - Global flags may stand anywhere, before or after the verb.

use crate::error::{CliError, usage_error};

const FLAG_PREFIX: &str = "--";
/// `--` alone: the words after it are positional even when they start with `--`.
const END_OF_FLAGS: &str = "--";
const INLINE_VALUE_SEPARATOR: char = '=';

// The global flags (contracts 15.1, `GLOBAL_FLAGS` in rastai-core's naming module). The first six are switches;
// the rest take a value.
pub const JSON_FLAG: &str = "--json";
pub const DETACH_FLAG: &str = "--detach";
pub const NO_START_FLAG: &str = "--no-start";
pub const VERBOSE_FLAG: &str = "--verbose";
pub const DRY_RUN_FLAG: &str = "--dry-run";
pub const HELP_FLAG: &str = "--help";
pub const NODE_FLAG: &str = "--node";
pub const PROJECT_FLAG: &str = "--project";
pub const SET_FLAG: &str = "--set";
pub const IDEMPOTENCY_KEY_FLAG: &str = "--idempotency-key";
pub const ARGUMENTS_JSON_FLAG: &str = "--arguments-json";
pub const ARGUMENTS_FILE_FLAG: &str = "--arguments-file";

const EXPECTED_SWITCH: &str = "a switch with no value";
const EXPECTED_VALUE: &str = "a value after the flag";

/// One word of the command line that isn't a global flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandLineItem {
    /// A mode or verb word, a positional argument, or the value of the flag before it.
    Word(String),
    /// `--name` or `--name=value`; `name` keeps its dashes (`--max-new-tokens`).
    Flag { name: String, inline_value: Option<String> },
}

/// The global flags (layout 9.4), each read once wherever it stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalFlags {
    /// One JSON object per line on stdout (layout 9.7).
    pub json: bool,
    /// Print a job's id and return without following it.
    pub detach: bool,
    /// Never start a daemon; fail with `EXIT_DAEMON_UNAVAILABLE` when none answers.
    pub no_start: bool,
    pub verbose: bool,
    pub dry_run: bool,
    pub help: bool,
    pub node: Option<String>,
    pub project: Option<String>,
    /// Each `--set section.key=value`, in order.
    pub set: Vec<String>,
    pub idempotency_key: Option<String>,
    pub arguments_json: Option<String>,
    pub arguments_file: Option<String>,
}

/// A whole command line: the global flags, and every other word in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArguments {
    pub items: Vec<CommandLineItem>,
    /// The words after `--`.
    pub words_after_flags: Vec<String>,
    pub global: GlobalFlags,
}

/// Parses the words after the program name. Fails only on a global flag used wrongly (`RRS-C0002`).
pub fn parse_command_line(words: Vec<String>) -> Result<ParsedArguments, CliError> {
    let mut parsed = ParsedArguments::default();
    let mut remaining = words.into_iter();
    while let Some(word) = remaining.next() {
        if word == END_OF_FLAGS {
            parsed.words_after_flags = remaining.collect();
            break;
        }
        let Some(item) = read_item(&word) else {
            parsed.items.push(CommandLineItem::Word(word));
            continue;
        };
        if !read_global_flag(&item, &mut remaining, &mut parsed.global)? {
            parsed.items.push(item);
        }
    }
    Ok(parsed)
}

/// The words before the first flag: they name the mode or the verb, and the positionals that follow it.
pub fn leading_words(parsed: &ParsedArguments) -> Vec<String> {
    let mut words = Vec::new();
    for item in &parsed.items {
        match item {
            CommandLineItem::Word(word) => words.push(word.clone()),
            CommandLineItem::Flag { .. } => break,
        }
    }
    words
}

/// A word as a flag item, or None for a plain word. `-5` and `-` are plain words: only `--` starts a flag.
fn read_item(word: &str) -> Option<CommandLineItem> {
    let flag = word.strip_prefix(FLAG_PREFIX)?;
    let item = match flag.split_once(INLINE_VALUE_SEPARATOR) {
        Some((name, value)) => CommandLineItem::Flag {
            name: format!("{FLAG_PREFIX}{name}"),
            inline_value: Some(value.to_string()),
        },
        None => CommandLineItem::Flag {
            name: word.to_string(),
            inline_value: None,
        },
    };
    Some(item)
}

/// Reads the item into `global` when it is a global flag (taking its value from `remaining` if needed) and says
/// whether it was one.
fn read_global_flag(
    item: &CommandLineItem,
    remaining: &mut impl Iterator<Item = String>,
    global: &mut GlobalFlags,
) -> Result<bool, CliError> {
    let CommandLineItem::Flag { name, inline_value } = item else {
        return Ok(false);
    };
    if let Some(switch) = global_switch(global, name) {
        if inline_value.is_some() {
            return Err(usage_error(&shown_flag(name, inline_value), EXPECTED_SWITCH));
        }
        *switch = true;
        return Ok(true);
    }
    if !is_global_value_flag(name) {
        return Ok(false);
    }
    let value = match inline_value {
        Some(value) => value.clone(),
        None => next_value(name, remaining)?,
    };
    store_global_value(global, name, value);
    Ok(true)
}

fn global_switch<'a>(global: &'a mut GlobalFlags, name: &str) -> Option<&'a mut bool> {
    match name {
        JSON_FLAG => Some(&mut global.json),
        DETACH_FLAG => Some(&mut global.detach),
        NO_START_FLAG => Some(&mut global.no_start),
        VERBOSE_FLAG => Some(&mut global.verbose),
        DRY_RUN_FLAG => Some(&mut global.dry_run),
        HELP_FLAG => Some(&mut global.help),
        _ => None,
    }
}

fn is_global_value_flag(name: &str) -> bool {
    [
        NODE_FLAG,
        PROJECT_FLAG,
        SET_FLAG,
        IDEMPOTENCY_KEY_FLAG,
        ARGUMENTS_JSON_FLAG,
        ARGUMENTS_FILE_FLAG,
    ]
    .contains(&name)
}

fn store_global_value(global: &mut GlobalFlags, name: &str, value: String) {
    match name {
        NODE_FLAG => global.node = Some(value),
        PROJECT_FLAG => global.project = Some(value),
        SET_FLAG => global.set.push(value),
        IDEMPOTENCY_KEY_FLAG => global.idempotency_key = Some(value),
        ARGUMENTS_JSON_FLAG => global.arguments_json = Some(value),
        _ => global.arguments_file = Some(value),
    }
}

/// The word after a flag that takes a value. A missing word, or another flag in its place, is refused: no value of
/// a global flag starts with `--`, so `--project --json` is a forgotten value, not a project named `--json`.
fn next_value(name: &str, remaining: &mut impl Iterator<Item = String>) -> Result<String, CliError> {
    match remaining.next() {
        Some(value) if !value.starts_with(FLAG_PREFIX) => Ok(value),
        _ => Err(usage_error(name, EXPECTED_VALUE)),
    }
}

/// A flag as the user wrote it, for a refusal.
pub fn shown_flag(name: &str, inline_value: &Option<String>) -> String {
    match inline_value {
        Some(value) => format!("{name}{INLINE_VALUE_SEPARATOR}{value}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rastai_core::commands::naming::GLOBAL_FLAGS;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn every_contract_global_flag_is_read() {
        let mut known = vec![
            JSON_FLAG,
            DETACH_FLAG,
            NO_START_FLAG,
            VERBOSE_FLAG,
            DRY_RUN_FLAG,
            HELP_FLAG,
        ];
        known.extend([
            NODE_FLAG,
            PROJECT_FLAG,
            SET_FLAG,
            IDEMPOTENCY_KEY_FLAG,
            ARGUMENTS_JSON_FLAG,
            ARGUMENTS_FILE_FLAG,
        ]);
        known.sort();
        let mut contract = GLOBAL_FLAGS.to_vec();
        contract.sort();
        assert_eq!(known, contract);
    }

    #[test]
    fn leading_words_stop_at_the_first_flag() {
        let parsed = parse_command_line(words("eval test x --model m y")).unwrap();
        assert_eq!(leading_words(&parsed), ["eval", "test", "x"]);
    }

    #[test]
    fn a_global_switch_takes_no_value() {
        let refusal = parse_command_line(words("system status --json=true")).unwrap_err();
        assert_eq!(refusal.exit_code(), 2);
    }
}
