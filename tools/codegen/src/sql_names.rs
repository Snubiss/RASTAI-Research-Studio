//! Table and column names from the store's migration files (contracts 12.6, layout rust-control-plane.md 2.8), so
//! `schemas/names.json` lists what the migrated databases really hold without rastai-codegen depending on
//! rastai-store. `rastai-store/tests/index_migrations.rs` runs the real migrations and compares `PRAGMA table_info`
//! with what this reader finds, so the two can't drift apart.
//!
//! A small reader of the two statements that name tables and columns: `CREATE TABLE` and
//! `ALTER TABLE ... ADD COLUMN`. Index statements and data changes (`INSERT`, `UPDATE`, `DELETE`) name nothing new
//! and are skipped. Any other statement (`DROP TABLE`, a rename, a trigger, a view) is refused with the file and the
//! statement's first words, so a migration this reader can't follow fails `just codegen` instead of leaving a wrong
//! `names.json`. The files are our own migrations, not untrusted input, so this reader has no fuzz target
//! (ADR-0037 covers parsers of untrusted input).

use std::fs;
use std::path::Path;

use crate::error::{CodegenError, codegen_error};

const SQL_EXTENSION: &str = ".sql";
const STATEMENT_END: char = ';';
const LINE_COMMENT_START: &str = "--";
const BLOCK_COMMENT_START: &str = "/*";
const BLOCK_COMMENT_END: &str = "*/";
/// Quotes SQLite accepts around a string (single) or an identifier (double, backtick). A `;`, `,` or comment start
/// inside quotes is text, not syntax.
const QUOTE_CHARACTERS: [char; 3] = ['\'', '"', '`'];
/// The first words of a table constraint inside `CREATE TABLE (...)`: these entries name no column.
const TABLE_CONSTRAINT_WORDS: [&str; 5] = ["PRIMARY", "UNIQUE", "CHECK", "FOREIGN", "CONSTRAINT"];
/// Statements that name no table or column: their first words, in capitals.
const SKIPPED_STATEMENT_STARTS: [&[&str]; 6] = [
    &["CREATE", "INDEX"],
    &["CREATE", "UNIQUE", "INDEX"],
    &["DROP", "INDEX"],
    &["INSERT"],
    &["UPDATE"],
    &["DELETE"],
];
/// How many of a refused statement's words its error shows.
const REFUSED_STATEMENT_WORDS: usize = 4;

/// One table a migration creates, and its columns in the order they were declared or added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlTable {
    pub table_name: String,
    pub column_names: Vec<String>,
}

/// Every table the `.sql` files in `folder` create, sorted by table name, with the columns of each in declaration
/// order. Files are read in name order (`0001_initial.sql` first), the order the store runs them in. A folder that
/// doesn't exist yet (`vault/migrations/` before M3) holds no tables.
pub fn read_migration_tables(folder: &Path) -> Result<Vec<SqlTable>, CodegenError> {
    let mut tables = Vec::new();
    for path in list_sql_files(folder)? {
        let sql =
            fs::read_to_string(&path).map_err(|error| codegen_error(&path, format!("can't read the file: {error}")))?;
        apply_migration_sql(&sql, &mut tables).map_err(|problem| codegen_error(&path, problem))?;
    }
    tables.sort_by(|left, right| left.table_name.cmp(&right.table_name));
    Ok(tables)
}

/// Applies one migration's statements to `tables`: a `CREATE TABLE` adds a table, an `ALTER TABLE ... ADD COLUMN`
/// adds a column. Returns the problem as a sentence when a statement can't be followed.
pub fn apply_migration_sql(sql: &str, tables: &mut Vec<SqlTable>) -> Result<(), String> {
    for (index, statement) in split_statements(sql).iter().enumerate() {
        let statement_number = index + 1;
        apply_statement(statement, tables).map_err(|problem| format!("statement {statement_number}: {problem}"))?;
    }
    Ok(())
}

fn list_sql_files(folder: &Path) -> Result<Vec<std::path::PathBuf>, CodegenError> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let entries =
        fs::read_dir(folder).map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| codegen_error(folder, format!("can't list the folder: {error}")))?
            .path();
        let is_sql = path.to_str().is_some_and(|text| text.ends_with(SQL_EXTENSION));
        if is_sql && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn apply_statement(statement: &str, tables: &mut Vec<SqlTable>) -> Result<(), String> {
    let words = leading_words(statement);
    if starts_with_words(&words, &["CREATE", "TABLE"]) {
        let table = read_create_table(statement)?;
        if find_table(tables, &table.table_name).is_some() {
            return Err(format!("table `{}` is created twice", table.table_name));
        }
        tables.push(table);
        return Ok(());
    }
    if starts_with_words(&words, &["ALTER", "TABLE"]) {
        return apply_alter_table(statement, tables);
    }
    if SKIPPED_STATEMENT_STARTS
        .iter()
        .any(|start| starts_with_words(&words, start))
    {
        return Ok(());
    }
    let shown: Vec<&str> = statement.split_whitespace().take(REFUSED_STATEMENT_WORDS).collect();
    Err(format!(
        "`{} ...` isn't a statement sql_names.rs reads (CREATE TABLE, ALTER TABLE ... ADD COLUMN, index and data \
         statements); teach it this one so names.json stays true",
        shown.join(" ")
    ))
}

/// `CREATE TABLE [IF NOT EXISTS] name ( column definitions and constraints ) [options]`.
fn read_create_table(statement: &str) -> Result<SqlTable, String> {
    let Some(open) = find_outside_quotes(statement, '(') else {
        return Err("a CREATE TABLE without a column list (CREATE TABLE ... AS SELECT) can't be read".to_string());
    };
    let table_name = table_name_before(&statement[..open])?;
    let Some(close) = matching_parenthesis(statement, open) else {
        return Err(format!(
            "table `{table_name}`: the column list has no closing parenthesis"
        ));
    };
    let mut column_names = Vec::new();
    for definition in split_outside_quotes(&statement[open + 1..close], ',') {
        let Some(first_word) = definition.split_whitespace().next() else {
            return Err(format!("table `{table_name}`: an empty entry in the column list"));
        };
        if TABLE_CONSTRAINT_WORDS.contains(&first_word.to_ascii_uppercase().as_str()) {
            continue;
        }
        push_column(&mut column_names, &table_name, unquoted(first_word))?;
    }
    Ok(SqlTable {
        table_name,
        column_names,
    })
}

/// The table name in `CREATE TABLE [IF NOT EXISTS] name`, the text before the column list. Anything else there
/// (`CREATE TABLE copy AS SELECT count(*) ...`) is refused rather than guessed at.
fn table_name_before(head: &str) -> Result<String, String> {
    let words: Vec<&str> = head.split_whitespace().collect();
    let name_words = if starts_with_words(&leading_words(head), &["CREATE", "TABLE", "IF", "NOT", "EXISTS"]) {
        &words[5..]
    } else {
        &words[2..]
    };
    match name_words {
        [name] => Ok(unquoted(name)),
        _ => Err(format!("`{}` doesn't read as CREATE TABLE <name> (...)", head.trim())),
    }
}

/// `ALTER TABLE name ADD [COLUMN] column ...`. Any other change to a table is refused.
fn apply_alter_table(statement: &str, tables: &mut [SqlTable]) -> Result<(), String> {
    let words: Vec<&str> = statement.split_whitespace().collect();
    let table_name = words.get(2).map(|word| unquoted(word)).unwrap_or_default();
    let action = words.get(3).map(|word| word.to_ascii_uppercase()).unwrap_or_default();
    if action != "ADD" {
        return Err(format!(
            "`ALTER TABLE {table_name} {action}` isn't read: sql_names.rs knows only ADD COLUMN"
        ));
    }
    let has_column_word = words.get(4).is_some_and(|word| word.eq_ignore_ascii_case("COLUMN"));
    let column_position = if has_column_word { 5 } else { 4 };
    let Some(column_word) = words.get(column_position) else {
        return Err(format!("`ALTER TABLE {table_name} ADD` names no column"));
    };
    let Some(table_index) = find_table(tables, &table_name) else {
        return Err(format!(
            "ALTER TABLE names `{table_name}`, which no earlier statement created"
        ));
    };
    let table = &mut tables[table_index];
    push_column(&mut table.column_names, &table.table_name, unquoted(column_word))
}

fn push_column(column_names: &mut Vec<String>, table_name: &str, column_name: String) -> Result<(), String> {
    if column_names.contains(&column_name) {
        return Err(format!("table `{table_name}` has column `{column_name}` twice"));
    }
    column_names.push(column_name);
    Ok(())
}

fn find_table(tables: &[SqlTable], table_name: &str) -> Option<usize> {
    tables.iter().position(|table| table.table_name == table_name)
}

/// The first words of a statement in capitals, up to the first parenthesis, enough to tell its kind.
fn leading_words(statement: &str) -> Vec<String> {
    let head = statement.split('(').next().unwrap_or_default();
    head.split_whitespace().map(|word| word.to_ascii_uppercase()).collect()
}

fn starts_with_words(words: &[String], start: &[&str]) -> bool {
    words.len() >= start.len() && words.iter().zip(start).all(|(word, expected)| word == expected)
}

/// A name without the quotes SQLite allows around it: `"runs"`, `` `runs` `` and `[runs]` are all `runs`.
fn unquoted(word: &str) -> String {
    let quoted_pairs = [('"', '"'), ('`', '`'), ('[', ']')];
    for (open, close) in quoted_pairs {
        if word.len() >= 2 && word.starts_with(open) && word.ends_with(close) {
            return word[1..word.len() - 1].to_string();
        }
    }
    word.to_string()
}

/// The statements of a migration, comments removed, without their `;`, empty ones left out.
fn split_statements(sql: &str) -> Vec<String> {
    split_outside_quotes(&without_comments(sql), STATEMENT_END)
        .into_iter()
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect()
}

/// The text with every `-- line comment` and `/* block comment */` replaced by a space. Comment starts inside
/// quotes are text.
fn without_comments(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut position = 0;
    let mut open_quote: Option<char> = None;
    while let Some(character) = sql[position..].chars().next() {
        let rest = &sql[position..];
        if open_quote.is_none() && rest.starts_with(LINE_COMMENT_START) {
            position += rest.find('\n').unwrap_or(rest.len());
            output.push(' ');
            continue;
        }
        if open_quote.is_none() && rest.starts_with(BLOCK_COMMENT_START) {
            let end = rest
                .find(BLOCK_COMMENT_END)
                .map_or(rest.len(), |end| end + BLOCK_COMMENT_END.len());
            position += end;
            output.push(' ');
            continue;
        }
        open_quote = next_quote_state(open_quote, character);
        output.push(character);
        position += character.len_utf8();
    }
    output
}

/// The pieces of `text` between each `separator` at parenthesis depth 0 and outside quotes.
fn split_outside_quotes(text: &str, separator: char) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut open_quote: Option<char> = None;
    let mut depth: usize = 0;
    for character in text.chars() {
        if open_quote.is_none() {
            match character {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ if character == separator && depth == 0 => {
                    pieces.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
        }
        open_quote = next_quote_state(open_quote, character);
        current.push(character);
    }
    pieces.push(current);
    pieces
}

/// The byte offset of the first `wanted` outside quotes.
fn find_outside_quotes(text: &str, wanted: char) -> Option<usize> {
    let mut open_quote: Option<char> = None;
    for (offset, character) in text.char_indices() {
        if open_quote.is_none() && character == wanted {
            return Some(offset);
        }
        open_quote = next_quote_state(open_quote, character);
    }
    None
}

/// The byte offset of the `)` that closes the `(` at `open`, outside quotes.
fn matching_parenthesis(text: &str, open: usize) -> Option<usize> {
    let mut open_quote: Option<char> = None;
    let mut depth: usize = 0;
    for (offset, character) in text[open..].char_indices() {
        if open_quote.is_none() {
            if character == '(' {
                depth += 1;
            } else if character == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
        }
        open_quote = next_quote_state(open_quote, character);
    }
    None
}

/// Which quote is open after `character`. A doubled quote (`'it''s'`) closes and reopens, which leaves the same
/// state, so it needs no special case.
fn next_quote_state(open_quote: Option<char>, character: char) -> Option<char> {
    match open_quote {
        Some(quote) if character == quote => None,
        Some(quote) => Some(quote),
        None if QUOTE_CHARACTERS.contains(&character) => Some(character),
        None => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tables_of(sql: &str) -> Vec<SqlTable> {
        let mut tables = Vec::new();
        apply_migration_sql(sql, &mut tables).unwrap();
        tables
    }

    fn table(table_name: &str, column_names: &[&str]) -> SqlTable {
        SqlTable {
            table_name: table_name.to_string(),
            column_names: column_names.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn columns_are_read_in_order_and_constraints_skipped() {
        let sql = "CREATE TABLE runs (\n    run_id TEXT PRIMARY KEY,\n    state TEXT NOT NULL DEFAULT 'local',\n    \
                   pinned INTEGER NOT NULL DEFAULT 0,\n    PRIMARY KEY (run_id, state),\n    UNIQUE (state),\n    \
                   CHECK (pinned IN (0, 1)),\n    FOREIGN KEY (run_id) REFERENCES jobs (run_id)\n) STRICT;";
        assert_eq!(tables_of(sql), [table("runs", &["run_id", "state", "pinned"])]);
    }

    #[test]
    fn comments_and_quotes_hide_their_syntax() {
        let sql = "-- a comment; with ( and ,\nCREATE TABLE IF NOT EXISTS \"notes\" (\n  body TEXT DEFAULT 'a;b,c(--', \
                   /* two; */ [kind] TEXT\n);\nCREATE INDEX notes_by_kind ON notes (kind);";
        assert_eq!(tables_of(sql), [table("notes", &["body", "kind"])]);
    }

    #[test]
    fn add_column_appends_with_or_without_the_column_word() {
        let sql = "CREATE TABLE runs (run_id TEXT);\nALTER TABLE runs ADD COLUMN state TEXT;\n\
                   ALTER TABLE runs ADD pinned INTEGER;\nINSERT INTO runs (run_id) VALUES ('a;b');";
        assert_eq!(tables_of(sql), [table("runs", &["run_id", "state", "pinned"])]);
    }

    #[test]
    fn statements_it_cannot_follow_are_refused() {
        for sql in [
            "CREATE TABLE runs (run_id TEXT); DROP TABLE runs;",
            "CREATE TABLE runs (run_id TEXT); ALTER TABLE runs RENAME COLUMN run_id TO id;",
            "CREATE TABLE runs (run_id TEXT); ALTER TABLE runs DROP COLUMN run_id;",
            "CREATE TRIGGER t AFTER INSERT ON runs BEGIN SELECT 1; END;",
            "CREATE VIEW v AS SELECT 1;",
            "CREATE TABLE copy AS SELECT * FROM runs;",
            "CREATE TABLE copy AS SELECT count(*) FROM runs;",
            "ALTER TABLE missing ADD COLUMN x TEXT;",
            "CREATE TABLE runs (run_id TEXT); CREATE TABLE runs (run_id TEXT);",
            "CREATE TABLE runs (run_id TEXT, run_id TEXT);",
        ] {
            let mut tables = Vec::new();
            let problem = apply_migration_sql(sql, &mut tables).unwrap_err();
            assert!(problem.starts_with("statement "), "{sql}: {problem}");
        }
    }

    #[test]
    fn files_run_in_name_order_and_tables_come_out_sorted() {
        let folder = std::env::temp_dir().join(format!("rastai-sql-names-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("0002_more.sql"), "ALTER TABLE runs ADD COLUMN state TEXT;").unwrap();
        fs::write(
            folder.join("0001_initial.sql"),
            "CREATE TABLE runs (run_id TEXT);\nCREATE TABLE jobs (job_id);",
        )
        .unwrap();
        fs::write(folder.join("README.md"), "not a migration").unwrap();
        let tables = read_migration_tables(&folder).unwrap();
        assert_eq!(
            tables,
            [table("jobs", &["job_id"]), table("runs", &["run_id", "state"])]
        );
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_missing_folder_holds_no_tables() {
        assert_eq!(read_migration_tables(Path::new("no/such/migrations")).unwrap(), []);
    }

    #[test]
    fn the_error_names_the_file() {
        let folder = std::env::temp_dir().join(format!("rastai-sql-names-error-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("0001_initial.sql"), "DROP TABLE runs;").unwrap();
        let error = read_migration_tables(&folder).unwrap_err();
        assert_eq!(error.path, folder.join("0001_initial.sql"));
        assert!(
            error.problem.starts_with("statement 1: `DROP TABLE runs ...`"),
            "{error}"
        );
        fs::remove_dir_all(&folder).unwrap();
    }
}
