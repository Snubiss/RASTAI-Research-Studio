//! The document types (contracts 5.2): the `Document` enum with one variant per type, the table of types the
//! validation layers read, type detection from a file's extension, and building the typed struct from a checked
//! value.
//!
//! The table is data, not a match, so the layer-2 code is the same for every type and a test can run it with a type
//! of its own. A new type adds its module here, its row in `DOCUMENT_TYPES`, its `Document` variant and an arm in
//! `document_from_value`, `document_header` and `document_header_mut` (crates/rastai-formats/CLAUDE.md).

pub mod lock;
pub mod model;
pub mod model_ops;
pub mod plans;
pub mod plugin_manifest;
pub mod project;
pub mod resume;
pub mod run_record;
pub mod settings;
pub mod theme;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::documents::lock::{LOCK_FILE_NAME, LOCK_SCHEMA_VERSION, LOCK_TYPE_NAME, LockFile};
use crate::documents::model::{
    MODEL_FILE_EXTENSION, MODEL_SCHEMA_VERSION, MODEL_SPEC_NAME, MODEL_TYPE_NAME, ModelSpec,
};
use crate::documents::plugin_manifest::{
    PLUGIN_MANIFEST_NAME, PLUGIN_MANIFEST_SCHEMA_VERSION, PLUGIN_MANIFEST_TYPE_NAME, PluginManifest,
};
use crate::documents::project::{PROJECT_NAME, PROJECT_SCHEMA_VERSION, PROJECT_TYPE_NAME, Project};
use crate::documents::resume::{RESUME_SCHEMA_VERSION, RESUME_STATE_NAME, RESUME_TYPE_NAME, ResumeState};
use crate::documents::run_record::{RUN_RECORD_NAME, RUN_RECORD_SCHEMA_VERSION, RUN_RECORD_TYPE_NAME, RunRecord};
use crate::documents::settings::{
    SETTINGS_DOCUMENT_NAME, SETTINGS_SCHEMA_VERSION, SETTINGS_TYPE_NAME, SettingsDocument,
};
use crate::documents::theme::{
    THEME_DOCUMENT_NAME, THEME_FILE_EXTENSION, THEME_SCHEMA_VERSION, THEME_TYPE_NAME, ThemeDocument,
};
use crate::header::SharedHeader;

/// One document type, as the validation layers need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentType {
    /// The header's `type` value, e.g. `rastai.model`.
    pub type_name: &'static str,
    /// The current schema version; files with an older one are migrated, newer ones open read-only.
    pub schema_version: u32,
    /// Lowercase endings that name this type, such as `.rrsmodel` or `.theme.json`. Empty for documents found by
    /// their place rather than their name (`run.json`, `settings.json`): those are known by their `type` field.
    pub file_extensions: &'static [&'static str],
    /// The `TypeDescription` that describes the whole document, header fields first (describe/registry.rs).
    pub description_name: &'static str,
}

/// Every document type `read_document` knows. Each type's contract PR adds its row. Documents found by their place
/// in a folder (`run.json`, `settings.json`, `rastai.lock`) have no extension and are known by their `type` field.
pub const DOCUMENT_TYPES: &[DocumentType] = &[
    DocumentType {
        type_name: RUN_RECORD_TYPE_NAME,
        schema_version: RUN_RECORD_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: RUN_RECORD_NAME,
    },
    DocumentType {
        type_name: RESUME_TYPE_NAME,
        schema_version: RESUME_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: RESUME_STATE_NAME,
    },
    DocumentType {
        type_name: SETTINGS_TYPE_NAME,
        schema_version: SETTINGS_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: SETTINGS_DOCUMENT_NAME,
    },
    DocumentType {
        type_name: PROJECT_TYPE_NAME,
        schema_version: PROJECT_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: PROJECT_NAME,
    },
    DocumentType {
        type_name: LOCK_TYPE_NAME,
        schema_version: LOCK_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: LOCK_FILE_NAME,
    },
    DocumentType {
        type_name: PLUGIN_MANIFEST_TYPE_NAME,
        schema_version: PLUGIN_MANIFEST_SCHEMA_VERSION,
        file_extensions: &[],
        description_name: PLUGIN_MANIFEST_NAME,
    },
    DocumentType {
        type_name: THEME_TYPE_NAME,
        schema_version: THEME_SCHEMA_VERSION,
        file_extensions: &[THEME_FILE_EXTENSION],
        description_name: THEME_DOCUMENT_NAME,
    },
    DocumentType {
        type_name: MODEL_TYPE_NAME,
        schema_version: MODEL_SCHEMA_VERSION,
        file_extensions: &[MODEL_FILE_EXTENSION],
        description_name: MODEL_SPEC_NAME,
    },
];

/// A document that passed layers 1 and 2, as its typed struct. Each type's contract PR adds its variant; the
/// `match` in `document_header` then fails to compile until the new variant is handled.
// The variants differ in size (a theme holds 41 color roles), but a document is read once and handed on, never
// kept in large numbers, so boxing each variant would only add a dereference at every use.
// `untagged` writes a document as its own struct, with no variant name around it, so the JSON is the file's JSON.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Document {
    RunRecord(RunRecord),
    Resume(ResumeState),
    Settings(SettingsDocument),
    Project(Project),
    Lock(LockFile),
    PluginManifest(PluginManifest),
    Theme(ThemeDocument),
    Model(ModelSpec),
}

/// The shared header of any document.
pub fn document_header(document: &Document) -> &SharedHeader {
    match document {
        Document::RunRecord(run_record) => &run_record.header,
        Document::Resume(resume) => &resume.header,
        Document::Settings(settings) => &settings.header,
        Document::Project(project) => &project.header,
        Document::Lock(lock) => &lock.header,
        Document::PluginManifest(manifest) => &manifest.header,
        Document::Theme(theme) => &theme.header,
        Document::Model(model) => &model.header,
    }
}

/// The shared header of any document, to change. `write_document` fills `content_hash` through it.
pub fn document_header_mut(document: &mut Document) -> &mut SharedHeader {
    match document {
        Document::RunRecord(run_record) => &mut run_record.header,
        Document::Resume(resume) => &mut resume.header,
        Document::Settings(settings) => &mut settings.header,
        Document::Project(project) => &mut project.header,
        Document::Lock(lock) => &mut lock.header,
        Document::PluginManifest(manifest) => &mut manifest.header,
        Document::Theme(theme) => &mut theme.header,
        Document::Model(model) => &mut model.header,
    }
}

/// The typed document for a value that passed layers 1 and 2. The error is serde's reason, which can only happen
/// when a description and its struct disagree (tests/describe_matches_struct.rs keeps them equal); None when no
/// type has that name.
pub fn document_from_value(type_name: &str, value: Value) -> Option<Result<Document, String>> {
    let document = match type_name {
        RUN_RECORD_TYPE_NAME => typed(value).map(Document::RunRecord),
        RESUME_TYPE_NAME => typed(value).map(Document::Resume),
        SETTINGS_TYPE_NAME => typed(value).map(Document::Settings),
        PROJECT_TYPE_NAME => typed(value).map(Document::Project),
        LOCK_TYPE_NAME => typed(value).map(Document::Lock),
        PLUGIN_MANIFEST_TYPE_NAME => typed(value).map(Document::PluginManifest),
        THEME_TYPE_NAME => typed(value).map(Document::Theme),
        MODEL_TYPE_NAME => typed(value).map(Document::Model),
        _ => return None,
    };
    Some(document)
}

fn typed<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// The row for a `type` value, or None when no type has that name.
pub fn find_document_type<'a>(document_types: &'a [DocumentType], type_name: &str) -> Option<&'a DocumentType> {
    document_types
        .iter()
        .find(|document_type| document_type.type_name == type_name)
}

/// The row whose extension the file name ends with, or None. Extensions compare without case, because
/// `Model.RRSMODEL` from a case-insensitive file system is still a ModelSpec.
pub fn document_type_for_file<'a>(document_types: &'a [DocumentType], file_name: &str) -> Option<&'a DocumentType> {
    let lowercase_name = file_name.to_ascii_lowercase();
    document_types.iter().find(|document_type| {
        let extensions = document_type.file_extensions;
        extensions.iter().any(|extension| lowercase_name.ends_with(extension))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TYPES: &[DocumentType] = &[
        DocumentType {
            type_name: "test.sample",
            schema_version: 1,
            file_extensions: &[".sample"],
            description_name: "Sample",
        },
        DocumentType {
            type_name: "test.theme",
            schema_version: 2,
            file_extensions: &[".theme.json"],
            description_name: "Theme",
        },
        DocumentType {
            type_name: "test.settings",
            schema_version: 1,
            file_extensions: &[],
            description_name: "Settings",
        },
    ];

    #[test]
    fn types_are_found_by_name() {
        let found = find_document_type(SAMPLE_TYPES, "test.theme").map(|row| row.schema_version);
        assert_eq!(found, Some(2));
        assert!(find_document_type(SAMPLE_TYPES, "test.missing").is_none());
    }

    #[test]
    fn every_built_in_type_has_a_unique_name_and_a_description() {
        let all_types = crate::describe::registry::describe_all_types();
        for row in DOCUMENT_TYPES {
            let same_name = DOCUMENT_TYPES
                .iter()
                .filter(|other| other.type_name == row.type_name)
                .count();
            assert_eq!(same_name, 1, "{}", row.type_name);
            let described = crate::describe::registry::find_description(&all_types, row.description_name);
            assert!(described.is_some(), "{} has no description", row.type_name);
        }
    }

    #[test]
    fn a_value_of_an_unknown_type_builds_nothing() {
        assert!(document_from_value("rastai.no_such_type", Value::Null).is_none());
        let refused = document_from_value(PROJECT_TYPE_NAME, Value::Null);
        assert!(matches!(refused, Some(Err(_))));
    }

    #[test]
    fn types_are_found_by_extension_without_case() {
        let type_of = |file_name: &str| document_type_for_file(SAMPLE_TYPES, file_name).map(|row| row.type_name);
        assert_eq!(type_of("designs/a.sample"), Some("test.sample"));
        assert_eq!(type_of("A.SAMPLE"), Some("test.sample"));
        assert_eq!(type_of("themes/night.theme.json"), Some("test.theme"));
        // A plain .json file is known only by its `type` field.
        assert_eq!(type_of("settings.json"), None);
        assert_eq!(type_of("sample"), None);
    }
}
