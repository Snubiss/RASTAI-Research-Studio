//! Rows of `artifacts`, `artifact_copies` and `lineage` (contracts 8.2). The indexer writes them from
//! `artifact.created` and `checkpoint.saved` records, so a rebuild restores every one (contracts 8.2 "Lineage").
//! The lineage queries arrive with build step 4 (layout section 6.9); `models` with `download.finished` (M2).

use crate::error::StoreError;
use crate::sqlite::{
    SqliteTransaction, boolean_value, execute, integer_value, query_rows_in_transaction, read_text, read_unsigned,
    text_value, unsigned_value,
};

// `pinned` is left out of the update: only pin.changed moves it.
const UPSERT_ARTIFACT: &str = "INSERT INTO artifacts (artifact_id, kind, content_hash, size_bytes, home_node_id,
        created_unix_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT (artifact_id) DO UPDATE SET kind = excluded.kind, content_hash = excluded.content_hash,
        size_bytes = excluded.size_bytes, home_node_id = excluded.home_node_id,
        created_unix_ms = excluded.created_unix_ms";
// A copy that is already recorded keeps its path and its last verification.
const INSERT_ARTIFACT_COPY: &str = "INSERT INTO artifact_copies (artifact_id, node_id, path) VALUES (?1, ?2, ?3)
    ON CONFLICT (artifact_id, node_id) DO NOTHING";
const SELECT_ARTIFACT_CONTENTS: &str = "SELECT kind, content_hash, size_bytes, home_node_id FROM artifacts
    WHERE artifact_id = ?1";
const SET_ARTIFACT_PINNED: &str = "UPDATE artifacts SET pinned = ?2 WHERE artifact_id = ?1";
const INSERT_LINEAGE: &str = "INSERT INTO lineage (from_id, to_id, relation) VALUES (?1, ?2, ?3)
    ON CONFLICT (from_id, to_id, relation) DO NOTHING";

// The home copy's bytes are in its node's blob store, found by the content hash, so it needs no path of its own.
// Copies elsewhere (transfer.finished, M9) record where they landed.
const HOME_COPY_PATH: &str = "";

/// An artifact as `artifact.created` reports it; `created_unix_ms` is the record's time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedArtifact<'a> {
    pub(crate) artifact_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) content_hash: &'a str,
    pub(crate) size_bytes: u64,
    pub(crate) home_node_id: &'a str,
    pub(crate) created_unix_ms: i64,
}

/// Upserts the artifact and records its copy on its home node.
pub(crate) fn upsert_artifact(
    transaction: &SqliteTransaction<'_>,
    artifact: &CreatedArtifact<'_>,
) -> Result<(), StoreError> {
    let parameters = [
        text_value(artifact.artifact_id),
        text_value(artifact.kind),
        text_value(artifact.content_hash),
        unsigned_value(artifact.size_bytes)?,
        text_value(artifact.home_node_id),
        integer_value(artifact.created_unix_ms),
    ];
    execute(transaction, UPSERT_ARTIFACT, &parameters)?;
    let copy = [
        text_value(artifact.artifact_id),
        text_value(artifact.home_node_id),
        text_value(HOME_COPY_PATH),
    ];
    execute(transaction, INSERT_ARTIFACT_COPY, &copy)?;
    Ok(())
}

/// Whether the index holds `artifact.artifact_id` with other contents (kind, hash, size or home node) than
/// `artifact`. An artifact never changes once made, so a run's log that names an existing one with other contents
/// would overwrite what another run made; the indexer refuses that record (indexer.rs). The same record applied again
/// finds equal contents, so this never stops a re-sent record.
pub(crate) fn artifact_differs(
    transaction: &SqliteTransaction<'_>,
    artifact: &CreatedArtifact<'_>,
) -> Result<bool, StoreError> {
    let rows = query_rows_in_transaction(
        transaction,
        SELECT_ARTIFACT_CONTENTS,
        &[text_value(artifact.artifact_id)],
    )?;
    let Some(row) = rows.first() else {
        return Ok(false);
    };
    let same = read_text(row, 0)? == artifact.kind
        && read_text(row, 1)? == artifact.content_hash
        && read_unsigned(row, 2)? == artifact.size_bytes
        && read_text(row, 3)? == artifact.home_node_id;
    Ok(!same)
}

pub(crate) fn set_artifact_pinned(
    transaction: &SqliteTransaction<'_>,
    artifact_id: &str,
    pinned: bool,
) -> Result<(), StoreError> {
    execute(
        transaction,
        SET_ARTIFACT_PINNED,
        &[text_value(artifact_id), boolean_value(pinned)],
    )?;
    Ok(())
}

/// One lineage link: `from_id` is the input, `to_id` what was made from it (contracts 8.2).
pub(crate) fn insert_lineage(
    transaction: &SqliteTransaction<'_>,
    from_id: &str,
    to_id: &str,
    relation: &str,
) -> Result<(), StoreError> {
    let parameters = [text_value(from_id), text_value(to_id), text_value(relation)];
    execute(transaction, INSERT_LINEAGE, &parameters)?;
    Ok(())
}
