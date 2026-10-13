//! `Mode` and `choose_mode` (layout: rust-control-plane.md 9.2): the first words pick a process mode, a local
//! command, help, or a verb. The fixed words come from rastai-core's naming module (`CLI_MODE_WORDS`), so the names
//! codegen lists exactly the words this file dispatches; every other word is a verb, resolved later against the
//! daemon's registry (verbs.rs). The CLI keeps no list of verbs of its own.

use rastai_core::commands::naming::CLI_MODE_WORDS;

use crate::arguments::{CommandLineItem, ParsedArguments, leading_words, shown_flag};
use crate::error::{CliError, usage_error};

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
    /// `rastai storage rebuild-index [--verify-blobs]` (M1-33 builds the rebuild itself).
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

/// The mode the command line asks for, its own flags read. A flag the mode doesn't take is `RRS-C0002`.
pub fn choose_mode(arguments: &ParsedArguments) -> Result<Mode, CliError> {
    let words = leading_words(arguments);
    if arguments.global.help {
        return Ok(Mode::Help(words));
    }
    if arguments.items.is_empty() {
        return Ok(Mode::OpenUi(OpenFlags::default()));
    }
    let Some(mode_words) = fixed_words_of(&words) else {
        return Ok(Mode::Verb(words));
    };
    let word_count = mode_words.split(' ').count();
    let rest = &arguments.items[word_count..];
    mode_from_words(mode_words, rest, arguments)
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
    let found = match item {
        CommandLineItem::Word(word) => word.clone(),
        CommandLineItem::Flag { name, inline_value } => shown_flag(name, inline_value),
    };
    if allowed_flags.is_empty() {
        return usage_error(&found, EXPECTED_NO_WORDS);
    }
    usage_error(&found, &allowed_flags.join(", "))
}
