//! `IndexWrite`, every change a caller can ask of `rastai.db`, and `apply_index_write`, one match arm per variant.
//! The writer thread applies each request inside its own savepoint (writer_thread.rs), so a write either lands whole
//! or not at all. There is no settings write: settings live only in files (contracts 11.1).
//!
//! Variants arrive with the code that needs them (layout section 6.2): the handler-call writes and the
//! `handler_calls` of the two `Apply*Events` (M5), `UpsertTransfer` (M9) and the archive writes (M11).
//! `SetUndoPosition` waits for undo (build step 4): `documents` has no column for a document's current position yet.

use rastai_proto::events::v1::EventRecord;

use crate::error::StoreError;
use crate::index::documents::{DocumentRow, UndoStepRow, clear_undo_steps, record_undo_step, upsert_document};
use crate::index::journal::{
    JournalFinishedRow, JournalStartedRow, finish_journal_row, insert_journal_started, mark_unfinished_journal_rows,
};
use crate::index::kept_tables::{
    NodeRow, PluginRow, ProjectRow, TokenRow, insert_token, revoke_plugin_tokens, revoke_token, upsert_node,
    upsert_plugin, upsert_project,
};
use crate::index::runs::{JobRow, RunRow, upsert_job, upsert_run};
use crate::indexer::{apply_daemon_events, apply_run_events};
use crate::sqlite::SqliteTransaction;

// PartialEq but not Eq: an event record can carry metric values, and floats have no total equality.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexWrite {
    /// A command started: its journal row, before it runs.
    JournalStarted(JournalStartedRow),
    /// The command ended: its outcome and redacted result. A row that is already finished is left as it is.
    JournalFinished(JournalFinishedRow),
    /// At startup: close every journal row a restart interrupted, as failed with `RRS-C0005`.
    MarkUnfinishedJournalRows,
    /// A batch of one run's event-log records, in log order: their rows and `runs.acked_sequence`, in one
    /// transaction. Records at or below the ack are skipped, so a re-sent batch changes nothing (indexer.rs). The
    /// run's row must already exist.
    ApplyRunEvents {
        run_id: String,
        records: Vec<EventRecord>,
    },
    /// A batch of the daemon log's records, in log order: their rows and `log_cursors.acked_sequence` of `log_name`
    /// (one name per segment, `daemon:<segment>`), in one transaction, skipping records at or below the cursor.
    ApplyDaemonEvents {
        log_name: String,
        records: Vec<EventRecord>,
    },
    UpsertRun(RunRow),
    UpsertJob(JobRow),
    UpsertDocument(DocumentRow),
    /// A new undo step; steps at or above its position (the redo steps) are dropped.
    RecordUndoStep(UndoStepRow),
    ClearUndoSteps {
        document_id: String,
    },
    UpsertNode(NodeRow),
    UpsertPlugin(PluginRow),
    /// A new token. One made by a token is refused with `StoreError::TokenCreatorRevoked` while that creator is
    /// revoked or gone (contracts 8.2).
    InsertToken(TokenRow),
    /// Revokes the token and every live token whose `created_by` chain leads to it, all with this time. A token
    /// already revoked keeps its first time.
    RevokeToken {
        token_id: String,
        revoked_unix_ms: i64,
    },
    /// Revokes every live token whose `created_by` chain leads to `plugin:<plugin_id>`, all with this time: a plugin
    /// disabled, uninstalled or gone at the start (contracts 8.2).
    RevokePluginTokens {
        plugin_id: String,
        revoked_unix_ms: i64,
    },
    UpsertProject(ProjectRow),
    /// Several writes, one transaction: all of them land, or none (a document edit's row, step and journal row).
    Batch(Vec<IndexWrite>),
}

/// Applies one write inside the caller's transaction. An error leaves it to the caller to undo what was written.
pub(crate) fn apply_index_write(transaction: &SqliteTransaction<'_>, write: &IndexWrite) -> Result<(), StoreError> {
    match write {
        IndexWrite::JournalStarted(row) => insert_journal_started(transaction, row),
        IndexWrite::JournalFinished(row) => finish_journal_row(transaction, row),
        IndexWrite::MarkUnfinishedJournalRows => {
            mark_unfinished_journal_rows(transaction, rastai_core::time::now_unix_ms())
        }
        IndexWrite::ApplyRunEvents { run_id, records } => apply_run_events(transaction, run_id, records),
        IndexWrite::ApplyDaemonEvents { log_name, records } => apply_daemon_events(transaction, log_name, records),
        IndexWrite::UpsertRun(row) => upsert_run(transaction, row),
        IndexWrite::UpsertJob(row) => upsert_job(transaction, row),
        IndexWrite::UpsertDocument(row) => upsert_document(transaction, row),
        IndexWrite::RecordUndoStep(row) => record_undo_step(transaction, row),
        IndexWrite::ClearUndoSteps { document_id } => clear_undo_steps(transaction, document_id),
        IndexWrite::UpsertNode(row) => upsert_node(transaction, row),
        IndexWrite::UpsertPlugin(row) => upsert_plugin(transaction, row),
        IndexWrite::InsertToken(row) => insert_token(transaction, row),
        IndexWrite::RevokeToken {
            token_id,
            revoked_unix_ms,
        } => revoke_token(transaction, token_id, *revoked_unix_ms),
        IndexWrite::RevokePluginTokens {
            plugin_id,
            revoked_unix_ms,
        } => revoke_plugin_tokens(transaction, plugin_id, *revoked_unix_ms),
        IndexWrite::UpsertProject(row) => upsert_project(transaction, row),
        IndexWrite::Batch(writes) => apply_each(transaction, writes),
    }
}

/// The writes of a `Batch`, in order; the first error stops it, and the writer's savepoint undoes the rest.
fn apply_each(transaction: &SqliteTransaction<'_>, writes: &[IndexWrite]) -> Result<(), StoreError> {
    for write in writes {
        apply_index_write(transaction, write)?;
    }
    Ok(())
}
