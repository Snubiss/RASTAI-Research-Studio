//! The names registry (contracts 12.6, layout rust-control-plane.md 2.8 and 4.9): `schemas/names.json`, the names
//! the code really has, one sorted list per kind. `tools/checks/check-names` compares it with contracts.md in both
//! directions, so a name can't reach the code without being registered, and a registered name can't stay in
//! contracts after the code dropped it.
//!
//! Every list is read from the code, never from a hand-kept list:
//! - command ids: rastai-core's `COMMANDS` and the `contributes.commands[]` of each first-party plugin manifest;
//! - CLI verbs: the `cli_words` of every command id, plus `CLI_MODE_WORDS` and the `ALIASES` words;
//! - event types: rastai-core's `EVENT_TYPES` (first-party manifests declare none yet: contracts 5.5 has no
//!   contribution key for event types, so a plugin's event types join when that key is registered);
//! - error codes: the catalog (`catalog/errors/` and `plugins/*/catalog/`);
//! - tables and columns: the store's migration files, read by sql_names.rs;
//! - settings keys: the `rastai.settings` document's description, flattened to `<section>.<key>`;
//! - document schema ids: every described type's schema id but the command ones;
//! - crates, JavaScript packages and Python import packages: the workspace manifests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rastai_core::commands::naming::{ALIASES, CLI_MODE_WORDS, cli_words};
use rastai_core::commands::table::COMMANDS;
use rastai_core::event_types::EVENT_TYPES;
use rastai_formats::describe::registry::{describe_all_types, find_description};
use rastai_formats::describe::types::{FieldKind, TypeDescription};
use rastai_formats::documents::{DOCUMENT_TYPES, DocumentType};
use rastai_formats::header::header_fields;
use rastai_formats::json::pretty::pretty_json;
use rastai_formats::json::reader::read_json;
use rastai_formats::limits::ImportLimits;
use serde_json::{Map, Value, json};

use crate::catalog_source::read_catalog;
use crate::error::{CodegenError, codegen_error};
use crate::sql_names::{SqlTable, read_migration_tables};
use crate::write_if_changed::write_if_changed;

const NAMES_FILE: &str = "schemas/names.json";
/// Bumped only if the shape of names.json itself changes (contracts 12.6).
const FORMAT_VERSION: u64 = 1;

// The two databases and their migration folders (contracts 8.2, 8.3).
const INDEX_DATABASE: &str = "rastai.db";
const VAULT_DATABASE: &str = "vault.db";
const INDEX_MIGRATIONS_FOLDER: &str = "crates/rastai-store/src/index/migrations";
const VAULT_MIGRATIONS_FOLDER: &str = "crates/rastai-store/src/vault/migrations";

// First-party plugins (contracts 1): plugins/<name>/rastai-plugin.json and plugins/<name>/python/.
const PLUGINS_FOLDER: &str = "plugins";
const PLUGIN_MANIFEST_FILE: &str = "rastai-plugin.json";
const MANIFEST_CONTRIBUTIONS_KEY: &str = "contributes";
const MANIFEST_COMMANDS_KEY: &str = "commands";
const MANIFEST_COMMAND_ID_KEY: &str = "command_id";

// Python: each project folder under python/ (a uv workspace member, with its pyproject.toml) and each plugin's
// python/ folder holds its import packages, the folders with an __init__.py directly inside it.
const PYTHON_FOLDER: &str = "python";
const PLUGIN_PYTHON_FOLDER: &str = "python";
const PYTHON_PROJECT_FILE: &str = "pyproject.toml";
const PYTHON_PACKAGE_MARKER: &str = "__init__.py";

// JavaScript: the pnpm workspace's package folders, each named by its package.json.
const PNPM_WORKSPACE_FILE: &str = "pnpm-workspace.yaml";
const PNPM_PACKAGES_KEY: &str = "packages:";
const PNPM_LIST_ITEM: &str = "- ";
const PACKAGE_MANIFEST_FILE: &str = "package.json";
const PACKAGE_NAME_KEY: &str = "name";
/// A folder pattern segment that matches every folder (`packages/*`, `plugins/*/ui`).
const ANY_FOLDER: &str = "*";

// Crates: the workspace members as cargo itself lists them. --no-deps reads only the manifests, so it needs no
// network and no lock resolution.
const CARGO_VARIABLE: &str = "CARGO";
const CARGO_PROGRAM: &str = "cargo";
const CARGO_METADATA_ARGUMENTS: [&str; 4] = ["metadata", "--no-deps", "--format-version", "1"];
const CARGO_WORKSPACE_MANIFEST: &str = "Cargo.toml";

/// The settings document's `type` (contracts 5.2, 11.2).
const SETTINGS_DOCUMENT_TYPE: &str = "rastai.settings";
/// Command argument and result types are covered by `command_ids` (contracts 12.6).
const COMMAND_SCHEMA_ID_PREFIX: &str = "rastai.command.";
const SETTINGS_KEY_SEPARATOR: &str = ".";
const CLI_WORD_SEPARATOR: &str = " ";

/// The names compiled into the crates rastai-codegen links: the core command and event tables, the CLI's fixed
/// words and the type descriptions. A test passes its own, so a renamed command can be shown without a rebuild.
#[derive(Debug, Clone)]
pub struct CompiledNames {
    pub command_ids: Vec<String>,
    pub event_types: Vec<String>,
    /// `CLI_MODE_WORDS`: `serve`, `node serve`, `storage rebuild-index`, ...
    pub cli_mode_words: Vec<String>,
    /// The first word of each `ALIASES` entry: `doctor`, `rerun`.
    pub alias_words: Vec<String>,
    pub all_types: Vec<TypeDescription>,
    pub document_types: Vec<DocumentType>,
}

/// The names of `schemas/names.json`, each list sorted and without repeats. Columns stay in declaration order, the
/// order `PRAGMA table_info` and contracts 8.2 list them in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRegistry {
    pub command_ids: Vec<String>,
    pub cli_verbs: Vec<String>,
    pub event_types: Vec<String>,
    pub error_codes: Vec<String>,
    pub index_tables: Vec<SqlTable>,
    pub vault_tables: Vec<SqlTable>,
    pub settings_keys: Vec<String>,
    pub document_schema_ids: Vec<String>,
    pub crates: Vec<String>,
    pub js_packages: Vec<String>,
    pub python_packages: Vec<String>,
}

/// The compiled-in names of this build: rastai-core's tables and rastai-formats' descriptions.
pub fn compiled_names() -> CompiledNames {
    CompiledNames {
        command_ids: COMMANDS.iter().map(|row| row.command_id.to_string()).collect(),
        event_types: EVENT_TYPES.iter().map(|row| row.event_type.to_string()).collect(),
        cli_mode_words: CLI_MODE_WORDS.iter().map(|words| words.to_string()).collect(),
        alias_words: ALIASES.iter().map(|(word, _)| word.to_string()).collect(),
        all_types: describe_all_types(),
        document_types: DOCUMENT_TYPES.to_vec(),
    }
}

/// Collects every name: the compiled-in ones, then those read from the files under `repo_root`.
pub fn collect_names(repo_root: &Path, compiled: &CompiledNames) -> Result<NameRegistry, CodegenError> {
    let mut command_ids = compiled.command_ids.clone();
    command_ids.extend(read_manifest_command_ids(repo_root)?);
    let cli_verbs = cli_verbs(&command_ids, compiled);
    Ok(NameRegistry {
        command_ids: sorted_unique(command_ids),
        cli_verbs: sorted_unique(cli_verbs),
        event_types: sorted_unique(compiled.event_types.clone()),
        error_codes: read_error_codes(repo_root)?,
        index_tables: read_migration_tables(&repo_root.join(INDEX_MIGRATIONS_FOLDER))?,
        vault_tables: read_migration_tables(&repo_root.join(VAULT_MIGRATIONS_FOLDER))?,
        settings_keys: settings_keys(&compiled.all_types, &compiled.document_types)?,
        document_schema_ids: document_schema_ids(&compiled.all_types),
        crates: read_crate_names(repo_root)?,
        js_packages: read_js_package_names(repo_root)?,
        python_packages: read_python_package_names(repo_root)?,
    })
}

/// names.json's text: pretty JSON, keys sorted, a newline at the end.
pub fn render_names_registry(registry: &NameRegistry) -> Result<String, CodegenError> {
    let mut databases = Map::new();
    databases.insert(INDEX_DATABASE.to_string(), tables_value(&registry.index_tables));
    databases.insert(VAULT_DATABASE.to_string(), tables_value(&registry.vault_tables));
    let value = json!({
        "format_version": FORMAT_VERSION,
        "command_ids": registry.command_ids,
        "cli_verbs": registry.cli_verbs,
        "event_types": registry.event_types,
        "error_codes": registry.error_codes,
        "tables": Value::Object(databases),
        "settings_keys": registry.settings_keys,
        "document_schema_ids": registry.document_schema_ids,
        "crates": registry.crates,
        "js_packages": registry.js_packages,
        "python_packages": registry.python_packages,
    });
    let path = Path::new(NAMES_FILE);
    let bytes = pretty_json(&value).map_err(|error| codegen_error(path, format!("can't write the JSON: {error:?}")))?;
    String::from_utf8(bytes).map_err(|error| codegen_error(path, format!("the JSON isn't UTF-8: {error}")))
}

/// Writes `schemas/names.json` under `output_root`. Returns true when the file changed.
pub fn write_names_registry(output_root: &Path, registry: &NameRegistry) -> Result<bool, CodegenError> {
    write_if_changed(&output_root.join(NAMES_FILE), &render_names_registry(registry)?)
}

fn tables_value(tables: &[SqlTable]) -> Value {
    let mut value = Map::new();
    for table in tables {
        value.insert(table.table_name.clone(), json!(table.column_names));
    }
    Value::Object(value)
}

fn sorted_unique(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

/// Each command id's CLI words joined by a space (`eval suite add-conversation`), the CLI's fixed words, and the
/// alias words.
fn cli_verbs(command_ids: &[String], compiled: &CompiledNames) -> Vec<String> {
    let mut verbs: Vec<String> = command_ids
        .iter()
        .map(|command_id| cli_words(command_id).join(CLI_WORD_SEPARATOR))
        .collect();
    verbs.extend(compiled.cli_mode_words.iter().cloned());
    verbs.extend(compiled.alias_words.iter().cloned());
    verbs
}

fn read_error_codes(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let catalog = read_catalog(repo_root)?;
    Ok(sorted_unique(
        catalog.errors.into_iter().map(|entry| entry.code).collect(),
    ))
}

/// Every described schema id but the command ones (`rastai.command.<command_id>/1`) and the shared header's empty
/// one: the documents and their parts share their document's id.
fn document_schema_ids(all_types: &[TypeDescription]) -> Vec<String> {
    let ids = all_types
        .iter()
        .map(|description| description.schema_id.clone())
        .filter(|schema_id| !schema_id.is_empty() && !schema_id.starts_with(COMMAND_SCHEMA_ID_PREFIX))
        .collect();
    sorted_unique(ids)
}

/// `<section>.<key>` for every key of every section of the settings document. A key holding an object stops there
/// (`training.lora`, not `training.lora.rank`). Empty while the settings document isn't described yet.
fn settings_keys(all_types: &[TypeDescription], document_types: &[DocumentType]) -> Result<Vec<String>, CodegenError> {
    let Some(settings_type) = document_types
        .iter()
        .find(|document_type| document_type.type_name == SETTINGS_DOCUMENT_TYPE)
    else {
        return Ok(Vec::new());
    };
    let document = find_type(all_types, settings_type.description_name)?;
    let header_names: Vec<String> = header_fields().into_iter().map(|field| field.name).collect();
    let mut keys = Vec::new();
    for section in &document.fields {
        if header_names.contains(&section.name) {
            continue;
        }
        let FieldKind::Type(section_type_name) = &section.kind else {
            let problem = format!(
                "settings section `{}` isn't a described type, so it has no keys",
                section.name
            );
            return Err(codegen_error(Path::new(NAMES_FILE), problem));
        };
        for key in &find_type(all_types, section_type_name)?.fields {
            keys.push(format!("{}{SETTINGS_KEY_SEPARATOR}{}", section.name, key.name));
        }
    }
    Ok(sorted_unique(keys))
}

fn find_type<'a>(all_types: &'a [TypeDescription], name: &str) -> Result<&'a TypeDescription, CodegenError> {
    find_description(all_types, name).ok_or_else(|| {
        let problem = format!("the settings description names type `{name}`, which describe_all_types() lacks");
        codegen_error(Path::new(NAMES_FILE), problem)
    })
}

/// The `command_id` of each `contributes.commands[]` entry of each first-party manifest.
fn read_manifest_command_ids(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let mut command_ids = Vec::new();
    for plugin_folder in child_folders(&repo_root.join(PLUGINS_FOLDER))? {
        let manifest_path = plugin_folder.join(PLUGIN_MANIFEST_FILE);
        if manifest_path.is_file() {
            command_ids.extend(manifest_command_ids(&read_json_file(&manifest_path)?, &manifest_path)?);
        }
    }
    Ok(command_ids)
}

fn manifest_command_ids(manifest: &Value, manifest_path: &Path) -> Result<Vec<String>, CodegenError> {
    let commands = manifest
        .get(MANIFEST_CONTRIBUTIONS_KEY)
        .and_then(|contributions| contributions.get(MANIFEST_COMMANDS_KEY));
    let Some(commands) = commands else {
        return Ok(Vec::new());
    };
    let Some(entries) = commands.as_array() else {
        return Err(codegen_error(
            manifest_path,
            "`contributes.commands` isn't a list".to_string(),
        ));
    };
    let mut command_ids = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match entry.get(MANIFEST_COMMAND_ID_KEY).and_then(Value::as_str) {
            Some(command_id) => command_ids.push(command_id.to_string()),
            None => {
                let problem = format!("`contributes.commands[{index}]` has no text `{MANIFEST_COMMAND_ID_KEY}`");
                return Err(codegen_error(manifest_path, problem));
            }
        }
    }
    Ok(command_ids)
}

/// The workspace members' package names, from `cargo metadata --no-deps` run in `repo_root`.
fn read_crate_names(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let manifest_path = repo_root.join(CARGO_WORKSPACE_MANIFEST);
    let cargo = std::env::var_os(CARGO_VARIABLE).unwrap_or_else(|| CARGO_PROGRAM.into());
    let output = Command::new(cargo)
        .args(CARGO_METADATA_ARGUMENTS)
        .current_dir(repo_root)
        .output()
        .map_err(|error| codegen_error(&manifest_path, format!("can't run cargo metadata: {error}")))?;
    if !output.status.success() {
        let problem = format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Err(codegen_error(&manifest_path, problem));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| codegen_error(&manifest_path, format!("cargo metadata wrote no JSON: {error}")))?;
    crate_names_from_metadata(&metadata, &manifest_path)
}

/// The names of the packages cargo metadata lists as workspace members.
fn crate_names_from_metadata(metadata: &Value, manifest_path: &Path) -> Result<Vec<String>, CodegenError> {
    let members: Vec<&str> = metadata["workspace_members"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(packages) = metadata["packages"].as_array() else {
        return Err(codegen_error(
            manifest_path,
            "cargo metadata lists no packages".to_string(),
        ));
    };
    let mut names = Vec::new();
    for package in packages {
        let is_member = package["id"].as_str().is_some_and(|id| members.contains(&id));
        if let (true, Some(name)) = (is_member, package["name"].as_str()) {
            names.push(name.to_string());
        }
    }
    Ok(sorted_unique(names))
}

/// The `name` of the package.json in each folder the pnpm workspace's `packages:` patterns match. A folder without
/// a package.json (a package whose generated files arrived before its manifest) or a manifest without a name has no
/// package name to list.
fn read_js_package_names(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let workspace_path = repo_root.join(PNPM_WORKSPACE_FILE);
    if !workspace_path.is_file() {
        return Ok(Vec::new());
    }
    let workspace = fs::read_to_string(&workspace_path)
        .map_err(|error| codegen_error(&workspace_path, format!("can't read the file: {error}")))?;
    let mut names = Vec::new();
    for pattern in pnpm_package_patterns(&workspace) {
        for folder in folders_matching(repo_root, &pattern)? {
            let manifest_path = folder.join(PACKAGE_MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            if let Some(name) = read_json_file(&manifest_path)?[PACKAGE_NAME_KEY].as_str() {
                names.push(name.to_string());
            }
        }
    }
    Ok(sorted_unique(names))
}

/// The items of the top-level `packages:` list of pnpm-workspace.yaml, quotes removed: `apps/*`, `plugins/*/ui`.
/// The list ends at the first line that is neither an item, a comment nor blank.
fn pnpm_package_patterns(workspace: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;
    for line in workspace.lines() {
        let trimmed = line.trim();
        if line.starts_with(PNPM_PACKAGES_KEY) {
            in_packages = true;
            continue;
        }
        if !in_packages || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(item) = trimmed.strip_prefix(PNPM_LIST_ITEM) else {
            break;
        };
        patterns.push(item.trim().trim_matches(['"', '\'']).to_string());
    }
    patterns
}

/// The folders under `repo_root` that a pattern of folder names matches, where a `*` segment matches any folder.
fn folders_matching(repo_root: &Path, pattern: &str) -> Result<Vec<PathBuf>, CodegenError> {
    let mut folders = vec![repo_root.to_path_buf()];
    for segment in pattern.split('/') {
        let mut next_folders = Vec::new();
        for folder in &folders {
            if segment == ANY_FOLDER {
                next_folders.extend(child_folders(folder)?);
            } else if folder.join(segment).is_dir() {
                next_folders.push(folder.join(segment));
            }
        }
        folders = next_folders;
    }
    Ok(folders)
}

/// The import packages of every Python project: python/<project>/<package>/__init__.py for each uv workspace member
/// under python/, and plugins/<plugin>/python/<package>/__init__.py for each first-party plugin.
fn read_python_package_names(repo_root: &Path) -> Result<Vec<String>, CodegenError> {
    let mut project_folders = Vec::new();
    for folder in child_folders(&repo_root.join(PYTHON_FOLDER))? {
        if folder.join(PYTHON_PROJECT_FILE).is_file() {
            project_folders.push(folder);
        }
    }
    for plugin_folder in child_folders(&repo_root.join(PLUGINS_FOLDER))? {
        project_folders.push(plugin_folder.join(PLUGIN_PYTHON_FOLDER));
    }
    let mut names = Vec::new();
    for project_folder in project_folders {
        for package_folder in child_folders(&project_folder)? {
            if package_folder.join(PYTHON_PACKAGE_MARKER).is_file() {
                names.push(folder_name(&package_folder));
            }
        }
    }
    Ok(sorted_unique(names))
}

/// The folders directly inside `folder`, sorted. A folder that doesn't exist has none.
fn child_folders(folder: &Path) -> Result<Vec<PathBuf>, CodegenError> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let entries =
        fs::read_dir(folder).map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?;
    let mut folders = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?
            .path();
        if path.is_dir() {
            folders.push(path);
        }
    }
    folders.sort();
    Ok(folders)
}

fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A JSON file through rastai-formats' strict reader: a key written twice is an error, never a silent overwrite.
fn read_json_file(path: &Path) -> Result<Value, CodegenError> {
    let bytes = fs::read(path).map_err(|error| codegen_error(path, format!("can't read the file: {error}")))?;
    let parsed = read_json(&bytes, &ImportLimits::default())
        .map_err(|error| codegen_error(path, format!("isn't valid JSON ({}: {error:?})", error.code())))?;
    Ok(parsed.value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn package_patterns_are_the_packages_list_items() {
        let workspace = "# comment\npackages:\n  - \"apps/*\"\n  # a note\n  - 'packages/*'\n  - plugins/*/ui\n\n\
                         minimumReleaseAge: 20160\n";
        assert_eq!(
            pnpm_package_patterns(workspace),
            ["apps/*", "packages/*", "plugins/*/ui"]
        );
    }

    #[test]
    fn cli_verbs_join_the_words_of_each_id() {
        let compiled = CompiledNames {
            command_ids: Vec::new(),
            event_types: Vec::new(),
            cli_mode_words: vec!["node serve".to_string()],
            alias_words: vec!["doctor".to_string()],
            all_types: Vec::new(),
            document_types: Vec::new(),
        };
        let verbs = cli_verbs(&["eval.suite.add_conversation".to_string()], &compiled);
        assert_eq!(verbs, ["eval suite add-conversation", "node serve", "doctor"]);
    }

    #[test]
    fn only_workspace_members_are_crates() {
        let metadata = json!({
            "workspace_members": ["path+file:///repo/crates/a#0.1.0"],
            "packages": [
                {"id": "path+file:///repo/crates/a#0.1.0", "name": "rastai-a"},
                {"id": "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0", "name": "serde"}
            ]
        });
        let names = crate_names_from_metadata(&metadata, Path::new("Cargo.toml")).unwrap();
        assert_eq!(names, ["rastai-a"]);
    }

    #[test]
    fn a_manifest_command_without_an_id_is_an_error() {
        let manifest = json!({"contributes": {"commands": [{"command_id": "acme.run"}, {"title": "No id"}]}});
        let error = manifest_command_ids(&manifest, Path::new("plugins/acme/rastai-plugin.json")).unwrap_err();
        assert!(error.problem.contains("commands[1]"), "{error}");
    }
}
