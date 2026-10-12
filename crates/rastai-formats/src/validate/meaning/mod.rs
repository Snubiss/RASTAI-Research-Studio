//! Layer 3: what a well-formed document means (`RRS-E2xxx`, and the plugin codes for a manifest). It runs on the
//! typed document, after layers 1 and 2 passed, and never blocks loading: the document opens with its errors shown,
//! and the actions that need a sound document (compile, train, load a plugin) refuse it (layout 4.4).
//!
//! One arm per document type. A type with no meaning rules yet has an empty arm, so a new type can't be forgotten.

pub mod model_graph;
pub mod model_shapes;
pub mod plugin_manifest;
pub mod project;
pub mod theme;

use rastai_diag::diagnostic::{Diagnostic, diagnostic_at};

use crate::documents::Document;
use crate::json::positions::{PositionMap, diagnostic_path};
use crate::validate::meaning::model_graph::check_model;
use crate::validate::meaning::plugin_manifest::check_plugin_manifest;
use crate::validate::meaning::project::check_project;
use crate::validate::meaning::theme::check_theme;

/// What a finding shows for a value that is empty or only spaces, so the sentence doesn't start with a blank:
/// "(blank) isn't a plugin id ...".
const BLANK_VALUE: &str = "(blank)";

/// Where a document came from, so each finding can name its file, line and column.
pub struct MeaningSource<'a> {
    pub file_name: &'a str,
    pub positions: &'a PositionMap,
}

/// Every meaning problem of a document, in the order its fields are written.
pub fn check_meaning(document: &Document, source: &MeaningSource) -> Vec<Diagnostic> {
    match document {
        Document::PluginManifest(manifest) => check_plugin_manifest(manifest, source),
        Document::Project(project) => check_project(project, source),
        Document::Theme(theme) => check_theme(theme, source),
        Document::Model(model) => check_model(model, source),
        // Rastai writes these itself; their values have no rules beyond their structure yet.
        Document::RunRecord(_) | Document::Resume(_) | Document::Settings(_) | Document::Lock(_) => Vec::new(),
    }
}

/// A value as a finding shows it: the value itself, or `(blank)` when it is empty or only spaces.
pub fn shown_value(value: &str) -> &str {
    if value.trim().is_empty() {
        return BLANK_VALUE;
    }
    value
}

/// A coded diagnostic at a JSON pointer of the document being checked.
pub fn meaning_diagnostic(code: &str, values: &[(&str, &str)], source: &MeaningSource, pointer: &str) -> Diagnostic {
    let path = diagnostic_path(source.file_name, source.positions, pointer);
    diagnostic_at(code, values, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_value_is_shown_as_a_word() {
        assert_eq!(shown_value(""), "(blank)");
        assert_eq!(shown_value("   "), "(blank)");
        assert_eq!(shown_value("acme"), "acme");
        assert_eq!(shown_value(" acme "), " acme ");
    }
}
