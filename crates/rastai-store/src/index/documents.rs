//! Rows of `documents` and `undo_steps` (contracts 8.2). The server writes a document's row, its new undo step and
//! its journal row in one transaction after the file's rename (flows/platform.md 3.4); the steps are history only
//! and a rebuild starts every document with an empty stack.

use crate::error::StoreError;
use crate::index::reads::IndexReader;
use crate::sqlite::{
    SqlValue, SqliteTransaction, execute, integer_value, query_rows, read_integer, read_text, text_value,
};

const UPSERT_DOCUMENT: &str = "INSERT INTO documents (document_id, type, path, project_id, content_hash,
        updated_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT (document_id) DO UPDATE SET type = excluded.type, path = excluded.path,
        project_id = excluded.project_id, content_hash = excluded.content_hash,
        updated_unix_ms = excluded.updated_unix_ms";
// document.changed moves only the hash and the time; the rest of the row is the server's (UpsertDocument) or the
// rebuild's project scan.
const SET_DOCUMENT_HASH: &str = "UPDATE documents SET content_hash = ?2, updated_unix_ms = ?3 WHERE document_id = ?1";
const SELECT_DOCUMENT_BY_ID: &str = "SELECT document_id, type, path, project_id, content_hash, updated_unix_ms
    FROM documents WHERE document_id = ?1";

// A new step at a position replaces that position and everything above it: a new edit after an undo drops the
// redo steps (flows/platform.md 3.4).
const DELETE_STEPS_FROM_POSITION: &str = "DELETE FROM undo_steps WHERE document_id = ?1 AND position >= ?2";
const INSERT_UNDO_STEP: &str = "INSERT INTO undo_steps (document_id, position, forward_patch_json, inverse_patch_json,
        journal_id)
    VALUES (?1, ?2, ?3, ?4, ?5)";
const DELETE_ALL_STEPS: &str = "DELETE FROM undo_steps WHERE document_id = ?1";
const SELECT_STEPS_BY_DOCUMENT: &str = "SELECT document_id, position, forward_patch_json, inverse_patch_json,
        journal_id
    FROM undo_steps WHERE document_id = ?1 ORDER BY position";

/// One row of `documents`. `document_type` is the `type` column (a Rust keyword); `project_id` is "" for a
/// document outside any project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRow {
    pub document_id: String,
    pub document_type: String,
    pub path: String,
    pub project_id: String,
    pub content_hash: String,
    pub updated_unix_ms: i64,
}

/// One step of a document's undo history: the JSON Patch that redoes it and the one that undoes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoStepRow {
    pub document_id: String,
    pub position: u32,
    pub forward_patch_json: String,
    pub inverse_patch_json: String,
    pub journal_id: String,
}

pub(crate) fn upsert_document(transaction: &SqliteTransaction<'_>, row: &DocumentRow) -> Result<(), StoreError> {
    let parameters = [
        text_value(&row.document_id),
        text_value(&row.document_type),
        text_value(&row.path),
        text_value(&row.project_id),
        text_value(&row.content_hash),
        integer_value(row.updated_unix_ms),
    ];
    execute(transaction, UPSERT_DOCUMENT, &parameters)?;
    Ok(())
}

pub(crate) fn set_document_hash(
    transaction: &SqliteTransaction<'_>,
    document_id: &str,
    content_hash: &str,
    updated_unix_ms: i64,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(document_id),
        text_value(content_hash),
        integer_value(updated_unix_ms),
    ];
    execute(transaction, SET_DOCUMENT_HASH, &parameters)?;
    Ok(())
}

/// Records a step at its position, dropping any steps at or above it first. The document's row must exist: a step
/// for an unknown document is refused by the foreign key.
pub(crate) fn record_undo_step(transaction: &SqliteTransaction<'_>, row: &UndoStepRow) -> Result<(), StoreError> {
    let position = integer_value(i64::from(row.position));
    execute(
        transaction,
        DELETE_STEPS_FROM_POSITION,
        &[text_value(&row.document_id), position.clone()],
    )?;
    let parameters = [
        text_value(&row.document_id),
        position,
        text_value(&row.forward_patch_json),
        text_value(&row.inverse_patch_json),
        text_value(&row.journal_id),
    ];
    execute(transaction, INSERT_UNDO_STEP, &parameters)?;
    Ok(())
}

/// Forgets a document's history, as after an edit made outside Rastai (`RRS-C2002`).
pub(crate) fn clear_undo_steps(transaction: &SqliteTransaction<'_>, document_id: &str) -> Result<(), StoreError> {
    execute(transaction, DELETE_ALL_STEPS, &[text_value(document_id)])?;
    Ok(())
}

pub fn get_document(reader: &IndexReader, document_id: &str) -> Result<Option<DocumentRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_DOCUMENT_BY_ID, &[text_value(document_id)])?;
    match rows.first() {
        Some(row) => Ok(Some(document_from_columns(row)?)),
        None => Ok(None),
    }
}

/// A document's undo steps, oldest first.
pub fn list_undo_steps(reader: &IndexReader, document_id: &str) -> Result<Vec<UndoStepRow>, StoreError> {
    let rows = query_rows(&reader.connection, SELECT_STEPS_BY_DOCUMENT, &[text_value(document_id)])?;
    let mut steps = Vec::with_capacity(rows.len());
    for row in &rows {
        steps.push(undo_step_from_columns(row)?);
    }
    Ok(steps)
}

/// Columns in the order of `SELECT_DOCUMENT_BY_ID`.
fn document_from_columns(row: &[SqlValue]) -> Result<DocumentRow, StoreError> {
    Ok(DocumentRow {
        document_id: read_text(row, 0)?,
        document_type: read_text(row, 1)?,
        path: read_text(row, 2)?,
        project_id: read_text(row, 3)?,
        content_hash: read_text(row, 4)?,
        updated_unix_ms: read_integer(row, 5)?,
    })
}

/// Columns in the order of `SELECT_STEPS_BY_DOCUMENT`.
fn undo_step_from_columns(row: &[SqlValue]) -> Result<UndoStepRow, StoreError> {
    let position = read_integer(row, 1)?;
    let position = match u32::try_from(position) {
        Ok(position) => position,
        Err(_) => {
            return Err(StoreError::IndexDamaged {
                reason: format!("undo step position {position} is out of range"),
            });
        }
    };
    Ok(UndoStepRow {
        document_id: read_text(row, 0)?,
        position,
        forward_patch_json: read_text(row, 2)?,
        inverse_patch_json: read_text(row, 3)?,
        journal_id: read_text(row, 4)?,
    })
}
