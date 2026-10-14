//! `Mode` and `choose_mode` (layout: rust-control-plane.md 9.2): the first words pick a process mode, a local
//! command, help, or a verb. The fixed words come from rastai-core's naming module (`CLI_MODE_WORDS`), so the names
//! codegen lists exactly the words this file dispatches; every other word is a verb, resolved later against the
//! daemon's registry (verbs.rs). The CLI keeps no list of verbs of its own.

use rastai_core::commands::naming::CLI_MODE_WORDS;

use crate::arguments::{
    ARGUMENTS_FILE_FLAG, ARGUMENTS_JSON_FLAG, CommandLineItem, DETACH_FLAG, DRY_RUN_FLAG, GlobalFlags,
    IDEMPOTENCY_KEY_FLAG, NODE_FLAG, PROJECT_FLAG, ParsedArguments, SET_FLAG, leading_words, shown_flag,
};
use crate::error::{CliError, not_available_yet, usage_error};

// The fixed words of CLI_MODE_WORDS, each matched once below.
const OPEN_WORDS: &str = "open";
const SERVE_WORDS: &str = "serve";
const NODE_SERVE_WORDS: &str = "node serve";
const NODE_INIT_WORDS: &str = "node init";
const NODE_CHECK_UPDATE_WORDS: &str = "node check-update";
const TUNNEL_WORDS: &str = "tunnel";
const MCP_WORDS: &str = "mcp";
const VALIDATE_WORDS: &str = "validate";
const VERSION_WORDS: &str = "version";
const REBUILD_INDEX_WORDS: &str = "storage rebuild-index";
const HELP_WORDS: &str = "help";
/// The longest fixed words are two words long (`node serve`, `storage rebuild-index`).
const MAX_MODE_WORDS: usize = 2;

// The flags each mode reads.
const PRINT_URL_FLAG: &str = "--print-url";
const REPLACE_OLDER_DAEMON_FLAG: &str = "--replace-older-daemon";
const HEADLESS_FLAG: &str = "--headless";
const PORT_FLAG: &str = "--port";
const VERIFY_BLOBS_FLAG: &str = "--verify-blobs";

const EXPECTED_NO_WORDS: &str = "no further words";
const EXPECTED_COMMAND_FIRST: &str = "a command or mode word before its flags";
const EXPECTED_COMMAND_BEFORE_END_OF_FLAGS: &str = "a command or mode word before --";
const END_OF_FLAGS: &str = "--";
const SET_ONLY_WITH_SERVE: &str = "--set only with rastai serve, or settings set for a running daemon";
const EXPECTED_FILES: &str = "one or more document files";
const EXPECTED_PORT: &str = "a port number from 0 to 65535";

/// `rastai`, `rastai open [--print-url] [--replace-older-daemon]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenFlags {
    /// Print one `DesktopLaunchInfo` JSON line instead of opening the browser (the desktop shell's way in).
    pub print_url: bool,
    /// Stop a daemon of another version with `system.shutdown` and start this one (flows/platform.md 9.2).
    pub replace_older_daemon: bool,
}

/// `rastai serve [--headless] [--port N]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeFlags {
    /// Never open a browser.
    pub headless: bool,
    pub port: Option<u16>,
}

/// The commands that work without a daemon (layout 9.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    /// `rastai validate <file>...`
    Validate(Vec<String>),
    /// `rastai version`
    Version,
    /// `rastai storage rebuild-index [--verify-blobs]`: rastai-store rebuilds `rastai.db` while this holds the lock.
    RebuildIndex { verify_blobs: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    OpenUi(OpenFlags),
    Serve(ServeFlags),
    /// `rastai node serve` (M9: the standalone node).
    NodeServe,
    /// `rastai node init --daemon-key <key>` (M9: pairing).
    NodeInit,
    /// (L2) `rastai node check-update`, run by the node's systemd unit before each start.
    NodeCheckUpdate,
    /// `rastai tunnel <user@host>` (M9: the headless box).
    Tunnel,
    McpBridge,
    Local(LocalCommand),
    /// `rastai help [words]`, or `--help` anywhere: the words to explain.
    Help(Vec<String>),
    /// Everything else: the leading words of a registry command.
    Verb(Vec<String>),
}

/// The mode the command line asks for, its own flags read. A flag the mode doesn't take is `RRS-C0002`, a global
/// one only a verb reads included: a mode that would ignore it refuses it instead.
pub fn choose_mode(arguments: &ParsedArguments) -> Result<Mode, CliError> {
    let words = leading_words(arguments);
    let mode = if arguments.global.help {
        Mode::Help(words)
    } else if arguments.items.is_empty() {
        Mode::OpenUi(OpenFlags::default())
    } else {
        let Some(mode_words) = fixed_words_of(&words) else {
            return verb_mode(words, arguments);
        };
        let word_count = mode_words.split(' ').count();
        let rest = &arguments.items[word_count..];
        mode_from_words(mode_words, rest, arguments)?
    };
    refuse_what_the_mode_ignores(&mode, arguments)?;
    Ok(mode)
}

/// Words after `--` and the global flags only a verb reads, refused in a mode that would ignore them: a line of
/// only `--` and stray words doesn't open the UI, and `rastai open --dry-run` doesn't pretend to be a dry run.
/// `--set` is `serve`'s too, and `validate` reads the words after `--` as files. The modes not built yet (M9, L2)
/// are refused whole with `RRS-C0010`, so their flags aren't looked at. A `--help` on a verb line wins: the verb's
/// flags are only read for their help, so they are ignored rather than refused.
fn refuse_what_the_mode_ignores(mode: &Mode, arguments: &ParsedArguments) -> Result<(), CliError> {
    match mode {
        Mode::NodeServe | Mode::NodeInit | Mode::NodeCheckUpdate | Mode::Tunnel | Mode::Verb(_) => return Ok(()),
        _ => {}
    }
    let takes_words_after_flags = matches!(mode, Mode::Local(LocalCommand::Validate(_)));
    if !arguments.words_after_flags.is_empty() && !takes_words_after_flags {
        let found = format!("{END_OF_FLAGS} {}", arguments.words_after_flags.join(" "));
        let expected = match mode {
            Mode::OpenUi(_) if arguments.items.is_empty() => EXPECTED_COMMAND_BEFORE_END_OF_FLAGS,
            _ => EXPECTED_NO_WORDS,
        };
        return Err(usage_error(&found, expected));
    }
    let global = &arguments.global;
    if !matches!(mode, Mode::Serve(_))
        && let Some(set_flag) = global.set.first()
    {
        return Err(usage_error(&format!("{SET_FLAG} {set_flag}"), SET_ONLY_WITH_SERVE));
    }
    let help_wins = matches!(mode, Mode::Help(_)) && global.help;
    if let Some(flag) = verb_only_flags_given(global).first()
        && !help_wins
    {
        let expected = format!("a command's flag, which rastai {} doesn't read", mode_words(mode));
        return Err(usage_error(flag, &expected));
    }
    Ok(())
}

/// The global flags given that only a verb reads, as typed (layout 9.4).
fn verb_only_flags_given(global: &GlobalFlags) -> Vec<String> {
    let mut given = Vec::new();
    if let Some(node) = &global.node {
        given.push(format!("{NODE_FLAG} {node}"));
    }
    if let Some(project) = &global.project {
        given.push(format!("{PROJECT_FLAG} {project}"));
    }
    if global.dry_run {
        given.push(DRY_RUN_FLAG.to_string());
    }
    if global.detach {
        given.push(DETACH_FLAG.to_string());
    }
    if let Some(key) = &global.idempotency_key {
        given.push(format!("{IDEMPOTENCY_KEY_FLAG} {key}"));
    }
    if global.arguments_json.is_some() {
        given.push(ARGUMENTS_JSON_FLAG.to_string());
    }
    if let Some(file) = &global.arguments_file {
        given.push(format!("{ARGUMENTS_FILE_FLAG} {file}"));
    }
    given
}

/// A verb's words, refused here when the line can't be a verb, so no daemon is started only to refuse it: flags
/// with no command before them, `--node` (M9), and settings flags, which are the daemon's start-up layer (contracts
/// 11.1) that a verb can't change on a running daemon.
fn verb_mode(words: Vec<String>, arguments: &ParsedArguments) -> Result<Mode, CliError> {
    if words.is_empty()
        && let Some(first) = arguments.items.first()
    {
        return Err(usage_error(&shown_item(first), EXPECTED_COMMAND_FIRST));
    }
    if arguments.global.node.is_some() {
        return Err(not_available_yet(NODE_FLAG));
    }
    if let Some(set_flag) = arguments.global.set.first() {
        return Err(usage_error(&format!("{SET_FLAG} {set_flag}"), SET_ONLY_WITH_SERVE));
    }
    Ok(Mode::Verb(words))
}

/// The words that name a mode, for a refusal that has to say which: the fixed words, or a verb's own.
pub fn mode_words(mode: &Mode) -> String {
    let fixed = match mode {
        Mode::OpenUi(_) => OPEN_WORDS,
        Mode::Serve(_) => SERVE_WORDS,
        Mode::NodeServe => NODE_SERVE_WORDS,
        Mode::NodeInit => NODE_INIT_WORDS,
        Mode::NodeCheckUpdate => NODE_CHECK_UPDATE_WORDS,
        Mode::Tunnel => TUNNEL_WORDS,
        Mode::McpBridge => MCP_WORDS,
        Mode::Local(LocalCommand::Validate(_)) => VALIDATE_WORDS,
        Mode::Local(LocalCommand::Version) => VERSION_WORDS,
        Mode::Local(LocalCommand::RebuildIndex { .. }) => REBUILD_INDEX_WORDS,
        Mode::Help(_) => HELP_WORDS,
        Mode::Verb(words) => return words.join(" "),
    };
    fixed.to_string()
}

/// The longest entry of `CLI_MODE_WORDS` the leading words start with.
pub fn fixed_words_of(words: &[String]) -> Option<&'static str> {
    let longest = MAX_MODE_WORDS.min(words.len());
    for word_count in (1..=longest).rev() {
        let candidate = words[..word_count].join(" ");
        if let Some(found) = CLI_MODE_WORDS.iter().find(|fixed| **fixed == candidate) {
            return Some(found);
        }
    }
    None
}

fn mode_from_words(mode_words: &str, rest: &[CommandLineItem], arguments: &ParsedArguments) -> Result<Mode, CliError> {
    let mode = match mode_words {
        OPEN_WORDS => Mode::OpenUi(open_flags(rest)?),
        SERVE_WORDS => Mode::Serve(serve_flags(rest)?),
        NODE_SERVE_WORDS => Mode::NodeServe,
        NODE_INIT_WORDS => Mode::NodeInit,
        NODE_CHECK_UPDATE_WORDS => Mode::NodeCheckUpdate,
        TUNNEL_WORDS => Mode::Tunnel,
        MCP_WORDS => no_more_words(rest, Mode::McpBridge)?,
        VALIDATE_WORDS => Mode::Local(LocalCommand::Validate(files_to_validate(rest, arguments)?)),
        VERSION_WORDS => no_more_words(rest, Mode::Local(LocalCommand::Version))?,
        REBUILD_INDEX_WORDS => Mode::Local(rebuild_index_flags(rest)?),
        HELP_WORDS => Mode::Help(plain_words(rest)),
        // Every entry of CLI_MODE_WORDS has an arm above (tests/arguments.rs checks it); a new entry without one
        // reads as a verb until its mode lands.
        _ => Mode::Verb(plain_words(rest)),
    };
    Ok(mode)
}

fn open_flags(rest: &[CommandLineItem]) -> Result<OpenFlags, CliError> {
    let mut flags = OpenFlags::default();
    for item in rest {
        match flag_of(item) {
            Some((PRINT_URL_FLAG, None)) => flags.print_url = true,
            Some((REPLACE_OLDER_DAEMON_FLAG, None)) => flags.replace_older_daemon = true,
            _ => return Err(unexpected(item, &[PRINT_URL_FLAG, REPLACE_OLDER_DAEMON_FLAG])),
        }
    }
    Ok(flags)
}

fn serve_flags(rest: &[CommandLineItem]) -> Result<ServeFlags, CliError> {
    let mut flags = ServeFlags::default();
    let mut index = 0;
    while index < rest.len() {
        let item = &rest[index];
        match flag_of(item) {
            Some((HEADLESS_FLAG, None)) => flags.headless = true,
            Some((PORT_FLAG, Some(value))) => flags.port = Some(port_number(value)?),
            Some((PORT_FLAG, None)) => {
                index += 1;
                let value = word_at(rest, index).ok_or_else(|| usage_error(PORT_FLAG, EXPECTED_PORT))?;
                flags.port = Some(port_number(value)?);
            }
            _ => return Err(unexpected(item, &[HEADLESS_FLAG, PORT_FLAG])),
        }
        index += 1;
    }
    Ok(flags)
}

fn rebuild_index_flags(rest: &[CommandLineItem]) -> Result<LocalCommand, CliError> {
    let mut verify_blobs = false;
    for item in rest {
        match flag_of(item) {
            Some((VERIFY_BLOBS_FLAG, None)) => verify_blobs = true,
            _ => return Err(unexpected(item, &[VERIFY_BLOBS_FLAG])),
        }
    }
    Ok(LocalCommand::RebuildIndex { verify_blobs })
}

/// The files after `validate`, the words after `--` included, so a file named `--x` can still be checked.
fn files_to_validate(rest: &[CommandLineItem], arguments: &ParsedArguments) -> Result<Vec<String>, CliError> {
    let mut files = Vec::new();
    for item in rest {
        match item {
            CommandLineItem::Word(word) => files.push(word.clone()),
            CommandLineItem::Flag { .. } => return Err(unexpected(item, &[])),
        }
    }
    files.extend(arguments.words_after_flags.iter().cloned());
    if files.is_empty() {
        return Err(usage_error(VALIDATE_WORDS, EXPECTED_FILES));
    }
    Ok(files)
}

fn no_more_words(rest: &[CommandLineItem], mode: Mode) -> Result<Mode, CliError> {
    match rest.first() {
        Some(item) => Err(unexpected(item, &[])),
        None => Ok(mode),
    }
}

fn plain_words(rest: &[CommandLineItem]) -> Vec<String> {
    let mut words = Vec::new();
    for item in rest {
        if let CommandLineItem::Word(word) = item {
            words.push(word.clone());
        }
    }
    words
}

fn flag_of(item: &CommandLineItem) -> Option<(&str, Option<&str>)> {
    match item {
        CommandLineItem::Flag { name, inline_value } => Some((name.as_str(), inline_value.as_deref())),
        CommandLineItem::Word(_) => None,
    }
}

fn word_at(items: &[CommandLineItem], index: usize) -> Option<&str> {
    match items.get(index) {
        Some(CommandLineItem::Word(word)) => Some(word),
        _ => None,
    }
}

fn port_number(text: &str) -> Result<u16, CliError> {
    text.parse::<u16>()
        .map_err(|_| usage_error(&format!("{PORT_FLAG} {text}"), EXPECTED_PORT))
}

/// A word or flag the mode doesn't take, with the flags it does.
fn unexpected(item: &CommandLineItem, allowed_flags: &[&str]) -> CliError {
    let found = shown_item(item);
    if allowed_flags.is_empty() {
        return usage_error(&found, EXPECTED_NO_WORDS);
    }
    usage_error(&found, &allowed_flags.join(", "))
}

/// An item as the user typed it.
fn shown_item(item: &CommandLineItem) -> String {
    match item {
        CommandLineItem::Word(word) => word.clone(),
        CommandLineItem::Flag { name, inline_value } => shown_flag(name, inline_value),
    }
}
