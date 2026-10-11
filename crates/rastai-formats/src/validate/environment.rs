//! Layer 4: what the caller's environment lacks for a document (`RRS-E3xxx`). These are warnings, never failures:
//! a document whose plugin is missing still opens, and only an action that needs the plugin fails then (ADR-0032
//! decision 9). It is a separate call because only the caller knows the environment; rastai-formats never looks at
//! the machine. The daemon runs it after `read_document` for `file.open` and `file.validate` (layout 4.4).
//!
//! The header's `requires` is checked here. The GPU and adapter checks join with the types they need (ModelSpec).

use std::collections::BTreeMap;

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};
use rastai_diag::path::{DiagnosticPath, format_path};

use crate::documents::{Document, document_header};
use crate::json::pointer::join_pointer;
use crate::semver::{parse_version, version_matches};

const CODE_PLUGIN_MISSING: &str = "RRS-E3001";
const REQUIRES_POINTER: &str = "/requires";

/// Plain facts the caller looked up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentFacts {
    /// (plugin_id, version) of every enabled plugin.
    pub enabled_plugins: Vec<(String, String)>,
    /// Host capabilities with their version, such as `designer.node@1`.
    pub host_capabilities: Vec<String>,
    pub gpu_memory_bytes: Vec<u64>,
    pub adapters: Vec<String>,
}

/// Every environment warning for a document.
pub fn check_environment(document: &Document, facts: &EnvironmentFacts) -> Vec<Diagnostic> {
    check_requirements(&document_header(document).requires, facts)
}

/// Each required plugin must be enabled at a version inside its range; otherwise `RRS-E3001` at
/// `#/requires/<plugin_id>`. A range semver.rs can't read is reported with its own code (`RRS-E1004`).
pub fn check_requirements(requires: &BTreeMap<String, String>, facts: &EnvironmentFacts) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (plugin_id, range) in requires {
        let path = format_path(&DiagnosticPath {
            json_pointer: join_pointer(REQUIRES_POINTER, plugin_id),
            ..DiagnosticPath::default()
        });
        let enabled_version = facts
            .enabled_plugins
            .iter()
            .find(|(enabled_id, _)| enabled_id == plugin_id)
            .map(|(_, version)| version.as_str());
        let values = [("plugin_id", plugin_id.as_str()), ("range", range.as_str())];
        match enabled_version.map(|version| plugin_version_matches(range, version)) {
            Some(Ok(true)) => {}
            Some(Err(code)) => diagnostics.push(diagnostic_at(code, &values, &path)),
            Some(Ok(false)) | None => diagnostics.push(diagnostic_at(CODE_PLUGIN_MISSING, &values, &path)),
        }
    }
    diagnostics
}

/// Whether an enabled plugin's version is inside the range. The range comes from the document, so a bad one is
/// the document's problem (`Err` with its code). The version comes from an installed manifest, already checked
/// when it was installed; one that still doesn't read counts as not matching.
fn plugin_version_matches(range: &str, version_text: &str) -> Result<bool, &'static str> {
    let Ok(version) = parse_version(version_text) else {
        return Ok(false);
    };
    version_matches(range, &version).map_err(|error| error.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(plugins: &[(&str, &str)]) -> EnvironmentFacts {
        EnvironmentFacts {
            enabled_plugins: plugins
                .iter()
                .map(|(id, version)| (id.to_string(), version.to_string()))
                .collect(),
            ..EnvironmentFacts::default()
        }
    }

    fn requires(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(id, range)| (id.to_string(), range.to_string()))
            .collect()
    }

    fn reported(entries: &[(&str, &str)], plugins: &[(&str, &str)]) -> Vec<(String, String)> {
        check_requirements(&requires(entries), &facts(plugins))
            .into_iter()
            .map(|item| (item.code, item.path))
            .collect()
    }

    fn pair(code: &str, path: &str) -> (String, String) {
        (code.to_string(), path.to_string())
    }

    #[test]
    fn enabled_plugins_inside_their_range_pass() {
        assert_eq!(reported(&[("psychology", "^1.2")], &[("psychology", "1.4.0")]), vec![]);
        assert_eq!(reported(&[], &[]), vec![]);
    }

    #[test]
    fn missing_or_out_of_range_plugins_are_e3001_at_their_key() {
        let expected = vec![
            pair("RRS-E3001", "#/requires/evaluation"),
            pair("RRS-E3001", "#/requires/psychology"),
        ];
        let entries = [("psychology", ">=2"), ("evaluation", "^1")];
        assert_eq!(reported(&entries, &[("psychology", "1.4.0")]), expected);
    }

    #[test]
    fn a_range_that_does_not_read_is_e1004_and_ids_are_escaped() {
        let expected = vec![pair("RRS-E1004", "#/requires/a~1b")];
        assert_eq!(reported(&[("a/b", "about one")], &[("a/b", "1.0.0")]), expected);
        // A broken installed version counts as not matching.
        assert_eq!(
            reported(&[("x", "^1")], &[("x", "one")]),
            vec![pair("RRS-E3001", "#/requires/x")]
        );
    }
}
