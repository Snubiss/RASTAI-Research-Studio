//! A diagnostic as a JSON object, for the CLI's `--json` output and log lines (layout: rust-control-plane.md 3.2).
//! The keys are the field names of `rastai.common.v1.Diagnostic` (contracts 4.1), and every field is always present,
//! so a script reading the output never has to guess a default.

use serde_json::{Map, Value};

use crate::diagnostic::{Diagnostic, severity_name};

/// The diagnostic's seven fields; severity as its lowercase word (`error`, `warning`, `info`).
pub fn render_json(diagnostic: &Diagnostic) -> Value {
    let mut object = Map::new();
    insert_text(&mut object, "code", &diagnostic.code);
    insert_text(&mut object, "severity", severity_name(diagnostic.severity));
    insert_text(&mut object, "message", &diagnostic.message);
    insert_text(&mut object, "path", &diagnostic.path);
    insert_text(&mut object, "fix_text", &diagnostic.fix_text);
    insert_text(&mut object, "fix_command_id", &diagnostic.fix_command_id);
    // A string, as on the wire: the arguments are validated by the command they belong to, not here.
    insert_text(&mut object, "fix_arguments_json", &diagnostic.fix_arguments_json);
    Value::Object(object)
}

fn insert_text(object: &mut Map<String, Value>, key: &str, text: &str) {
    object.insert(key.to_string(), Value::String(text.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;

    #[test]
    fn every_field_is_present_with_its_wire_name() {
        let diagnostic = Diagnostic {
            code: "RRS-S3001".to_string(),
            severity: Severity::Warning,
            message: "Saving needs \"4096\" bytes.".to_string(),
            path: "a.json:1:2#/x".to_string(),
            fix_text: "Free space.".to_string(),
            fix_command_id: "storage.gc".to_string(),
            fix_arguments_json: "{\"dry_run\":true}".to_string(),
        };
        let expected = concat!(
            r#"{"code":"RRS-S3001","fix_arguments_json":"{\"dry_run\":true}","fix_command_id":"storage.gc","#,
            r#""fix_text":"Free space.","message":"Saving needs \"4096\" bytes.","path":"a.json:1:2#/x","#,
            r#""severity":"warning"}"#
        );
        assert_eq!(render_json(&diagnostic).to_string(), expected);
    }
}
