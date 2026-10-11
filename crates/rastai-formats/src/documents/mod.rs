//! The document types (contracts 5.2): the `Document` enum with one variant per type, the table of types the
//! validation layers read, and type detection from a file's extension.
//!
//! Every type is added by its own contract PR (M1-17 adds the step-1 documents). Until then the enum and the table
//! are empty: `read_document` reads the syntax and checks the shared header of any file, and reports its type as
//! unknown. The table is data, not a match, so the layer-2 code is the same for every type and a test can run it
//! with a type of its own.

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

/// Every document type `read_document` knows. Each type's contract PR adds its row.
pub const DOCUMENT_TYPES: &[DocumentType] = &[];

/// A document that passed layers 1 and 2, as its typed struct. Each type's contract PR adds its variant; the
/// `match` in `document_header` then fails to compile until the new variant is handled.
#[derive(Debug, Clone, PartialEq)]
pub enum Document {}

/// The shared header of any document.
pub fn document_header(document: &Document) -> &SharedHeader {
    match *document {}
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
