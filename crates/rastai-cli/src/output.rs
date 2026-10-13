//! The only file that writes to stdout and stderr (layout: rust-control-plane.md 9.7; the workspace denies
//! `print_stdout` elsewhere). Text mode prints results on stdout and diagnostics on stderr, rendered by rastai-diag.
//! `--json` prints one JSON object per line on stdout: results, events while following, and a final
//! `{"exit_code": n, "diagnostics": [...]}`.
//!
//! A write that fails (a closed pipe, `rastai ... | head`) is dropped: the exit code still tells the outcome, and
//! there is nowhere left to report it.

use std::io::{IsTerminal, Write};

use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_diag::render_json::render_json;
use rastai_diag::render_text::{RenderStyle, render_text};
use serde_json::{Value, json};

use crate::arguments::GlobalFlags;

const EXIT_CODE_KEY: &str = "exit_code";
const DIAGNOSTICS_KEY: &str = "diagnostics";
const FILE_KEY: &str = "file";

/// How this invocation prints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputSettings {
    pub json: bool,
    /// Also print info diagnostics, which text mode leaves out otherwise.
    pub verbose: bool,
}

pub fn output_settings(global: &GlobalFlags) -> OutputSettings {
    OutputSettings {
        json: global.json,
        verbose: global.verbose,
    }
}

/// A command's result: one compact line with `--json`, indented JSON otherwise.
pub fn print_result(settings: OutputSettings, result: &Value) {
    if settings.json {
        write_stdout(&result.to_string());
        return;
    }
    let text = serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    write_stdout(&text);
}

/// One line that is the same information in both modes: `text` in text mode, `value` with `--json`.
pub fn print_line(settings: OutputSettings, text: &str, value: &Value) {
    if settings.json {
        write_stdout(&value.to_string());
    } else {
        write_stdout(text);
    }
}

/// Plain text on stdout, in text mode only (help, a job's log lines).
pub fn print_text(settings: OutputSettings, text: &str) {
    if !settings.json {
        write_stdout(text);
    }
}

/// What `rastai validate` found in one file: each diagnostic with its source line in text mode, or one
/// `{"file", "diagnostics"}` line with `--json`.
pub fn print_document_report(settings: OutputSettings, file: &str, diagnostics: &[Diagnostic], source_text: &str) {
    if settings.json {
        let rendered: Vec<Value> = diagnostics.iter().map(render_json).collect();
        write_stdout(&json!({ FILE_KEY: file, DIAGNOSTICS_KEY: rendered }).to_string());
        return;
    }
    let style = style_for(std::io::stdout().is_terminal());
    for diagnostic in diagnostics {
        write_stdout(&render_text(diagnostic, Some(source_text), style));
    }
}

/// The end of every invocation: the diagnostics on stderr in text mode, or the final JSON line.
pub fn print_final(settings: OutputSettings, exit_code: u8, diagnostics: &[Diagnostic]) {
    if settings.json {
        let rendered: Vec<Value> = diagnostics.iter().map(render_json).collect();
        write_stdout(&json!({ EXIT_CODE_KEY: exit_code, DIAGNOSTICS_KEY: rendered }).to_string());
        return;
    }
    let style = style_for(std::io::stderr().is_terminal());
    for diagnostic in diagnostics {
        if diagnostic.severity == Severity::Info && !settings.verbose {
            continue;
        }
        write_stderr(&render_text(diagnostic, None, style));
    }
}

/// Colors only on a terminal, so a file or a pipe gets plain text.
fn style_for(is_terminal: bool) -> RenderStyle {
    if is_terminal {
        RenderStyle::Color
    } else {
        RenderStyle::Plain
    }
}

fn write_stdout(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", text.trim_end_matches('\n'));
    let _ = stdout.flush();
}

fn write_stderr(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{}", text.trim_end_matches('\n'));
}
