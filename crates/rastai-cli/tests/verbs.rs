//! Generated verbs (layout: rust-control-plane.md 9.3, 9.4, 10.3):
//! - every CLI line of `fixtures/command-golden/copy-as.json` parses back to its command id and its arguments, with
//!   each command's flags taken from its argument schema (`schemas/commands/`, or, for a command not built yet, a
//!   schema of the example's own value kinds);
//! - flags of every kind, positionals, `--arguments-json` and its key-by-key override;
//! - a verb with no daemon running starts one, and help lists the verbs from the command cache once it is gone;
//! - a daemon that can't start reports its own reason, read from its final `--json` line, and exit 3.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs;
use std::path::Path;

use rastai_cli::arguments::{ParsedArguments, parse_command_line};
use rastai_cli::modes::{Mode, choose_mode};
use rastai_cli::verb_arguments::build_arguments_json;
use rastai_cli::verbs::find_verb;
use rastai_core::commands::registry::CommandRegistry;
use rastai_core::versions::APP_VERSION;
use rastai_proto::commands::v1::CommandDescriptor;
use serde_json::{Map, Value, json};

use common::{
    DaemonGuard, daemon_answers, daemon_processes, final_codes, final_line, run_rastai, run_rastai_in, test_home,
};

const COPY_AS_GOLDEN: &str = "fixtures/command-golden/copy-as.json";
const SCHEMAS_FOLDER: &str = "schemas/commands";
const PROGRAM_NAME: &str = "rastai";

fn repository_file(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

/// The arguments schema of a core command, as the daemon fills `arguments_schema_json`; for one without a schema
/// file yet (a plugin's, a later milestone's), a schema of the example's own value kinds.
fn arguments_schema(command_id: &str, example_arguments: &Value) -> Value {
    let file = repository_file(&format!("{SCHEMAS_FOLDER}/rastai.command.{command_id}/1.json"));
    if let Ok(text) = fs::read_to_string(file) {
        let schema: Value = serde_json::from_str(&text).unwrap();
        return schema["$defs"]["arguments"].clone();
    }
    let mut properties = Map::new();
    for (name, value) in example_arguments.as_object().unwrap() {
        properties.insert(name.clone(), json!({ "type": json_kind(value) }));
    }
    json!({ "type": "object", "properties": properties, "required": [], "additionalProperties": false })
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        _ => "string",
    }
}

fn descriptor(command_id: &str, schema: &Value) -> CommandDescriptor {
    CommandDescriptor {
        command_id: command_id.to_string(),
        arguments_schema_json: schema.to_string(),
        ..CommandDescriptor::default()
    }
}

/// The words of a CLI line as a POSIX shell splits it: spaces separate words, single quotes keep everything up to
/// the next single quote, and a backslash outside quotes keeps the next character (`'it'\''s'` is `it's`).
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            ' ' => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for quoted in characters.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    current.push(quoted);
                }
            }
            '\\' => {
                in_word = true;
                current.extend(characters.next());
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

/// Parses a command line the way main.rs does, to the verb's words.
fn verb_of(words: Vec<String>) -> (Vec<String>, ParsedArguments) {
    let parsed = parse_command_line(words).unwrap();
    let Mode::Verb(verb_words) = choose_mode(&parsed).unwrap() else {
        panic!("not a verb: {parsed:?}");
    };
    (verb_words, parsed)
}

/// The command id and arguments a command line comes to, against this registry.
fn command_and_arguments(registry: &CommandRegistry, words: Vec<String>) -> Result<(String, Value), Vec<String>> {
    let (verb_words, parsed) = verb_of(words);
    let command = find_verb(registry, &verb_words).map_err(|error| codes(error.diagnostics()))?;
    let schema: Value = serde_json::from_str(&command.arguments_schema_json).unwrap();
    let arguments = build_arguments_json(command, &schema, &parsed).map_err(|problems| codes(&problems))?;
    Ok((command.command_id.clone(), serde_json::from_str(&arguments).unwrap()))
}

fn codes(diagnostics: &[rastai_diag::diagnostic::Diagnostic]) -> Vec<String> {
    diagnostics.iter().map(|item| item.code.clone()).collect()
}

fn words(line: &str) -> Vec<String> {
    shell_words(line)
}

#[test]
fn every_copy_as_cli_line_parses_to_its_command_and_arguments() {
    let golden: Value = serde_json::from_str(&fs::read_to_string(repository_file(COPY_AS_GOLDEN)).unwrap()).unwrap();
    let examples = golden["examples"].as_array().unwrap();
    let commands = examples
        .iter()
        .map(|example| {
            let command_id = example["command_id"].as_str().unwrap();
            descriptor(command_id, &arguments_schema(command_id, &example["arguments"]))
        })
        .collect();
    let registry = CommandRegistry { commands };
    for example in examples {
        let line = example["cli"].as_str().unwrap();
        let mut line_words = shell_words(line);
        assert_eq!(line_words.remove(0), PROGRAM_NAME, "{line}");
        let (command_id, arguments) = command_and_arguments(&registry, line_words).unwrap_or_else(|codes| {
            panic!("{line}: {codes:?}");
        });
        assert_eq!(command_id, example["command_id"].as_str().unwrap(), "{line}");
        assert_eq!(arguments, example["arguments"], "{line}");
    }
}

/// A command with a field of every kind the command line can write.
fn every_kind_registry() -> CommandRegistry {
    let schema = json!({
        "type": "object",
        "properties": {
            "suite": {"type": "string", "x-rastai-cli-position": 0},
            "model": {"type": "string", "x-rastai-cli-position": 1},
            "max_new_tokens": {"type": "integer"},
            "temperature": {"type": "number"},
            "fail_fast": {"type": "boolean"},
            "runtime": {"type": "string", "enum": ["fast", "inspectable"]},
            "gpu": {"type": "array", "items": {"type": "integer"}},
            "set_env": {"type": "object", "additionalProperties": {"type": "string"}},
            "sampling": {"$ref": "#/$defs/SamplingSettings"}
        },
        "required": ["suite"],
        "additionalProperties": false
    });
    CommandRegistry {
        commands: vec![
            descriptor("eval.test", &schema),
            descriptor("system.doctor", &json!({})),
        ],
    }
}

#[test]
fn flags_of_every_kind_become_their_json() {
    let registry = every_kind_registry();
    let line = "eval test evals/f.rrseval ckpt:c --max-new-tokens -3 --temperature=0.5 --fail-fast --runtime fast \
                --gpu 0 --gpu 1 --set-env HF_HUB_OFFLINE=1 --set-env A=b=c";
    let (command_id, arguments) = command_and_arguments(&registry, words(line)).unwrap();
    assert_eq!(command_id, "eval.test");
    let expected = json!({
        "suite": "evals/f.rrseval",
        "model": "ckpt:c",
        "max_new_tokens": -3,
        "temperature": 0.5,
        "fail_fast": true,
        "runtime": "fast",
        "gpu": [0, 1],
        "set_env": {"HF_HUB_OFFLINE": "1", "A": "b=c"}
    });
    assert_eq!(arguments, expected);
}

#[test]
fn off_flags_and_explicit_booleans() {
    let registry = every_kind_registry();
    let (_, off) = command_and_arguments(&registry, words("eval test s --no-fail-fast")).unwrap();
    assert_eq!(off["fail_fast"], false);
    let (_, explicit) = command_and_arguments(&registry, words("eval test s --fail-fast=false")).unwrap();
    assert_eq!(explicit["fail_fast"], false);
}

#[test]
fn flags_override_arguments_json_key_by_key() {
    let registry = every_kind_registry();
    let line =
        r#"eval test --arguments-json {"suite":"a","model":"m","gpu":[7],"sampling":{"top_k":5}} --suite b --gpu 2"#;
    let (_, arguments) = command_and_arguments(&registry, words(line)).unwrap();
    let expected = json!({"suite": "b", "model": "m", "gpu": [2], "sampling": {"top_k": 5}});
    assert_eq!(arguments, expected);
}

#[test]
fn an_alias_names_its_command() {
    let registry = every_kind_registry();
    let (command_id, arguments) = command_and_arguments(&registry, words("doctor")).unwrap();
    assert_eq!((command_id.as_str(), arguments), ("system.doctor", json!({})));
}

#[test]
fn arguments_wrong_before_sending_are_refused_with_rrs_c0002() {
    let registry = every_kind_registry();
    for line in [
        "eval test",
        "eval test s --runtime slow",
        "eval test s --max-new-tokens many",
        "eval test s --temperature",
        "eval test s --gpu x",
        "eval test s --set-env NOVALUE",
        "eval test s --sampling x",
        "eval test s --no-fail-fast=true",
        "eval test s --unknown 1",
        "eval test s m extra",
        "eval test --arguments-json [1]",
        "eval test --arguments-json {} --arguments-file x.json",
        r#"eval test s --arguments-json {"nope":1}"#,
    ] {
        let refusal = command_and_arguments(&registry, words(line)).unwrap_err();
        assert!(!refusal.is_empty(), "{line}");
        assert!(refusal.iter().all(|code| code == "RRS-C0002"), "{line}: {refusal:?}");
    }
}

#[test]
fn an_unknown_verb_is_rrs_c0001() {
    let registry = every_kind_registry();
    let refusal = command_and_arguments(&registry, words("eval tset s")).unwrap_err();
    assert_eq!(refusal, ["RRS-C0001"]);
}

#[test]
fn a_verb_with_no_daemon_running_starts_one() {
    let home = test_home("auto-start");
    let guard = DaemonGuard { home: home.clone() };
    // Inside the test's home, so the guard removes it too.
    let working_folder = home.join("work");
    fs::create_dir_all(&working_folder).unwrap();
    assert!(!daemon_answers(&home));

    let started = run_rastai_in(&home, &working_folder, &["system", "status", "--json"]);
    assert_eq!(started.exit_code, 0, "{started:?}");
    let status: Value = serde_json::from_str(started.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(status["app_version"], APP_VERSION);
    assert_eq!(final_line(&started), json!({"exit_code": 0, "diagnostics": []}));
    assert!(daemon_answers(&home));
    let daemons = daemon_processes(&home);
    assert_eq!(daemons.len(), 1, "one daemon runs on");
    // The daemon runs in its data folder (or the nearest one above it), never in the CLI's working folder.
    let daemon_folder = fs::read_link(format!("/proc/{}/cwd", daemons[0])).unwrap();
    assert!(daemon_folder.starts_with(&home), "{daemon_folder:?}");
    assert!(!daemon_folder.starts_with(&working_folder), "{daemon_folder:?}");

    // The next verb finds it and starts no second one.
    let again = run_rastai(&home, &["system", "status", "--no-start", "--json"]);
    assert_eq!(again.exit_code, 0, "{again:?}");
    assert_eq!(daemon_processes(&home).len(), 1);

    let stopped = run_rastai(&home, &["system", "shutdown", "--no-start"]);
    assert_eq!(stopped.exit_code, 0, "{stopped:?}");
    wait_for_no_daemon(&home);

    // Help still lists the verbs from the command cache, and starts nothing.
    assert_help_lists_system_status(&home);
    assert!(daemon_processes(&home).is_empty());
    drop(guard);
}

/// `rastai help` as text and as JSON, with no daemon running: both list `system status` from the command cache.
fn assert_help_lists_system_status(home: &Path) {
    let help = run_rastai(home, &["help"]);
    assert_eq!(help.exit_code, 0, "{help:?}");
    assert!(help.stdout.contains("rastai system status"), "{}", help.stdout);
    let help_json = run_rastai(home, &["help", "--json"]);
    assert_eq!(help_json.exit_code, 0, "{help_json:?}");
    let overview: Value = serde_json::from_str(help_json.stdout.lines().next().unwrap()).unwrap();
    let verbs: Vec<&str> = overview["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|command| command["verb"].as_str())
        .collect();
    assert!(verbs.contains(&"system status"), "{overview}");
    assert_eq!(final_line(&help_json), json!({"exit_code": 0, "diagnostics": []}));
}

/// An `owner.token` others can read stops the start (`RRS-C1008`, contracts 8.1); the CLI reports the daemon's own
/// reason from its final `--json` line and exits `EXIT_DAEMON_UNAVAILABLE`.
#[cfg(unix)]
#[test]
fn a_daemon_that_cannot_start_reports_its_reason_and_exits_3() {
    use std::os::unix::fs::PermissionsExt;

    let home = test_home("start-fails");
    let guard = DaemonGuard { home: home.clone() };
    let auth_folder = home.join("config").join("auth");
    fs::create_dir_all(&auth_folder).unwrap();
    let token_path = auth_folder.join("owner.token");
    fs::write(&token_path, format!("{}\n", "a".repeat(64))).unwrap();
    fs::set_permissions(&token_path, fs::Permissions::from_mode(0o644)).unwrap();

    let result = run_rastai(&home, &["system", "status", "--json"]);
    assert_eq!(result.exit_code, 3, "{result:?}");
    assert_eq!(final_codes(&result), ["RRS-C1008"]);
    assert!(!daemon_answers(&home));
    drop(guard);
}

fn wait_for_no_daemon(home: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !daemon_processes(home).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon didn't stop after system.shutdown"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
