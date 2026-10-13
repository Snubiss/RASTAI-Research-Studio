//! The `rastai` binary (layout: rust-control-plane.md 9.2): parse the command line, choose the mode, run it, report
//! and exit with the code of the table in exit_codes.rs. A tokio runtime is started only for the modes that talk to
//! a daemon or are one; `validate` and `version` run without.

use std::process::ExitCode;

use rastai_cli::arguments::{GlobalFlags, ParsedArguments, parse_command_line};
use rastai_cli::daemon_client::{data_paths, ensure_daemon_running};
use rastai_cli::error::{CliError, not_available_yet};
use rastai_cli::exit_codes::EXIT_OK;
use rastai_cli::help::run_help;
use rastai_cli::local_commands::run_local_command;
use rastai_cli::mcp_bridge::run_mcp_bridge;
use rastai_cli::modes::{Mode, choose_mode};
use rastai_cli::open_ui::run_open;
use rastai_cli::output::{OutputSettings, output_settings, print_final};
use rastai_cli::serve::run_serve;
use rastai_cli::verbs::{outcome_diagnostics, outcome_exit_code, run_verb};
use rastai_diag::diagnostic::Diagnostic;

// The words of the modes whose task hasn't landed, for their `RRS-C0010`.
const NODE_SERVE_WORDS: &str = "node serve";
const NODE_INIT_WORDS: &str = "node init";
const NODE_CHECK_UPDATE_WORDS: &str = "node check-update";
const TUNNEL_WORDS: &str = "tunnel";

/// How an invocation ended: its exit code and what to report.
struct Outcome {
    exit_code: u8,
    diagnostics: Vec<Diagnostic>,
}

fn main() -> ExitCode {
    let arguments = match parse_command_line(command_line_words()) {
        Ok(arguments) => arguments,
        Err(error) => return report_and_exit(output_settings(&GlobalFlags::default()), outcome_of_error(error)),
    };
    let settings = report_settings(&arguments);
    let outcome = match choose_mode(&arguments) {
        Ok(mode) => run_mode(mode, &arguments),
        Err(error) => outcome_of_error(error),
    };
    report_and_exit(settings, outcome)
}

/// The words after the program name. A word that isn't valid Unicode keeps its readable part.
fn command_line_words() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|word| word.to_string_lossy().into_owned())
        .collect()
}

/// How the end is reported. `rastai mcp`'s stdout is the MCP channel, so its report always goes to stderr as text.
fn report_settings(arguments: &ParsedArguments) -> OutputSettings {
    let mut settings = output_settings(&arguments.global);
    if matches!(choose_mode(arguments), Ok(Mode::McpBridge)) {
        settings.json = false;
    }
    settings
}

fn run_mode(mode: Mode, arguments: &ParsedArguments) -> Outcome {
    let settings = output_settings(&arguments.global);
    match mode {
        Mode::Local(command) => match run_local_command(&command, settings) {
            Ok(exit_code) => Outcome {
                exit_code,
                diagnostics: Vec::new(),
            },
            Err(error) => outcome_of_error(error),
        },
        Mode::NodeServe => outcome_of_error(not_available_yet(NODE_SERVE_WORDS)),
        Mode::NodeInit => outcome_of_error(not_available_yet(NODE_INIT_WORDS)),
        Mode::NodeCheckUpdate => outcome_of_error(not_available_yet(NODE_CHECK_UPDATE_WORDS)),
        Mode::Tunnel => outcome_of_error(not_available_yet(TUNNEL_WORDS)),
        other => run_async_mode(other, arguments, settings),
    }
}

/// The modes that need the runtime: every one that talks to a daemon, and the daemon itself.
fn run_async_mode(mode: Mode, arguments: &ParsedArguments, settings: OutputSettings) -> Outcome {
    let paths = match data_paths() {
        Ok(paths) => paths,
        Err(error) => return outcome_of_error(error),
    };
    let global = &arguments.global;
    let runtime = new_runtime();
    let result: Result<Outcome, CliError> = runtime.block_on(async {
        match mode {
            Mode::Serve(flags) => run_serve(&paths, &flags, &global.set)
                .await
                .map(|()| ok_outcome(Vec::new())),
            Mode::McpBridge => run_mcp_bridge(&paths, global.no_start).await.map(ok_outcome),
            Mode::OpenUi(flags) => run_open(&paths, &flags, global.no_start, settings)
                .await
                .map(ok_outcome),
            Mode::Help(words) => run_help(&paths, &words, settings)
                .await
                .map(|()| ok_outcome(Vec::new())),
            Mode::Verb(words) => {
                let client = ensure_daemon_running(&paths, global.no_start).await?;
                let outcome = run_verb(&client, &paths, &words, arguments).await?;
                let mut diagnostics = client.warnings.clone();
                diagnostics.extend(outcome_diagnostics(&outcome));
                Ok(Outcome {
                    exit_code: outcome_exit_code(&outcome),
                    diagnostics,
                })
            }
            other => Err(not_available_yet(&format!("{other:?}"))),
        }
    });
    // A daemon's background tasks (an open stream, a pending browser start) end with the process, not here.
    runtime.shutdown_background();
    result.unwrap_or_else(outcome_of_error)
}

/// The multi-threaded runtime the daemon expects (rastai-server's tests run it the same way).
#[allow(clippy::expect_used)]
fn new_runtime() -> tokio::runtime::Runtime {
    // Building it fails only when the operating system can't give this process a thread or an event queue, and then
    // nothing else could run either; there is no outcome to report.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the operating system refused the threads and event queue the CLI needs")
}

fn ok_outcome(warnings: Vec<Diagnostic>) -> Outcome {
    Outcome {
        exit_code: EXIT_OK,
        diagnostics: warnings,
    }
}

fn outcome_of_error(error: CliError) -> Outcome {
    Outcome {
        exit_code: error.exit_code(),
        diagnostics: error.diagnostics().to_vec(),
    }
}

fn report_and_exit(settings: OutputSettings, outcome: Outcome) -> ExitCode {
    print_final(settings, outcome.exit_code, &outcome.diagnostics);
    ExitCode::from(outcome.exit_code)
}
