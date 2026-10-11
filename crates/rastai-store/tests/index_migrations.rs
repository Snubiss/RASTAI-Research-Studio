//! Migration 1 of `rastai.db` (layout rust-control-plane.md section 6.7): the migrated tables and columns are exactly
//! the ones contracts 8.2 lists (no `settings` table), `runs.archive_state` starts `local`, reopening migrates
//! nothing twice, and a file a newer app migrated is refused with `RRS-S0002` and left byte for byte as it was.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use rastai_core::paths::{DataPaths, rastai_db_path};
use rastai_store::index::reads::{TableColumns, list_table_columns, open_index_reader, read_index_schema_version};
use rastai_store::index::runs::{RunRow, get_run};
use rastai_store::index::writes::IndexWrite;
use rastai_store::sqlite::{Migration, open_database_for_writing};
use rastai_store::store::{INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS, Store, close_store, open_store};
use rastai_store::writer_thread::write_index;

// Where contracts 8.2's table of rastai.db tables starts (its first table) and where the next section begins.
const CONTRACTS_PATH: &str = "docs/architecture/contracts.md";
const INDEX_TABLES_HEADING: &str = "### 8.2 ";
const NEXT_HEADING: &str = "### 8.3 ";
// A table row of 8.2 starts with its backticked table name.
const TABLE_ROW_START: &str = "| `";
// The migration a newer app would have added; this build doesn't know it.
const NEWER_MIGRATION_SQL: &str = "CREATE TABLE added_by_a_newer_app (note TEXT NOT NULL) STRICT;";

fn test_paths(test_name: &str) -> DataPaths {
    let root = std::env::temp_dir().join(format!("rastai-store-migrations-{}-{test_name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    DataPaths {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
        state: root.join("state"),
        runtime: root.join("runtime"),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn close(store: Store) {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(close_store(store));
}

fn migrated_tables(test_name: &str) -> Vec<TableColumns> {
    let store = open_store(&test_paths(test_name)).unwrap();
    let tables = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        list_table_columns(&reader).unwrap()
    };
    close(store);
    tables
}

/// The backticked names of a contracts list cell that count as names (contracts 12.6): outside parentheses, and
/// before the first `:`, `;` or full stop outside backticks and parentheses. Notes after them are prose.
fn names_in_list_cell(cell: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = String::new();
    let mut in_backticks = false;
    let mut parenthesis_depth = 0;
    for character in cell.chars() {
        if in_backticks {
            if character == '`' {
                in_backticks = false;
                if parenthesis_depth == 0 {
                    names.push(current.clone());
                }
                current.clear();
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '`' => in_backticks = true,
            '(' => parenthesis_depth += 1,
            ')' => parenthesis_depth -= 1,
            ':' | ';' | '.' if parenthesis_depth == 0 => break,
            _ => {}
        }
    }
    names
}

/// The tables of contracts 8.2 that are built: rows still carrying a milestone marker, such as `archive_cursors`
/// (M11), are tables a later migration adds.
fn contracts_index_tables() -> Vec<TableColumns> {
    let contracts = std::fs::read_to_string(repo_root().join(CONTRACTS_PATH)).unwrap();
    let start = contracts.find(INDEX_TABLES_HEADING).unwrap();
    let end = start + contracts[start..].find(NEXT_HEADING).unwrap();
    let mut tables = Vec::new();
    let mut in_table = false;
    for line in contracts[start..end].lines() {
        // Only the first table of the section: the lineage relations table comes after it.
        if in_table && !line.starts_with('|') {
            break;
        }
        in_table = line.starts_with('|');
        if !line.starts_with(TABLE_ROW_START) {
            continue;
        }
        let cells: Vec<&str> = line.split('|').collect();
        let table_cell = cells[1];
        if table_cell.contains('(') {
            continue;
        }
        tables.push(TableColumns {
            table_name: names_in_list_cell(table_cell)[0].clone(),
            column_names: names_in_list_cell(cells[2]),
        });
    }
    tables.sort_by(|left, right| left.table_name.cmp(&right.table_name));
    tables
}

fn run_row(run_id: &str) -> RunRow {
    RunRow {
        run_id: run_id.to_string(),
        kind: "train".to_string(),
        state: "running".to_string(),
        project_id: String::new(),
        node_id: "node_local".to_string(),
        parent_run_id: String::new(),
        record_hash: "blake3:00".to_string(),
        acked_sequence: 0,
        created_unix_ms: 1,
        finished_unix_ms: None,
        pinned: false,
        // Ignored on write: the column default decides.
        archive_state: "archived".to_string(),
    }
}

#[test]
fn the_migrated_tables_and_columns_are_the_ones_contracts_8_2_lists() {
    let expected = contracts_index_tables();
    assert!(expected.len() > 10, "8.2's table wasn't found in {CONTRACTS_PATH}");
    assert_eq!(migrated_tables("contracts"), expected);
}

#[test]
fn the_migrated_schema_has_no_settings_table() {
    let tables = migrated_tables("no-settings");
    assert!(tables.iter().all(|table| table.table_name != "settings"));
    assert!(tables.iter().any(|table| table.table_name == "runs"));
    // archive_cursors arrives with M11's migration, not this one.
    assert!(tables.iter().all(|table| table.table_name != "archive_cursors"));
}

#[test]
fn a_new_run_starts_with_archive_state_local() {
    let store = open_store(&test_paths("archive-state")).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime
        .block_on(write_index(&store.index, IndexWrite::UpsertRun(run_row("run_a"))))
        .unwrap();
    let run = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        get_run(&reader, "run_a").unwrap().unwrap()
    };
    assert_eq!(run.archive_state, "local");
    runtime.block_on(close_store(store));
}

#[test]
fn reopening_migrates_nothing_twice() {
    let paths = test_paths("reopen");
    close(open_store(&paths).unwrap());
    let store = open_store(&paths).unwrap();
    let version = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        read_index_schema_version(&reader).unwrap()
    };
    assert_eq!(version, 1);
    close(store);
}

#[test]
fn a_newer_schema_version_is_refused_and_the_file_is_unchanged() {
    let paths = test_paths("newer-schema");
    close(open_store(&paths).unwrap());
    let database_path = rastai_db_path(&paths);
    // A newer app is this one plus a migration: it migrates the file to version 2 and closes it cleanly.
    let newer_migrations = [
        Migration {
            version: 1,
            sql: INDEX_MIGRATIONS[0].sql,
        },
        Migration {
            version: 2,
            sql: NEWER_MIGRATION_SQL,
        },
    ];
    drop(open_database_for_writing(&database_path, &INDEX_DATABASE_SETTINGS, &newer_migrations).unwrap());
    let bytes_before = std::fs::read(&database_path).unwrap();

    let error = open_store(&paths).err().unwrap();

    assert_eq!(error.code(), "RRS-S0002");
    assert_eq!(std::fs::read(&database_path).unwrap(), bytes_before);
}

#[test]
fn a_file_that_is_not_a_database_is_refused_as_damaged() {
    let paths = test_paths("not-a-database");
    std::fs::create_dir_all(&paths.data).unwrap();
    std::fs::write(rastai_db_path(&paths), vec![0x5a; 4096]).unwrap();
    let error = open_store(&paths).err().unwrap();
    assert_eq!(error.code(), "RRS-S0003");
}

#[test]
fn list_cells_are_read_the_way_check_names_reads_them() {
    let cell = " `run_id`, `state` (`local`; 9.9), `pinned` (no `x`: y): the notes `after` ";
    assert_eq!(names_in_list_cell(cell), vec!["run_id", "state", "pinned"]);
}
