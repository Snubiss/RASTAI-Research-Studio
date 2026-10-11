//! Conversions between the in-process twins and their proto messages (layout rust-control-plane.md 5.2): rastai-diag's
//! `Diagnostic` and `rastai.common.v1.Diagnostic` both ways, and a `CommandRow` to `rastai.commands.v1.CommandDescriptor`.
//!
//! Each conversion takes its input apart with a full pattern (no `..`), so a field added to either side fails to
//! compile here until it is converted. `tests/wire_round_trip.rs` checks the field lists against the .proto files.

use rastai_diag::diagnostic::{Diagnostic, Severity};
use rastai_proto::commands::v1::CommandDescriptor;
use rastai_proto::common::v1::{Diagnostic as ProtoDiagnostic, Severity as ProtoSeverity};

use crate::commands::descriptor::CommandRow;
use crate::scopes::scope_name;

/// The registry's descriptor for a core row. The registration fields stay empty: a core command has no
/// `plugin_id`, and rastai-server fills the type names and schemas from the argument descriptions.
pub fn descriptor_from_row(row: &CommandRow) -> CommandDescriptor {
    let CommandRow {
        command_id,
        title,
        summary,
        section,
        scope,
        kind,
        stability,
        undoable,
        basic,
        default_shortcut,
        experimental,
        since_version,
    } = row;
    CommandDescriptor {
        command_id: command_id.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        section: section.to_string(),
        scope: scope_name(scope),
        undoable: *undoable,
        kind: *kind as i32,
        stability: *stability as i32,
        arguments_type: String::new(),
        result_type: String::new(),
        arguments_schema_json: String::new(),
        result_schema_json: String::new(),
        default_shortcut: default_shortcut.to_string(),
        basic: *basic,
        plugin_id: String::new(),
        experimental: *experimental,
        since_version: since_version.to_string(),
    }
}

/// The proto message for a diagnostic, field for field.
pub fn diagnostic_to_proto(diagnostic: &Diagnostic) -> ProtoDiagnostic {
    let Diagnostic {
        code,
        severity,
        message,
        path,
        fix_text,
        fix_command_id,
        fix_arguments_json,
    } = diagnostic;
    ProtoDiagnostic {
        code: code.clone(),
        severity: severity_to_proto(*severity) as i32,
        message: message.clone(),
        path: path.clone(),
        fix_text: fix_text.clone(),
        fix_command_id: fix_command_id.clone(),
        fix_arguments_json: fix_arguments_json.clone(),
    }
}

/// The diagnostic a proto message carries. An unspecified or unknown severity reads as an error: a problem from a
/// newer or broken sender must not be hidden as a warning.
pub fn diagnostic_from_proto(message: &ProtoDiagnostic) -> Diagnostic {
    let ProtoDiagnostic {
        code,
        severity,
        message: text,
        path,
        fix_text,
        fix_command_id,
        fix_arguments_json,
    } = message;
    Diagnostic {
        code: code.clone(),
        severity: severity_from_proto(*severity),
        message: text.clone(),
        path: path.clone(),
        fix_text: fix_text.clone(),
        fix_command_id: fix_command_id.clone(),
        fix_arguments_json: fix_arguments_json.clone(),
    }
}

fn severity_to_proto(severity: Severity) -> ProtoSeverity {
    match severity {
        Severity::Error => ProtoSeverity::Error,
        Severity::Warning => ProtoSeverity::Warning,
        Severity::Info => ProtoSeverity::Info,
    }
}

fn severity_from_proto(severity: i32) -> Severity {
    match ProtoSeverity::try_from(severity) {
        Ok(ProtoSeverity::Warning) => Severity::Warning,
        Ok(ProtoSeverity::Info) => Severity::Info,
        Ok(ProtoSeverity::Error) | Ok(ProtoSeverity::Unspecified) | Err(_) => Severity::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severities_map_both_ways() {
        for severity in [Severity::Error, Severity::Warning, Severity::Info] {
            assert_eq!(severity_from_proto(severity_to_proto(severity) as i32), severity);
        }
    }

    #[test]
    fn unknown_severities_read_as_errors() {
        assert_eq!(severity_from_proto(ProtoSeverity::Unspecified as i32), Severity::Error);
        assert_eq!(severity_from_proto(42), Severity::Error);
        assert_eq!(severity_from_proto(-1), Severity::Error);
    }
}
