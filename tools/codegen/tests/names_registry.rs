//! The names registry on a sample tree (layout rust-control-plane.md 4.9): every kind of name is read from its
//! source into the expected, sorted `names.json`; a second run writes nothing; and a renamed command or a dropped
//! column shows up as a change in the file's diff, which is what check-names and the reviewer see.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use rastai_codegen::names_registry::{
    CompiledNames, NameRegistry, collect_names, compiled_names, render_names_registry, write_names_registry,
};
use rastai_codegen::repo_root::find_repo_root;
use rastai_formats::describe::types::{FieldKind, OPTIONAL, TypeDescription, field};
use rastai_formats::documents::DocumentType;
use rastai_formats::header::header_fields;

const NAMES_FILE: &str = "schemas/names.json";
const INDEX_MIGRATION: &str = "crates/rastai-store/src/index/migrations/0001_initial.sql";
const INDEX_MIGRATION_SQL: &str = "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_unix_ms INTEGER);\n\
     -- The run's state, a comment the reader skips.\n\
     CREATE TABLE runs (run_id TEXT PRIMARY KEY, state TEXT NOT NULL, PRIMARY KEY (run_id));\n\
     CREATE INDEX runs_by_state ON runs (state);\n";
const VAULT_MIGRATION: &str = "crates/rastai-store/src/vault/migrations/0001_initial.sql";
const VAULT_MIGRATION_SQL: &str = "CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY);\n\
     ALTER TABLE conversations ADD COLUMN title TEXT;\n";
const CATALOG_FILE: &str = "catalog/errors/RRS-E.json";
const CATALOG_ENTRY: &str = r#"[
  {
    "code": "RRS-E1003",
    "severity": "error",
    "title": "Unknown field",
    "message": "{field} isn't a field of {type_name}.",
    "basic_text": "This file has a setting Rastai doesn't know.",
    "fix_text": "Remove the field.",
    "docs_anchor": "rrs-e1003",
    "retired": false
  }
]
"#;

/// The sample's expected names.json: every list sorted, columns in declaration order, keys sorted.
const EXPECTED_NAMES_JSON: &str = r#"{
  "cli_verbs": [
    "acme tool run",
    "doctor",
    "model load",
    "serve"
  ],
  "command_ids": [
    "acme.tool.run",
    "model.load"
  ],
  "crates": [
    "sample-core",
    "sample-store"
  ],
  "document_schema_ids": [
    "rastai.model/1",
    "rastai.settings/1"
  ],
  "error_codes": [
    "RRS-E1003"
  ],
  "event_types": [
    "job.state_changed",
    "run.metrics"
  ],
  "format_version": 1,
  "js_packages": [
    "@rastai/acme-ui",
    "@rastai/client"
  ],
  "python_packages": [
    "rastai_acme",
    "rastai_sample"
  ],
  "settings_keys": [
    "training.lora",
    "ui.density",
    "ui.theme"
  ],
  "tables": {
    "rastai.db": {
      "runs": [
        "run_id",
        "state"
      ],
      "schema_migrations": [
        "version",
        "applied_unix_ms"
      ]
    },
    "vault.db": {
      "conversations": [
        "conversation_id",
        "title"
      ]
    }
  }
}
"#;

fn scratch_folder(test_name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rastai-names-registry-{}-{test_name}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn write_file(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A tree holding one source of every kind: two workspace crates, a catalog file, both databases' migrations, a
/// first-party plugin with a command, a Python package and a UI, and the pnpm workspace with a folder that has no
/// package.json yet.
fn write_sample_tree(root: &Path) {
    write_file(
        root,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/core\", \"crates/store\"]\n",
    );
    for (folder, name) in [("crates/core", "sample-core"), ("crates/store", "sample-store")] {
        let manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
        write_file(root, &format!("{folder}/Cargo.toml"), &manifest);
        write_file(root, &format!("{folder}/src/lib.rs"), "");
    }
    write_file(root, CATALOG_FILE, CATALOG_ENTRY);
    write_file(root, INDEX_MIGRATION, INDEX_MIGRATION_SQL);
    write_file(root, VAULT_MIGRATION, VAULT_MIGRATION_SQL);
    write_file(
        root,
        "plugins/acme/rastai-plugin.json",
        r#"{"plugin_id": "acme", "contributes": {"commands": [{"command_id": "acme.tool.run"}]}}"#,
    );
    write_file(root, "plugins/acme/python/rastai_acme/__init__.py", "");
    write_file(root, "plugins/acme/ui/package.json", r#"{"name": "@rastai/acme-ui"}"#);
    write_file(
        root,
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n  - \"plugins/*/ui\"\n\nminimumReleaseAge: 20160\n",
    );
    write_file(
        root,
        "packages/client/package.json",
        r#"{"name": "@rastai/client", "private": true}"#,
    );
    write_file(root, "packages/not-yet/src/generated/commands.js", "export {};\n");
    write_file(
        root,
        "python/rastai_sample/pyproject.toml",
        "[project]\nname = \"rastai-sample\"\n",
    );
    write_file(root, "python/rastai_sample/rastai_sample/__init__.py", "");
    // Neither is an import package: tests/ has no __init__.py, and envs/ isn't a project folder.
    write_file(root, "python/rastai_sample/tests/test_sample.py", "");
    write_file(root, "python/envs/cpu/pyproject.toml", "[project]\nname = \"cpu\"\n");
}

fn described(
    name: &str,
    schema_id: &str,
    fields: Vec<rastai_formats::describe::types::FieldDescription>,
) -> TypeDescription {
    TypeDescription {
        name: name.to_string(),
        schema_id: schema_id.to_string(),
        doc: String::new(),
        fields,
    }
}

/// A settings document with two sections, one of whose keys holds an object, a ModelSpec part, a command's
/// arguments and the shared header.
fn sample_types() -> Vec<TypeDescription> {
    let mut settings_fields = header_fields();
    settings_fields.push(field(
        "ui",
        FieldKind::Type("UiSettings".to_string()),
        OPTIONAL,
        "Interface.",
    ));
    settings_fields.push(field(
        "training",
        FieldKind::Type("TrainingSettings".to_string()),
        OPTIONAL,
        "Training.",
    ));
    vec![
        described("SharedHeader", "", header_fields()),
        described("Settings", "rastai.settings/1", settings_fields),
        described(
            "UiSettings",
            "rastai.settings/1",
            vec![
                field("theme", FieldKind::String, OPTIONAL, "Theme."),
                field("density", FieldKind::String, OPTIONAL, "Density."),
            ],
        ),
        described(
            "TrainingSettings",
            "rastai.settings/1",
            vec![field(
                "lora",
                FieldKind::Type("LoraSettings".to_string()),
                OPTIONAL,
                "LoRA.",
            )],
        ),
        described(
            "LoraSettings",
            "rastai.settings/1",
            vec![field("rank", FieldKind::Integer, OPTIONAL, "Rank.")],
        ),
        described(
            "GraphNode",
            "rastai.model/1",
            vec![field("node_id", FieldKind::String, OPTIONAL, "Id.")],
        ),
        described(
            "ModelLoadArgs",
            "rastai.command.model.load/1",
            vec![field("model_ref", FieldKind::String, OPTIONAL, "Model.")],
        ),
    ]
}

fn sample_compiled_names() -> CompiledNames {
    CompiledNames {
        command_ids: vec!["model.load".to_string()],
        event_types: vec!["run.metrics".to_string(), "job.state_changed".to_string()],
        cli_mode_words: vec!["serve".to_string()],
        alias_words: vec!["doctor".to_string()],
        all_types: sample_types(),
        document_types: vec![DocumentType {
            type_name: "rastai.settings",
            schema_version: 1,
            file_extensions: &[],
            description_name: "Settings",
        }],
    }
}

fn rendered(root: &Path, compiled: &CompiledNames) -> String {
    let registry: NameRegistry = collect_names(root, compiled).unwrap();
    render_names_registry(&registry).unwrap()
}

/// The lines only one side has, marked "-" (only in `old`) and "+" (only in `new`): enough of a diff to show which
/// names a change added or removed.
fn changed_lines(old: &str, new: &str) -> Vec<String> {
    let mut changes = Vec::new();
    for line in old.lines().filter(|line| !new.lines().any(|other| other == *line)) {
        changes.push(format!("-{}", line.trim()));
    }
    for line in new.lines().filter(|line| !old.lines().any(|other| other == *line)) {
        changes.push(format!("+{}", line.trim()));
    }
    changes
}

#[test]
fn a_sample_tree_gives_the_expected_sorted_names_json() {
    let root = scratch_folder("expected");
    write_sample_tree(&root);
    assert_eq!(rendered(&root, &sample_compiled_names()), EXPECTED_NAMES_JSON);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_second_run_writes_nothing() {
    let root = scratch_folder("stable");
    write_sample_tree(&root);
    let registry = collect_names(&root, &sample_compiled_names()).unwrap();
    assert!(write_names_registry(&root, &registry).unwrap());
    assert!(!write_names_registry(&root, &registry).unwrap());
    assert_eq!(fs::read_to_string(root.join(NAMES_FILE)).unwrap(), EXPECTED_NAMES_JSON);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_renamed_command_shows_in_the_diff() {
    let root = scratch_folder("renamed");
    write_sample_tree(&root);
    let before = rendered(&root, &sample_compiled_names());
    let mut renamed = sample_compiled_names();
    renamed.command_ids = vec!["model.open".to_string()];
    let after = rendered(&root, &renamed);
    assert_eq!(
        changed_lines(&before, &after),
        [
            "-\"model load\",",
            "-\"model.load\"",
            "+\"model open\",",
            "+\"model.open\""
        ]
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_dropped_column_shows_in_the_diff() {
    let root = scratch_folder("dropped");
    write_sample_tree(&root);
    let before = rendered(&root, &sample_compiled_names());
    write_file(
        &root,
        INDEX_MIGRATION,
        &INDEX_MIGRATION_SQL
            .replace("state TEXT NOT NULL, ", "")
            .replace(" ON runs (state)", " ON runs (run_id)"),
    );
    let after = rendered(&root, &sample_compiled_names());
    assert_eq!(
        changed_lines(&before, &after),
        ["-\"run_id\",", "-\"state\"", "+\"run_id\""]
    );
    fs::remove_dir_all(&root).unwrap();
}

/// The repository's own registry holds the core event types and every workspace crate, so the generator reads the
/// real sources, not only the sample's.
#[test]
fn the_repository_registry_has_the_core_tables_and_crates() {
    let repo_root = find_repo_root().unwrap();
    let registry = collect_names(&repo_root, &compiled_names()).unwrap();
    assert!(registry.event_types.contains(&"job.state_changed".to_string()));
    assert!(registry.crates.contains(&"rastai-codegen".to_string()));
    assert!(registry.cli_verbs.contains(&"storage rebuild-index".to_string()));
    assert!(registry.index_tables.iter().any(|table| table.table_name == "runs"));
}
