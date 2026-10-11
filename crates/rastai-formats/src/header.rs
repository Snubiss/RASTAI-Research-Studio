//! The shared header every definition document starts with (contracts 5.1, ADR-0032 decision 3), and its
//! description. A document's struct holds it as `#[serde(flatten)] pub header: SharedHeader`, and the document's
//! `describe_<type>()` starts its field list with `header_fields()`, so the header is written once.
//!
//! Optional fields read as empty when absent and are always written, so a file read and written back has every
//! header key and the description lists exactly what serde writes (tests/describe_matches_struct.rs).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::describe::types::{FieldDescription, FieldKind, OPTIONAL, REQUIRED, TypeDescription, field, list_of};

/// The name `FieldKind::Type` and `find_description` know the header by.
pub const SHARED_HEADER_TYPE_NAME: &str = "SharedHeader";

// The two header keys the validation layers read before anything else: which type, and which version of it.
pub const TYPE_KEY: &str = "type";
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// The header fields of contracts 5.1, in the order files are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedHeader {
    /// `https://schemas.<domain>/<type>/<schema_version>.json`, so editors can validate the file.
    #[serde(rename = "$schema")]
    pub schema_url: String,
    /// `rastai.model`. Named `document_type` in Rust because `type` is a keyword.
    #[serde(rename = "type")]
    pub document_type: String,
    pub schema_version: u32,
    #[serde(default)]
    pub created_with: String,
    pub id: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub derived_from: Vec<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source_url: String,
    /// `{plugin_id: semver range}`; layer 4 checks them against the enabled plugins (validate/environment.rs).
    #[serde(default)]
    pub requires: BTreeMap<String, String>,
}

/// The header as a described type of its own, for the codegen and for checking a document whose type is unknown.
pub fn describe_shared_header() -> TypeDescription {
    TypeDescription {
        name: SHARED_HEADER_TYPE_NAME.to_string(),
        schema_id: String::new(),
        doc: "The fields every Rastai document starts with.".to_string(),
        fields: header_fields(),
    }
}

/// The header's field descriptions. Every document's description starts with these.
pub fn header_fields() -> Vec<FieldDescription> {
    vec![
        field(
            "$schema",
            FieldKind::String,
            REQUIRED,
            "URL of this document's JSON Schema.",
        ),
        field(
            TYPE_KEY,
            FieldKind::String,
            REQUIRED,
            "Document type, e.g. rastai.model.",
        ),
        field(
            SCHEMA_VERSION_KEY,
            FieldKind::Integer,
            REQUIRED,
            "Version of the type's schema, from 1.",
        ),
        field(
            "created_with",
            FieldKind::String,
            OPTIONAL,
            "rastai <app version> that wrote the file.",
        ),
        field("id", FieldKind::String, REQUIRED, "Stable id, kept across edits."),
        field(
            "content_hash",
            FieldKind::String,
            OPTIONAL,
            "blake3:<64 hex> of the canonical content.",
        ),
        field(
            "derived_from",
            list_of(FieldKind::String),
            OPTIONAL,
            "<id>@<content_hash> of each parent (remix, import source).",
        ),
        field(
            "authors",
            list_of(FieldKind::String),
            OPTIONAL,
            "Free-text author names.",
        ),
        field("license", FieldKind::String, OPTIONAL, "SPDX license id."),
        field(
            "source_url",
            FieldKind::String,
            OPTIONAL,
            "Where the document came from, if anywhere.",
        ),
        field(
            "requires",
            FieldKind::Map(Box::new(FieldKind::String)),
            OPTIONAL,
            "Plugins the document needs: plugin id to semver range.",
        ),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_fields_read_as_empty_when_absent() {
        let minimal = json!({"$schema": "s", "type": "rastai.model", "schema_version": 1, "id": "model_1"});
        let header: SharedHeader = serde_json::from_value(minimal).unwrap();
        assert_eq!(header.document_type, "rastai.model");
        assert_eq!(header.license, "");
        assert!(header.authors.is_empty() && header.requires.is_empty());
    }

    #[test]
    fn json_keys_are_the_contract_names() {
        let header: SharedHeader =
            serde_json::from_value(json!({"$schema": "s", "type": "t", "schema_version": 2, "id": "i"})).unwrap();
        let written = serde_json::to_value(&header).unwrap();
        assert_eq!(written["$schema"], "s");
        assert_eq!(written["type"], "t");
        assert_eq!(written["schema_version"], 2);
    }

    #[test]
    fn description_names_the_header_and_lists_its_fields_in_order() {
        let description = describe_shared_header();
        assert_eq!(description.name, SHARED_HEADER_TYPE_NAME);
        let names: Vec<&str> = description.fields.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(&names[..3], &["$schema", "type", "schema_version"]);
        assert_eq!(names.len(), 11);
    }
}
