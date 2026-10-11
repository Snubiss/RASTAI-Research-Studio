//! The JSON Schema output (layout rust-control-plane.md 2.7 and 4.9):
//! - `schemas/<type>/<version>.json`, one per document type,
//! - `schemas/commands/rastai.command.<command_id>/1.json`, one per core command: the arguments and result as two
//!   `$defs`, plus `x-rastai-stability` and `x-rastai-since-version` from the command's row,
//! - `schemas/index.json`, every schema id with its file and content hash.
//!
//! The schemas themselves come from `rastai_formats::describe::json_schema`, the same code MCP tool lists and
//! `ListCommands` use at run time, so a file here and a schema the daemon serves never differ. Files are written
//! with the stable pretty writer, so an unchanged schema keeps its bytes.

use std::path::Path;

use rastai_formats::describe::json_schema::json_schema_for;
use rastai_formats::describe::registry::find_description;
use rastai_formats::describe::types::TypeDescription;
use rastai_formats::documents::DocumentType;
use rastai_formats::hashing::content_hash_of_value;
use rastai_formats::json::pretty::pretty_json;
use serde_json::{Map, Value, json};

use crate::command_source::CommandDescription;
use crate::error::{CodegenError, codegen_error};
use crate::type_modules::DESCRIPTIONS_SOURCE_FILE;
use crate::write_if_changed::{remove_unwritten_files, write_if_changed};

const SCHEMAS_FOLDER: &str = "schemas";
const INDEX_FILE: &str = "index.json";
const COMMANDS_FOLDER: &str = "commands";
/// The `$id` of a file is this plus its path under schemas/ (contracts 14 row 8). Nothing serves it before Phase 2.
const SCHEMA_URL_BASE: &str = "https://schemas.rastai.dev/";
/// A command's arguments and results are one schema id per command (contracts 5.2).
const COMMAND_SCHEMA_ID_PREFIX: &str = "rastai.command.";
const COMMAND_SCHEMA_VERSION: u32 = 1;
/// Bumped only if the shape of index.json itself changes.
const INDEX_FORMAT_VERSION: u32 = 1;
// The keywords a command schema adds to JSON Schema (contracts 12.6, 15.1).
const STABILITY_KEYWORD: &str = "x-rastai-stability";
const SINCE_VERSION_KEYWORD: &str = "x-rastai-since-version";
// The two `$defs` entries of a command schema (layout 2.7).
const ARGUMENTS_DEFINITION: &str = "arguments";
const RESULT_DEFINITION: &str = "result";
const DEFINITIONS_KEY: &str = "$defs";
const DIALECT_KEY: &str = "$schema";
/// Where the core command rows will live; a command whose types aren't described is fixed there or in the registry.
const COMMAND_TABLE_FILE: &str = "crates/rastai-core/src/commands/table.rs";

/// One schema file: its schema id, its path under schemas/ (with `/` separators) and its contents.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaFile {
    pub schema_id: String,
    pub relative_path: String,
    pub schema: Value,
}

/// Writes every document and command schema and the index under `output_root`, and deletes the command schemas
/// it no longer writes: `schemas/commands/` is this generator's alone. The rest of `schemas/` is not
/// (`schemas/names.json` joins it in M1-57), so a document schema it stops writing is deleted by hand; check-generated
/// names it. Returns how many files changed or were deleted.
pub fn write_json_schema_files(
    output_root: &Path,
    document_types: &[DocumentType],
    all_types: &[TypeDescription],
    commands: &[CommandDescription],
) -> Result<usize, CodegenError> {
    let mut files = document_schema_files(document_types, all_types)?;
    files.extend(command_schema_files(commands, all_types)?);
    files.sort_by(|left, right| left.schema_id.cmp(&right.schema_id));
    let schemas_folder = output_root.join(SCHEMAS_FOLDER);
    let mut changed_count = 0;
    let mut written_paths = Vec::new();
    for file in &files {
        let path = schemas_folder.join(&file.relative_path);
        if write_if_changed(&path, &pretty_text(&file.schema, &path)?)? {
            changed_count += 1;
        }
        written_paths.push(path);
    }
    let index_path = schemas_folder.join(INDEX_FILE);
    if write_if_changed(&index_path, &pretty_text(&schema_index(&files), &index_path)?)? {
        changed_count += 1;
    }
    changed_count += remove_unwritten_files(&schemas_folder.join(COMMANDS_FOLDER), &written_paths)?;
    Ok(changed_count)
}

/// One file per document type: `schemas/rastai.model/1.json` holds the schema of the type's whole description.
pub fn document_schema_files(
    document_types: &[DocumentType],
    all_types: &[TypeDescription],
) -> Result<Vec<SchemaFile>, CodegenError> {
    let mut files = Vec::new();
    for document_type in document_types {
        let description = find_or_fail(all_types, document_type.description_name, DESCRIPTIONS_SOURCE_FILE)?;
        let schema_id = format!("{}/{}", document_type.type_name, document_type.schema_version);
        let relative_path = format!("{schema_id}.json");
        let mut schema = json_schema_for(description, all_types);
        insert_member(&mut schema, "$id", json!(format!("{SCHEMA_URL_BASE}{relative_path}")));
        files.push(SchemaFile {
            schema_id,
            relative_path,
            schema,
        });
    }
    Ok(files)
}

/// One file per command: `schemas/commands/rastai.command.model.load/1.json`.
pub fn command_schema_files(
    commands: &[CommandDescription],
    all_types: &[TypeDescription],
) -> Result<Vec<SchemaFile>, CodegenError> {
    let mut files = Vec::new();
    for command in commands {
        files.push(command_schema_file(command, all_types)?);
    }
    Ok(files)
}

fn command_schema_file(
    command: &CommandDescription,
    all_types: &[TypeDescription],
) -> Result<SchemaFile, CodegenError> {
    let schema_id = format!(
        "{COMMAND_SCHEMA_ID_PREFIX}{}/{COMMAND_SCHEMA_VERSION}",
        command.command_id
    );
    let relative_path = format!("{COMMANDS_FOLDER}/{schema_id}.json");
    let arguments = find_or_fail(all_types, &command.arguments_type, COMMAND_TABLE_FILE)?;
    let result = find_or_fail(all_types, &command.result_type, COMMAND_TABLE_FILE)?;
    // Each type's own schema carries the types it refers to in `$defs`. They are merged into the file's `$defs`
    // beside `arguments` and `result`, so every `#/$defs/<Name>` reference still points at its definition.
    // The dialect line is the one every rastai-formats schema carries, taken from the arguments' schema.
    let arguments_full = json_schema_for(arguments, all_types);
    let dialect = arguments_full.get(DIALECT_KEY).cloned().unwrap_or(Value::Null);
    let mut definitions = Map::new();
    let arguments_schema = split_off_definitions(arguments_full, &mut definitions);
    let result_schema = split_off_definitions(json_schema_for(result, all_types), &mut definitions);
    definitions.insert(ARGUMENTS_DEFINITION.to_string(), arguments_schema);
    definitions.insert(RESULT_DEFINITION.to_string(), result_schema);
    let schema = json!({
        DIALECT_KEY: dialect,
        "$id": format!("{SCHEMA_URL_BASE}{relative_path}"),
        "title": command.command_id,
        "description": command.title,
        STABILITY_KEYWORD: command.stability,
        SINCE_VERSION_KEYWORD: command.since_version,
        DEFINITIONS_KEY: Value::Object(definitions),
    });
    Ok(SchemaFile {
        schema_id,
        relative_path,
        schema,
    })
}

/// Takes `$defs` (and the dialect line) out of one type's schema, adding the definitions to `definitions`, and
/// returns what is left: the type's own object schema.
fn split_off_definitions(schema: Value, definitions: &mut Map<String, Value>) -> Value {
    let Value::Object(mut members) = schema else {
        return schema;
    };
    members.remove(DIALECT_KEY);
    if let Some(Value::Object(referenced)) = members.remove(DEFINITIONS_KEY) {
        for (name, definition) in referenced {
            definitions.insert(name, definition);
        }
    }
    Value::Object(members)
}

/// index.json: every schema id, sorted, with its file under schemas/ and its content hash (canonical JSON, then
/// BLAKE3, the same hash every document uses), for the Phase 2 publish step.
pub fn schema_index(files: &[SchemaFile]) -> Value {
    let entries: Vec<Value> = files
        .iter()
        .map(|file| {
            json!({
                "schema_id": file.schema_id,
                "file": file.relative_path,
                "content_hash": content_hash_of_value(&file.schema),
            })
        })
        .collect();
    json!({ "format_version": INDEX_FORMAT_VERSION, "schemas": entries })
}

fn find_or_fail<'a>(
    all_types: &'a [TypeDescription],
    name: &str,
    source_file: &str,
) -> Result<&'a TypeDescription, CodegenError> {
    find_description(all_types, name).ok_or_else(|| {
        codegen_error(
            Path::new(source_file),
            format!("the type `{name}` has no description in describe_all_types()"),
        )
    })
}

fn insert_member(schema: &mut Value, key: &str, value: Value) {
    if let Value::Object(members) = schema {
        members.insert(key.to_string(), value);
    }
}

fn pretty_text(value: &Value, path: &Path) -> Result<String, CodegenError> {
    let bytes = pretty_json(value).map_err(|error| codegen_error(path, format!("can't write the JSON: {error:?}")))?;
    String::from_utf8(bytes).map_err(|error| codegen_error(path, format!("the JSON isn't UTF-8: {error}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rastai_formats::describe::types::{FieldKind, OPTIONAL, REQUIRED, field, list_of};
    use rastai_formats::header::{describe_shared_header, header_fields};

    fn sample_types() -> Vec<TypeDescription> {
        let mut document_fields = header_fields();
        document_fields.push(field(
            "items",
            list_of(FieldKind::Type("SampleItem".to_string())),
            REQUIRED,
            "The items.",
        ));
        vec![
            describe_shared_header(),
            TypeDescription {
                name: "SampleDocument".to_string(),
                schema_id: "rastai.sample/1".to_string(),
                doc: "A sample document.".to_string(),
                fields: document_fields,
            },
            TypeDescription {
                name: "SampleItem".to_string(),
                schema_id: "rastai.sample/1".to_string(),
                doc: "One item.".to_string(),
                fields: vec![field("label", FieldKind::String, OPTIONAL, "A label.")],
            },
            TypeDescription {
                name: "SampleRunArgs".to_string(),
                schema_id: "rastai.command.sample.run/1".to_string(),
                doc: "Arguments.".to_string(),
                fields: vec![field(
                    "item",
                    FieldKind::Type("SampleItem".to_string()),
                    REQUIRED,
                    "Which.",
                )],
            },
            TypeDescription {
                name: "SampleRunResult".to_string(),
                schema_id: "rastai.command.sample.run/1".to_string(),
                doc: "Result.".to_string(),
                fields: vec![field("ok", FieldKind::Boolean, REQUIRED, "Whether it worked.")],
            },
        ]
    }

    fn sample_command() -> CommandDescription {
        CommandDescription {
            command_id: "sample.run".to_string(),
            title: "Run the sample".to_string(),
            kind: "sync".to_string(),
            stability: "preview".to_string(),
            since_version: "0.1.0".to_string(),
            arguments_type: "SampleRunArgs".to_string(),
            result_type: "SampleRunResult".to_string(),
        }
    }

    const SAMPLE_DOCUMENT_TYPE: DocumentType = DocumentType {
        type_name: "rastai.sample",
        schema_version: 1,
        file_extensions: &[".sample"],
        description_name: "SampleDocument",
    };

    #[test]
    fn a_document_schema_has_its_id_and_its_parts_in_defs() {
        let files = document_schema_files(&[SAMPLE_DOCUMENT_TYPE], &sample_types()).unwrap();
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.schema_id, "rastai.sample/1");
        assert_eq!(file.relative_path, "rastai.sample/1.json");
        assert_eq!(file.schema["$id"], "https://schemas.rastai.dev/rastai.sample/1.json");
        assert_eq!(file.schema["title"], "SampleDocument");
        assert_eq!(
            file.schema["properties"]["items"]["items"]["$ref"],
            "#/$defs/SampleItem"
        );
        assert_eq!(file.schema["$defs"]["SampleItem"]["type"], "object");
    }

    #[test]
    fn a_command_schema_has_arguments_result_stability_and_since_version() {
        let file = command_schema_file(&sample_command(), &sample_types()).unwrap();
        assert_eq!(file.schema_id, "rastai.command.sample.run/1");
        assert_eq!(file.relative_path, "commands/rastai.command.sample.run/1.json");
        let schema = &file.schema;
        assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
        assert_eq!(
            schema["$id"],
            "https://schemas.rastai.dev/commands/rastai.command.sample.run/1.json"
        );
        assert_eq!(schema["title"], "sample.run");
        assert_eq!(schema[STABILITY_KEYWORD], "preview");
        assert_eq!(schema[SINCE_VERSION_KEYWORD], "0.1.0");
        let definitions = schema["$defs"].as_object().unwrap();
        let names: Vec<&String> = definitions.keys().collect();
        assert_eq!(names, ["SampleItem", "arguments", "result"]);
        // The arguments' reference resolves in the merged `$defs`, and the parts carry no dialect of their own.
        assert_eq!(
            definitions["arguments"]["properties"]["item"]["$ref"],
            "#/$defs/SampleItem"
        );
        assert!(definitions["arguments"].get("$schema").is_none());
        assert_eq!(definitions["result"]["required"], json!(["ok"]));
    }

    #[test]
    fn a_command_whose_types_are_not_described_is_an_error() {
        let mut command = sample_command();
        command.result_type = "MissingResult".to_string();
        let error = command_schema_file(&command, &sample_types()).unwrap_err();
        assert!(error.problem.contains("`MissingResult`"), "{error}");
        assert_eq!(error.path, Path::new(COMMAND_TABLE_FILE));
    }

    #[test]
    fn the_index_lists_each_file_with_its_content_hash() {
        let files = command_schema_files(&[sample_command()], &sample_types()).unwrap();
        let index = schema_index(&files);
        assert_eq!(index["format_version"], 1);
        let entry = &index["schemas"][0];
        assert_eq!(entry["schema_id"], "rastai.command.sample.run/1");
        assert_eq!(entry["file"], "commands/rastai.command.sample.run/1.json");
        assert_eq!(entry["content_hash"], content_hash_of_value(&files[0].schema).as_str());
        assert!(entry["content_hash"].as_str().unwrap().starts_with("blake3:"));
    }

    #[test]
    fn an_empty_registry_writes_an_empty_index() {
        let index = schema_index(&[]);
        assert_eq!(index, json!({"format_version": 1, "schemas": []}));
    }

    #[test]
    fn files_are_written_under_schemas_and_rewritten_only_on_change() {
        let output_root = std::env::temp_dir().join(format!("rastai-codegen-schemas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&output_root);
        let changed = write_json_schema_files(
            &output_root,
            &[SAMPLE_DOCUMENT_TYPE],
            &sample_types(),
            &[sample_command()],
        )
        .unwrap();
        assert_eq!(changed, 3);
        let index_text = std::fs::read_to_string(output_root.join("schemas/index.json")).unwrap();
        assert!(index_text.ends_with("}\n"));
        assert!(output_root.join("schemas/rastai.sample/1.json").is_file());
        assert!(
            output_root
                .join("schemas/commands/rastai.command.sample.run/1.json")
                .is_file()
        );
        let changed_again = write_json_schema_files(
            &output_root,
            &[SAMPLE_DOCUMENT_TYPE],
            &sample_types(),
            &[sample_command()],
        )
        .unwrap();
        assert_eq!(changed_again, 0);
        std::fs::remove_dir_all(&output_root).unwrap();
    }
}
