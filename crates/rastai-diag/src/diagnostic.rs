//! The `Diagnostic` type every crate returns, and the functions that build one from the catalog
//! (layout: rust-control-plane.md section 3.2).

use crate::catalog::{CatalogEntry, fill_placeholders, find_entry};
use crate::logger::log_warn;

/// What a diagnostic says when its code isn't in the catalog. The codegen's source scan (layout 4.9) makes this
/// unreachable for first-party code; it stays as a safety net, so a typo shows up as a coded line, not a panic.
const UNKNOWN_CODE_MESSAGE: &str = "unknown diagnostic code";

/// How bad a diagnostic is. Mirrors `rastai.common.v1.Diagnostic.severity` (contracts 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// The in-process twin of `rastai.common.v1.Diagnostic`. Same fields, same meaning.
/// rastai-core/src/wire.rs converts between the two; a test there checks the field lists match (layout 5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// "RRS-E2004", or "PLG-<plugin_id>-0001" for third-party plugins.
    pub code: String,
    pub severity: Severity,
    /// The catalog message with its placeholders filled.
    pub message: String,
    /// Where the problem is, in the path.rs grammar; empty when the problem has no location.
    pub path: String,
    pub fix_text: String,
    /// The command a fix button runs; empty when there is none.
    pub fix_command_id: String,
    pub fix_arguments_json: String,
}

/// Builds a diagnostic from the catalog. Severity, message and fix come from the entry,
/// so call sites never write user-facing text.
pub fn diagnostic(code: &str, values: &[(&str, &str)]) -> Diagnostic {
    diagnostic_at(code, values, "")
}

/// Same as `diagnostic`, with a location (see path.rs for the grammar and `format_path` to build one).
pub fn diagnostic_at(code: &str, values: &[(&str, &str)], path: &str) -> Diagnostic {
    match find_entry(code) {
        Some(entry) => diagnostic_from_entry(entry, values, path),
        None => unknown_code_diagnostic(code, path),
    }
}

/// True when any diagnostic is an error. Warnings and info never stop an operation.
pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|item| item.severity == Severity::Error)
}

/// The lowercase word renderers, JSON output and log lines use for a severity.
pub(crate) fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn diagnostic_from_entry(entry: &CatalogEntry, values: &[(&str, &str)], path: &str) -> Diagnostic {
    Diagnostic {
        code: entry.code.to_string(),
        severity: entry.severity,
        message: fill_placeholders(entry.message, values),
        path: path.to_string(),
        // Fix text may name the same values as the message ("free {needed_bytes} bytes").
        fix_text: fill_placeholders(entry.fix_text, values),
        fix_command_id: entry.fix_command_id.to_string(),
        fix_arguments_json: String::new(),
    }
}

fn unknown_code_diagnostic(code: &str, path: &str) -> Diagnostic {
    log_warn(UNKNOWN_CODE_MESSAGE, &[("code", code)]);
    Diagnostic {
        code: code.to_string(),
        severity: Severity::Error,
        message: UNKNOWN_CODE_MESSAGE.to_string(),
        path: path.to_string(),
        fix_text: String::new(),
        fix_command_id: String::new(),
        fix_arguments_json: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A stand-in entry: the real catalog arrives with M1-12 and M1-13, and these tests must not depend on its wording.
    const DISK_FULL: CatalogEntry = CatalogEntry {
        code: "RRS-S3001",
        severity: Severity::Error,
        title: "Not enough free disk space",
        message: "{operation} needs {needed_bytes} bytes on {volume}.",
        basic_text: "Your disk is almost full.",
        fix_text: "Free {needed_bytes} bytes.",
        fix_command_id: "storage.gc",
        docs_anchor: "storage-disk-full",
        retired: false,
    };

    // A code that no catalog will ever hold, so the test keeps meaning the same thing after M1-13.
    const MISSING_CODE: &str = "RRS-E9999";

    fn sample(severity: Severity) -> Diagnostic {
        Diagnostic {
            code: "RRS-E1001".to_string(),
            severity,
            message: String::new(),
            path: String::new(),
            fix_text: String::new(),
            fix_command_id: String::new(),
            fix_arguments_json: String::new(),
        }
    }

    #[test]
    fn entry_fills_message_and_fix_and_copies_the_rest() {
        let values = [("operation", "Saving"), ("needed_bytes", "4096"), ("volume", "/home")];
        let built = diagnostic_from_entry(&DISK_FULL, &values, "a.rrsmodel:1:2");
        assert_eq!(built.code, "RRS-S3001");
        assert_eq!(built.severity, Severity::Error);
        assert_eq!(built.message, "Saving needs 4096 bytes on /home.");
        assert_eq!(built.fix_text, "Free 4096 bytes.");
        assert_eq!(built.fix_command_id, "storage.gc");
        assert_eq!(built.path, "a.rrsmodel:1:2");
        assert_eq!(built.fix_arguments_json, "");
    }

    #[test]
    fn unknown_code_is_an_error_that_keeps_its_code() {
        let built = diagnostic_at(MISSING_CODE, &[("name", "x")], "#/inputs");
        assert_eq!(built.code, MISSING_CODE);
        assert_eq!(built.severity, Severity::Error);
        assert_eq!(built.message, UNKNOWN_CODE_MESSAGE);
        assert_eq!(built.path, "#/inputs");
        assert_eq!(diagnostic(MISSING_CODE, &[]).path, "");
    }

    #[test]
    fn has_errors_ignores_warnings_and_info() {
        assert!(!has_errors(&[]));
        assert!(!has_errors(&[sample(Severity::Warning), sample(Severity::Info)]));
        assert!(has_errors(&[sample(Severity::Info), sample(Severity::Error)]));
    }
}
