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
use crate::semver::{Version, parse_version, version_matches};

const CODE_PLUGIN_MISSING: &str = "RRS-E3001";
const REQUIRES_POINTER: &str = "/requires";
/// What RRS-E1004's message says a requirement must be when its range doesn't read.
const RANGE_EXPECTED: &str = "a version range, such as ^1.2";
// Any release does to find out whether a range reads; the answer to the match itself is thrown away.
const RANGE_PROBE_VERSION: Version = Version {
    major: 0,
    minor: 0,
    patch: 0,
    pre_release: String::new(),
};

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
/// `#/requires/<plugin_id>`. A range semver.rs can't read is reported with its own code (`RRS-E1004`) whether the
/// plugin is enabled or not, so the same file gets the same diagnostic on every machine.
///
/// The paths carry the JSON pointer only, with no file name or position: the layout's `check_environment` takes a
/// `Document`, which no longer knows its file. Callers that show these to people put their file name in front.
pub fn check_requirements(requires: &BTreeMap<String, String>, facts: &EnvironmentFacts) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (plugin_id, range) in requires {
        let path = format_path(&DiagnosticPath {
            json_pointer: join_pointer(REQUIRES_POINTER, plugin_id),
            ..DiagnosticPath::default()
        });
        // `found` and `expected` are the values RRS-E1004's message names when the range doesn't read.
        let values = [
            ("plugin_id", plugin_id.as_str()),
            ("range", range.as_str()),
            ("found", range.as_str()),
            ("expected", RANGE_EXPECTED),
        ];
        if let Some(code) = range_problem(range) {
            diagnostics.push(diagnostic_at(code, &values, &path));
        } else if !plugin_is_satisfied(plugin_id, range, facts) {
            diagnostics.push(diagnostic_at(CODE_PLUGIN_MISSING, &values, &path));
        }
    }
    diagnostics
}

/// The code for a range that doesn't read, or `None` for a good one. The range comes from the document, so a bad
/// one is the document's problem. Matching any version tells: semver.rs reads the whole range before comparing.
fn range_problem(range: &str) -> Option<&'static str> {
    version_matches(range, &RANGE_PROBE_VERSION)
        .err()
        .map(|error| error.code())
}

/// Whether the plugin is enabled at a version inside a range already known to read. The version comes from an
/// installed manifest, already checked when it was installed; one that still doesn't read counts as not matching.
fn plugin_is_satisfied(plugin_id: &str, range: &str, facts: &EnvironmentFacts) -> bool {
    let Some((_, version_text)) = facts
        .enabled_plugins
        .iter()
        .find(|(enabled_id, _)| enabled_id == plugin_id)
    else {
        return false;
    };
    let Ok(version) = parse_version(version_text) else {
        return false;
    };
    version_matches(range, &version).unwrap_or(false)
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
        // The same bad range gives the same code when the plugin isn't installed at all.
        assert_eq!(reported(&[("a/b", "about one")], &[]), expected);
        // A broken installed version counts as not matching.
        assert_eq!(
            reported(&[("x", "^1")], &[("x", "one")]),
            vec![pair("RRS-E3001", "#/requires/x")]
        );
    }

    /// A placeholder without a value stays visible as `{name}` (rastai-diag), so a message with a `{` left in it is
    /// missing a value. Only the braces are checked, not the catalog's wording.
    #[test]
    fn both_messages_have_every_placeholder_filled() {
        let entries = [("a", "about one"), ("b", "^1")];
        let diagnostics = check_requirements(&requires(&entries), &facts(&[]));
        assert_eq!(diagnostics.len(), 2);
        for diagnostic in diagnostics {
            assert!(
                !diagnostic.message.contains('{'),
                "{}: {}",
                diagnostic.code,
                diagnostic.message
            );
        }
    }
}
