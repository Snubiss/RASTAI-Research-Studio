//! The parser table (layout: rust-control-plane.md 9.1, 9.4): command lines to their global flags, words and
//! flags, and to the mode they choose. Every fixed word of `CLI_MODE_WORDS` picks its own mode, never a verb.

#![allow(clippy::unwrap_used)]

use rastai_cli::arguments::{CommandLineItem, GlobalFlags, ParsedArguments, leading_words, parse_command_line};
use rastai_cli::modes::{LocalCommand, Mode, OpenFlags, ServeFlags, choose_mode, mode_words};
use rastai_core::commands::naming::CLI_MODE_WORDS;

fn words(line: &str) -> Vec<String> {
    if line.is_empty() {
        return Vec::new();
    }
    line.split(' ').map(str::to_string).collect()
}

fn parse(line: &str) -> ParsedArguments {
    parse_command_line(words(line)).unwrap()
}

fn mode_of(line: &str) -> Mode {
    choose_mode(&parse(line)).unwrap()
}

/// The exit code of a command line the parser or the mode refuses.
fn refusal_exit_code(line: &str) -> u8 {
    match parse_command_line(words(line)) {
        Err(error) => error.exit_code(),
        Ok(parsed) => choose_mode(&parsed).unwrap_err().exit_code(),
    }
}

fn word(text: &str) -> CommandLineItem {
    CommandLineItem::Word(text.to_string())
}

fn flag(name: &str, inline_value: Option<&str>) -> CommandLineItem {
    CommandLineItem::Flag {
        name: name.to_string(),
        inline_value: inline_value.map(str::to_string),
    }
}

fn owned(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| text.to_string()).collect()
}

#[test]
fn global_flags_are_read_wherever_they_stand() {
    let parsed = parse("--json eval test x --verbose --model=m --project p --set a.b=1 --set c.d=2 --detach");
    let expected = GlobalFlags {
        json: true,
        verbose: true,
        detach: true,
        project: Some("p".to_string()),
        set: owned(&["a.b=1", "c.d=2"]),
        ..GlobalFlags::default()
    };
    assert_eq!(parsed.global, expected);
    assert_eq!(
        parsed.items,
        [word("eval"), word("test"), word("x"), flag("--model", Some("m"))]
    );
    assert_eq!(leading_words(&parsed), ["eval", "test", "x"]);
}

#[test]
fn every_global_value_flag_takes_its_value_either_way() {
    let parsed = parse(
        "--node n --project=p --idempotency-key k --arguments-json {} --arguments-file=a.json --no-start --dry-run",
    );
    assert_eq!(parsed.global.node.as_deref(), Some("n"));
    assert_eq!(parsed.global.project.as_deref(), Some("p"));
    assert_eq!(parsed.global.idempotency_key.as_deref(), Some("k"));
    assert_eq!(parsed.global.arguments_json.as_deref(), Some("{}"));
    assert_eq!(parsed.global.arguments_file.as_deref(), Some("a.json"));
    assert!(parsed.global.no_start && parsed.global.dry_run);
    assert!(parsed.items.is_empty());
}

#[test]
fn command_flags_stay_in_order_and_unread() {
    let parsed = parse("token create bot --scopes read --scopes=chat --no-include-revoked -5 -");
    assert_eq!(
        parsed.items,
        [
            word("token"),
            word("create"),
            word("bot"),
            flag("--scopes", None),
            word("read"),
            flag("--scopes", Some("chat")),
            flag("--no-include-revoked", None),
            word("-5"),
            word("-"),
        ]
    );
}

#[test]
fn a_value_may_hold_an_equals_sign() {
    let parsed = parse("x y --set-env=A=1 --set k=v=w");
    assert_eq!(parsed.items[2], flag("--set-env", Some("A=1")));
    assert_eq!(parsed.global.set, ["k=v=w"]);
}

#[test]
fn words_after_a_double_dash_are_positional() {
    let parsed = parse("validate a.json -- --odd.json --json");
    assert_eq!(parsed.items, [word("validate"), word("a.json")]);
    assert_eq!(parsed.words_after_flags, ["--odd.json", "--json"]);
    assert!(!parsed.global.json);
    assert_eq!(
        mode_of("validate a.json -- --odd.json"),
        Mode::Local(LocalCommand::Validate(owned(&["a.json", "--odd.json"])))
    );
}

#[test]
fn modes_are_chosen_from_their_fixed_words() {
    let table = [
        ("", Mode::OpenUi(OpenFlags::default())),
        ("--json", Mode::OpenUi(OpenFlags::default())),
        (
            "open --print-url",
            Mode::OpenUi(OpenFlags {
                print_url: true,
                replace_older_daemon: false,
            }),
        ),
        (
            "open --replace-older-daemon --print-url",
            Mode::OpenUi(OpenFlags {
                print_url: true,
                replace_older_daemon: true,
            }),
        ),
        ("serve", Mode::Serve(ServeFlags::default())),
        (
            "serve --headless --port 8899",
            Mode::Serve(ServeFlags {
                headless: true,
                port: Some(8899),
            }),
        ),
        (
            "serve --port=0",
            Mode::Serve(ServeFlags {
                headless: false,
                port: Some(0),
            }),
        ),
        ("node serve", Mode::NodeServe),
        ("node init --daemon-key k", Mode::NodeInit),
        ("node check-update", Mode::NodeCheckUpdate),
        ("tunnel me@box", Mode::Tunnel),
        ("mcp", Mode::McpBridge),
        (
            "validate a.rrsmodel b.json",
            Mode::Local(LocalCommand::Validate(owned(&["a.rrsmodel", "b.json"]))),
        ),
        ("version", Mode::Local(LocalCommand::Version)),
        (
            "storage rebuild-index --verify-blobs",
            Mode::Local(LocalCommand::RebuildIndex { verify_blobs: true }),
        ),
        ("help", Mode::Help(Vec::new())),
        ("help eval test", Mode::Help(owned(&["eval", "test"]))),
        ("eval test --help", Mode::Help(owned(&["eval", "test"]))),
        ("serve --help", Mode::Help(owned(&["serve"]))),
    ];
    for (line, expected) in table {
        assert_eq!(mode_of(line), expected, "{line:?}");
    }
}

#[test]
fn everything_else_is_a_verb() {
    let table = [
        ("system status", owned(&["system", "status"])),
        (
            "eval suite add-conversation x --json",
            owned(&["eval", "suite", "add-conversation", "x"]),
        ),
        ("doctor", owned(&["doctor"])),
        ("node list", owned(&["node", "list"])),
        ("storage gc", owned(&["storage", "gc"])),
    ];
    for (line, expected) in table {
        assert_eq!(mode_of(line), Mode::Verb(expected), "{line:?}");
    }
}

#[test]
fn a_misused_flag_is_a_usage_error() {
    for line in [
        "system status --json=yes",
        "system status --project",
        "system status --project --json",
        "serve --port",
        "serve --port 70000",
        "serve --colour",
        "open now",
        "open --headless",
        "mcp extra",
        "version --x",
        "validate",
        "validate --strict a.json",
        "storage rebuild-index --fast",
    ] {
        assert_eq!(refusal_exit_code(line), 2, "{line:?}");
    }
}

/// Refused when the mode is chosen, before main.rs starts any daemon: flags with no command, `--node` (M9) and
/// `--set` on a verb.
#[test]
fn a_line_that_cannot_be_a_verb_is_refused_before_a_daemon_starts() {
    let table = [
        ("--frob", 2, "RRS-C0002"),
        ("--print-url", 2, "RRS-C0002"),
        ("--frob=1 system status", 2, "RRS-C0002"),
        ("system status --node gpu-box", 1, "RRS-C0010"),
        ("system status --set ui.theme=dark", 2, "RRS-C0002"),
    ];
    for (line, exit_code, code) in table {
        let refusal = choose_mode(&parse(line)).unwrap_err();
        assert_eq!(refusal.exit_code(), exit_code, "{line:?}");
        assert_eq!(refusal.diagnostics()[0].code, code, "{line:?}");
    }
    let refusal = choose_mode(&parse("--frob")).unwrap_err();
    assert!(refusal.diagnostics()[0].message.contains("--frob"));
}

/// A mode refuses what it would ignore: a global flag only a verb reads (`--set` outside `serve`), and words after
/// `--` outside `validate` and a verb. Nothing opens the UI or serves on a line it doesn't understand.
#[test]
fn a_mode_refuses_the_flags_and_words_it_would_ignore() {
    for line in [
        "-- stray",
        "--json -- stray words",
        "open -- x",
        "serve -- x",
        "mcp -- x",
        "help -- x",
        "version -- x",
        "--set ui.theme=dark",
        "open --set ui.theme=dark",
        "mcp --set ui.theme=dark",
        "validate a.json --set ui.theme=dark",
        "--project p",
        "open --dry-run",
        "open --detach",
        "open --idempotency-key k",
        "mcp --node gpu-box",
        "mcp --project p",
        "help --dry-run",
        "system status --help --detach",
        "validate a.json --project p",
        "validate a.json --arguments-json {}",
        "version --arguments-file a.json",
        "serve --dry-run",
        "serve --project p",
        "storage rebuild-index --detach",
    ] {
        let refusal = choose_mode(&parse(line)).unwrap_err();
        assert_eq!(refusal.exit_code(), 2, "{line:?}");
        assert_eq!(refusal.diagnostics()[0].code, "RRS-C0002", "{line:?}");
    }
    let refusal = choose_mode(&parse("-- stray")).unwrap_err();
    assert!(refusal.diagnostics()[0].message.contains("-- stray"));
    // What each mode does read still passes.
    assert_eq!(
        mode_of("serve --set ui.theme=dark --json --verbose"),
        Mode::Serve(ServeFlags::default())
    );
    assert_eq!(mode_of("--"), Mode::OpenUi(OpenFlags::default()));
    assert_eq!(mode_of("open --no-start --json"), Mode::OpenUi(OpenFlags::default()));
    assert_eq!(mode_of("mcp --no-start"), Mode::McpBridge);
    assert_eq!(
        mode_of("validate -- a.json"),
        Mode::Local(LocalCommand::Validate(owned(&["a.json"])))
    );
    assert_eq!(
        mode_of("system status --project p --dry-run -- x"),
        Mode::Verb(owned(&["system", "status"]))
    );
}

#[test]
fn every_mode_names_its_own_words() {
    for fixed_words in CLI_MODE_WORDS {
        let line = match *fixed_words {
            "validate" => "validate a.json".to_string(),
            other => other.to_string(),
        };
        assert_eq!(mode_words(&mode_of(&line)), *fixed_words);
    }
    assert_eq!(mode_words(&mode_of("system status")), "system status");
}

#[test]
fn every_fixed_word_picks_a_mode() {
    for mode_words in CLI_MODE_WORDS {
        let line = match *mode_words {
            "validate" => "validate a.json".to_string(),
            other => other.to_string(),
        };
        let mode = mode_of(&line);
        assert!(!matches!(mode, Mode::Verb(_)), "{mode_words} reads as a verb");
    }
}
