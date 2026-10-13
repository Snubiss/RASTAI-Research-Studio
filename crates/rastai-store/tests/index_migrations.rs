//! The migrations of `rastai.db` (layout rust-control-plane.md section 6.7): the migrated tables and columns are exactly
//! the ones contracts 8.2 lists (no `settings` table) and the ones rastai-codegen's `sql_names.rs` reads from the
//! migration files for `schemas/names.json` (contracts 12.6), `runs.archive_state` starts `local`, and reopening
//! migrates nothing twice. The refusal of a file a newer app migrated is tested at the bottom of `src/store.rs`, because
//! modelling the newer app takes the crate's own migration functions, which no other crate may call.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use rastai_codegen::sql_names::read_migration_tables;
use rastai_core::paths::rastai_db_path;
use rastai_store::index::reads::{TableColumns, list_table_columns, open_index_reader, read_index_schema_version};
use rastai_store::index::runs::{RunRow, get_run};
use rastai_store::index::writes::IndexWrite;
use rastai_store::store::{Store, close_store, open_store};
use rastai_store::writer_thread::write_index;

mod common;
use common::{remove_test_folder, test_paths};

// Where contracts 8.2's table of rastai.db tables starts (its first table) and where the next section begins.
const CONTRACTS_PATH: &str = "docs/architecture/contracts.md";
const INDEX_TABLES_HEADING: &str = "### 8.2 ";
const NEXT_HEADING: &str = "### 8.3 ";
// A table row of 8.2 starts with its backticked table name.
const TABLE_ROW_START: &str = "| `";
// The milestone markers of contracts' conventions. In a list cell one covers only the name it follows (12.6).
const MILESTONE_MARKER_STARTS: &[&str] = &["(M", "(L2)", "(Phase 2)", "(Phase 3)"];
// The migration files the store compiles in, which sql_names.rs reads for schemas/names.json.
// The version of the last one (0003_token_creator.sql), which a migrated file records.
const LAST_MIGRATION_VERSION: i64 = 3;
const INDEX_MIGRATIONS_FOLDER: &str = "crates/rastai-store/src/index/migrations";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn close(store: Store) {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(close_store(store));
}

fn migrated_tables(test_name: &str) -> Vec<TableColumns> {
    let paths = test_paths(test_name);
    let store = open_store(&paths).unwrap();
    let tables = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        list_table_columns(&reader).unwrap()
    };
    close(store);
    remove_test_folder(&paths);
    tables
}

/// Whether a milestone marker is among the parenthesized groups right after a name: `x` (M4), `x` (`y`) (M1).
/// Such a name is built by a later migration, as `check-names` reads it (contracts 12.6).
fn milestone_marker_follows(rest: &str) -> bool {
    let mut rest = rest.trim_start();
    while rest.starts_with('(') {
        if MILESTONE_MARKER_STARTS.iter().any(|marker| rest.starts_with(marker)) {
            return true;
        }
        let Some(group_end) = rest.find(')') else {
            return false;
        };
        rest = rest[group_end + 1..].trim_start();
    }
    false
}

/// The backticked names of a contracts list cell that count as built names (contracts 12.6): outside parentheses,
/// before the first `:`, `;` or full stop outside backticks and parentheses, and without a milestone marker of their
/// own. Notes after them are prose.
fn names_in_list_cell(cell: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = String::new();
    let mut in_backticks = false;
    let mut parenthesis_depth = 0;
    for (position, character) in cell.char_indices() {
        if in_backticks {
            if character == '`' {
                in_backticks = false;
                let rest = &cell[position + 1..];
                if parenthesis_depth == 0 && !milestone_marker_follows(rest) {
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

// names.json's tables come from reading the .sql files, not from a database; this keeps that reader honest.
#[test]
fn the_tables_sql_names_reads_are_the_migrated_tables() {
    let read_from_files: Vec<TableColumns> = read_migration_tables(&repo_root().join(INDEX_MIGRATIONS_FOLDER))
        .unwrap()
        .into_iter()
        .map(|table| TableColumns {
            table_name: table.table_name,
            column_names: table.column_names,
        })
        .collect();
    assert!(
        read_from_files.len() > 10,
        "no migrations read from {INDEX_MIGRATIONS_FOLDER}"
    );
    assert_eq!(read_from_files, migrated_tables("sql-names"));
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
    let paths = test_paths("archive-state");
    let store = open_store(&paths).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime
        .block_on(write_index(&store.index, IndexWrite::UpsertRun(run_row("run_a"))))
        .unwrap();
    let run = {
        let reader = open_index_reader(&store.index_readers).unwrap();
        get_run(&reader, "run_a").unwrap().unwrap()
    };
    runtime.block_on(close_store(store));
    assert_eq!(run.archive_state, "local");
    remove_test_folder(&paths);
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
    close(store);
    assert_eq!(version, LAST_MIGRATION_VERSION);
    remove_test_folder(&paths);
}

#[test]
fn a_file_that_is_not_a_database_is_refused_as_damaged() {
    let paths = test_paths("not-a-database");
    std::fs::create_dir_all(&paths.data).unwrap();
    std::fs::write(rastai_db_path(&paths), vec![0x5a; 4096]).unwrap();
    let error = open_store(&paths).err().unwrap();
    assert_eq!(error.code(), "RRS-S0003");
    remove_test_folder(&paths);
}

#[test]
fn list_cells_are_read_the_way_check_names_reads_them() {
    let cell = " `run_id`, `state` (`local`; 9.9), `pinned` (no `x`: y): the notes `after` ";
    assert_eq!(names_in_list_cell(cell), vec!["run_id", "state", "pinned"]);
}

#[test]
fn a_column_with_its_own_milestone_marker_isnt_built_yet() {
    let cell = " `token_id`, `name` (its name), `created_by` (M1) (who made it; added later), `revoked_unix_ms` ";
    assert_eq!(names_in_list_cell(cell), vec!["token_id", "name", "revoked_unix_ms"]);
}
