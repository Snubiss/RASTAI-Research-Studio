//! The indexer (layout rust-control-plane.md 6.4 and 13.1, flows/platform.md 6.1): `apply_event_record` turns one
//! `EventRecord` into its `rastai.db` rows. It is the one indexing path: live run logs, the daemon's own log, the
//! offline rebuild and restored frames all go through it, so their rows can't differ.
//!
//! Exactly once, in two layers:
//! - **Every write is an upsert or an update keyed by ids** (`run_metrics` by run, sequence and name), so a record
//!   applied twice leaves the same rows.
//! - **The ack commits with the rows.** `apply_run_events` and `apply_daemon_events` run inside one writer
//!   transaction: they skip records at or below the log's ack, index the rest and move the ack to the last one
//!   (`runs.acked_sequence` or `log_cursors.acked_sequence`). A crash before the commit loses both; a re-sent batch
//!   after it changes nothing, and an older one can't move the ack back.
//!
//! A run's log is untrusted (contracts 7.1): the job writes it (worker and plugin code), and a remote run's log is
//! mirrored from another machine. So `apply_run_events` checks every record before it writes a row: the record must
//! name the log's run, its payload must be one a run's log may carry (`run_log_may_carry`), and it may not reach a
//! row outside its run: a checkpoint or artifact id must be one of its kind (so it can't name a run, a message or an
//! id of the other kind), a checkpoint id may not be another run's, an existing artifact may only be repeated exactly
//! (every column, its home copy and its lineage rows already there), and a parent's relation must be one whose child
//! is an artifact (`RUN_LOG_PARENT_RELATIONS`). A record that breaks a rule is refused with `RRS-S2004`:
//! nothing of its batch is applied, the ack stays before it, and the log counts as damaged there, as at a corrupt
//! frame. The daemon's own log is the daemon's, so its records are applied as they are.
//!
//! The row a record makes is chosen by its payload, the typed `oneof` the event type names (contracts 7.3). A record
//! without a payload, or one whose payload makes no rows (live state, plugin events), is acknowledged and skipped.
//! Event types that have no payload message yet (`transfer.finished`, `download.finished`, `lineage.linked`,
//! `run.deleted`, the `archive.*` records) get their rows with the contract PR that adds the payload.

use rastai_core::ids::{IdPrefix, id_has_prefix};
use rastai_core::states::{
    eval_run_state_name, is_terminal_eval_run_state, job_state_name, node_state_name, run_state_name,
};
use rastai_proto::common::v1::{EvalRunState, JobState, NodeState, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    ArtifactCreated, CheckpointSaved, CommandFinished, DocumentChanged, EventRecord, JobStateChanged, NodeStateChanged,
    PinChanged, RunMetrics, RunStateChanged,
};

use crate::error::StoreError;
use crate::index::artifacts::{
    CreatedArtifact, StoredArtifact, compare_stored_artifact, has_artifact_copy, has_lineage, insert_lineage,
    set_artifact_pinned, upsert_artifact,
};
use crate::index::documents::set_document_hash;
use crate::index::journal::{FinishedCommand, record_command_finished};
use crate::index::kept_tables::set_node_state;
use crate::index::log_cursors::{read_log_cursor_for_writing, set_log_cursor};
use crate::index::runs::{
    JobStateUpdate, MetricValue, SavedCheckpoint, read_checkpoint_run_id, read_run_acked_sequence, replace_run_tags,
    set_checkpoint_pinned, set_run_acked_sequence, set_run_pinned, set_run_state, upsert_checkpoint, upsert_job_state,
    upsert_run_metric,
};
use crate::sqlite::SqliteTransaction;

// The lineage relation of a checkpoint to its run (contracts 8.2: "checkpoint_of: from checkpoint.saved").
const CHECKPOINT_OF_RELATION: &str = "checkpoint_of";
// The relations an `artifact.created` in a run's log may give a parent: those of contracts 8.2 whose child can be an
// artifact. `checkpoint_of` comes only from `checkpoint.saved`; `branch_of` and `trained_on` name a message or a run,
// which the daemon links with `lineage.linked`. A parent may be another run's checkpoint (an eval or a diff reads
// one), since the row's child is always the record's own new artifact.
const RUN_LOG_PARENT_RELATIONS: &[&str] = &[
    "pair_from",
    "dataset_of",
    "diff_of",
    "replay_of",
    "analysis_of",
    "evaluated_by",
    "exported_as",
];
// pin.changed names what it pins by one of these (contracts 7.3).
const PIN_SUBJECT_RUN: &str = "run";
const PIN_SUBJECT_CHECKPOINT: &str = "checkpoint";
const PIN_SUBJECT_ARTIFACT: &str = "artifact";
// The states that end a training run. Only `completed` is terminal (train.resume reopens the other two, contracts
// 9.5), but all three stop the run, so all three get a finish time; a resume clears it.
const RUN_STATES_THAT_END_A_RUN: &[RunState] = &[RunState::Completed, RunState::Failed, RunState::Cancelled];

/// Writes the rows of one record inside the writer's transaction (layout 6.4). Applying the same record again
/// leaves the same rows. The caller moves the ack; this function doesn't know which log the record came from.
pub fn apply_event_record(transaction: &SqliteTransaction<'_>, record: &EventRecord) -> Result<(), StoreError> {
    let Some(payload) = record.payload.as_ref().and_then(|payload| payload.payload.as_ref()) else {
        return Ok(());
    };
    match payload {
        Payload::JobStateChanged(change) => index_job_state(transaction, record, change),
        Payload::RunStateChanged(change) => index_run_state(transaction, record, change),
        Payload::RunMetrics(metrics) => index_run_metrics(transaction, record, metrics),
        Payload::RunTagsChanged(change) => replace_run_tags(transaction, &change.run_id, &change.tags),
        Payload::CheckpointSaved(checkpoint) => index_checkpoint(transaction, record, checkpoint),
        Payload::ArtifactCreated(artifact) => index_artifact(transaction, record, artifact),
        Payload::PinChanged(pin) => index_pin(transaction, pin),
        Payload::NodeStateChanged(change) => index_node_state(transaction, change),
        Payload::DocumentChanged(change) => index_document(transaction, record, change),
        Payload::CommandFinished(finished) => index_command_finished(transaction, record, finished),
        // Live state comes from heartbeats, a diagnostic is shown, not stored, and plugin events are the plugin's.
        // A plugin's `plugins.state` is the user's decision, written with `UpsertPlugin` when it is made; the
        // record also carries the host's own outcomes (`incompatible`, a `disabled` for a missing capability), which
        // are decided again at every start and must not be saved over that decision (layout 6.4, 12.1).
        Payload::JobProgress(_)
        | Payload::JobLogLine(_)
        | Payload::NodeHeartbeat(_)
        | Payload::DiagnosticReported(_)
        | Payload::PluginEvent(_)
        | Payload::PluginStateChanged(_) => Ok(()),
    }
}

/// `IndexWrite::ApplyRunEvents`: the records of one run's log after its ack, and the ack moved to the last one, in
/// the caller's transaction. The run's row must exist (the daemon writes it when it starts the run), so the ack has
/// somewhere to go; without it nothing is written. A record the run's log may not carry fails the whole write with
/// `RRS-S2004` (`check_run_log_record`); the caller sends the records before it again and stops there.
pub(crate) fn apply_run_events(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    records: &[EventRecord],
) -> Result<(), StoreError> {
    let Some(acked_sequence) = read_run_acked_sequence(transaction, run_id)? else {
        return Err(StoreError::DatabaseWriteFailed {
            reason: format!("no runs row for {run_id}, so its event records have no ack to move"),
        });
    };
    let mut newest_sequence = acked_sequence;
    for record in records_after(records, acked_sequence) {
        check_run_log_record(transaction, run_id, record)?;
        apply_event_record(transaction, record)?;
        newest_sequence = record.sequence;
    }
    set_run_acked_sequence(transaction, run_id, newest_sequence)
}

/// `IndexWrite::ApplyDaemonEvents`: the records of one daemon log segment after its cursor, and the cursor moved to
/// the last one, in the caller's transaction.
pub(crate) fn apply_daemon_events(
    transaction: &SqliteTransaction<'_>,
    log_name: &str,
    records: &[EventRecord],
) -> Result<(), StoreError> {
    let acked_sequence = read_log_cursor_for_writing(transaction, log_name)?.unwrap_or(0);
    let mut newest_sequence = acked_sequence;
    for record in records_after(records, acked_sequence) {
        apply_event_record(transaction, record)?;
        newest_sequence = record.sequence;
    }
    set_log_cursor(transaction, log_name, newest_sequence)
}

/// The records past the ack, in log order. A record at or below the ack was indexed by an earlier commit (or earlier
/// in this batch) and is skipped, so each one is applied exactly once.
fn records_after(records: &[EventRecord], acked_sequence: u64) -> Vec<&EventRecord> {
    let mut newer = Vec::new();
    let mut newest_sequence = acked_sequence;
    for record in records {
        if record.sequence <= newest_sequence {
            continue;
        }
        newer.push(record);
        newest_sequence = record.sequence;
    }
    newer
}

/// Refuses, with `RRS-S2004`, a record that run `run_id`'s log may not carry: one that names another run, one whose
/// payload only the daemon's log carries, or a checkpoint or artifact that would reach rows outside the run. Each
/// would let the job change rows outside its own run.
fn check_run_log_record(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    record: &EventRecord,
) -> Result<(), StoreError> {
    if record.run_id != run_id {
        let reason = format!("it names run {:?}", record.run_id);
        return Err(refused_record(run_id, record, reason));
    }
    let Some(payload) = record.payload.as_ref().and_then(|payload| payload.payload.as_ref()) else {
        return Ok(());
    };
    if !run_log_may_carry(payload) {
        let reason = format!(
            "{} is an event only the daemon's log carries",
            payload_event_type(payload)
        );
        return Err(refused_record(run_id, record, reason));
    }
    match payload {
        Payload::CheckpointSaved(checkpoint) => check_checkpoint(transaction, run_id, record, checkpoint),
        Payload::ArtifactCreated(artifact) => check_artifact(transaction, run_id, record, artifact),
        _ => Ok(()),
    }
}

/// The payloads a run's log may carry (contracts 7.1): what the worker, a plugin's code in the job and the chain
/// driver write. Every other payload is the daemon's (tags, pins, node, plugin and document states, finished
/// commands), the node's (job states, which reach the daemon by heartbeat and its own log) or ephemeral (progress,
/// log lines and heartbeats are never written to a log).
fn run_log_may_carry(payload: &Payload) -> bool {
    match payload {
        Payload::RunStateChanged(_)
        | Payload::RunMetrics(_)
        | Payload::CheckpointSaved(_)
        | Payload::ArtifactCreated(_)
        | Payload::DiagnosticReported(_)
        | Payload::PluginEvent(_) => true,
        Payload::JobStateChanged(_)
        | Payload::JobProgress(_)
        | Payload::JobLogLine(_)
        | Payload::RunTagsChanged(_)
        | Payload::PinChanged(_)
        | Payload::NodeHeartbeat(_)
        | Payload::NodeStateChanged(_)
        | Payload::DocumentChanged(_)
        | Payload::CommandFinished(_)
        | Payload::PluginStateChanged(_) => false,
    }
}

/// A checkpoint id must be a `ckpt_` id, so its `checkpoint_of` row can't land on a run, an artifact or any other
/// subject, and one the index already has for another run is refused: the upsert would move it to this run.
fn check_checkpoint(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    record: &EventRecord,
    checkpoint: &CheckpointSaved,
) -> Result<(), StoreError> {
    if !id_has_prefix(&checkpoint.checkpoint_id, IdPrefix::Checkpoint) {
        let reason = format!("{:?} is not a checkpoint id", checkpoint.checkpoint_id);
        return Err(refused_record(run_id, record, reason));
    }
    match read_checkpoint_run_id(transaction, &checkpoint.checkpoint_id)? {
        Some(owner) if owner != run_id => {
            let reason = format!("checkpoint {:?} belongs to run {owner:?}", checkpoint.checkpoint_id);
            Err(refused_record(run_id, record, reason))
        }
        _ => Ok(()),
    }
}

/// An artifact id must be an `art_` id and each parent's relation one of `RUN_LOG_PARENT_RELATIONS`. A new artifact
/// is the run's own, so its row and its lineage rows (whose child is that artifact) stay inside the run. An existing
/// one may have been made by another run or the daemon, so it is accepted only when the record would change nothing.
fn check_artifact(
    transaction: &SqliteTransaction<'_>,
    run_id: &str,
    record: &EventRecord,
    artifact: &ArtifactCreated,
) -> Result<(), StoreError> {
    if !id_has_prefix(&artifact.artifact_id, IdPrefix::Artifact) {
        let reason = format!("{:?} is not an artifact id", artifact.artifact_id);
        return Err(refused_record(run_id, record, reason));
    }
    for parent in &artifact.parents {
        if !RUN_LOG_PARENT_RELATIONS.contains(&parent.relation.as_str()) {
            let reason = format!(
                "parent {:?} of artifact {:?} has the relation {:?}, which a run's log may not give",
                parent.id, artifact.artifact_id, parent.relation
            );
            return Err(refused_record(run_id, record, reason));
        }
    }
    let created = created_artifact(record, artifact);
    let unchanged = match compare_stored_artifact(transaction, &created)? {
        StoredArtifact::Missing => return Ok(()),
        StoredArtifact::Same => artifact_rows_already_written(transaction, artifact)?,
        StoredArtifact::Different => false,
    };
    if !unchanged {
        let reason = format!(
            "artifact {:?} already exists and the record would change it",
            artifact.artifact_id
        );
        return Err(refused_record(run_id, record, reason));
    }
    Ok(())
}

/// Whether the home copy and every parent's lineage row of an existing artifact are already recorded, so applying
/// its record again adds no row.
fn artifact_rows_already_written(
    transaction: &SqliteTransaction<'_>,
    artifact: &ArtifactCreated,
) -> Result<bool, StoreError> {
    if !has_artifact_copy(transaction, &artifact.artifact_id, &artifact.home_node_id)? {
        return Ok(false);
    }
    for parent in &artifact.parents {
        if !has_lineage(transaction, &parent.id, &artifact.artifact_id, &parent.relation)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn refused_record(run_id: &str, record: &EventRecord, reason: String) -> StoreError {
    StoreError::RunLogRecordRefused {
        run_id: run_id.to_string(),
        sequence: record.sequence,
        reason,
    }
}

/// The event type a payload stands for (contracts 7.3), for the refusal's reason. The record's own `event_type` is
/// the writer's word; the payload is what would make the rows.
fn payload_event_type(payload: &Payload) -> &'static str {
    match payload {
        Payload::JobStateChanged(_) => "job.state_changed",
        Payload::JobProgress(_) => "job.progress",
        Payload::JobLogLine(_) => "job.log_line",
        Payload::RunStateChanged(_) => "run.state_changed",
        Payload::RunMetrics(_) => "run.metrics",
        Payload::RunTagsChanged(_) => "run.tags_changed",
        Payload::CheckpointSaved(_) => "checkpoint.saved",
        Payload::ArtifactCreated(_) => "artifact.created",
        Payload::PinChanged(_) => "pin.changed",
        Payload::NodeHeartbeat(_) => "node.heartbeat",
        Payload::NodeStateChanged(_) => "node.state_changed",
        Payload::DocumentChanged(_) => "document.changed",
        Payload::CommandFinished(_) => "command.finished",
        Payload::PluginStateChanged(_) => "plugin.state_changed",
        Payload::DiagnosticReported(_) => "diagnostic.reported",
        Payload::PluginEvent(_) => "a plugin event",
    }
}

/// `jobs`: state, exit code and error code. A record that names no known state changes nothing.
fn index_job_state(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    change: &JobStateChanged,
) -> Result<(), StoreError> {
    let Some(state) = known_job_state(change.to_state) else {
        return Ok(());
    };
    let error_code = match &change.diagnostic {
        Some(diagnostic) => diagnostic.code.clone(),
        None => String::new(),
    };
    let update = JobStateUpdate {
        job_id: change.job_id.clone(),
        run_id: record.run_id.clone(),
        state: job_state_name(state).to_string(),
        exit_code: change.exit_code.map(i64::from),
        error_code,
    };
    upsert_job_state(transaction, &update)
}

/// `runs.state`, and `finished_unix_ms` (the record's time) when the state ends the run.
fn index_run_state(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    change: &RunStateChanged,
) -> Result<(), StoreError> {
    let Some((state_name, ends_the_run)) = run_state_of(change) else {
        return Ok(());
    };
    let finished_unix_ms = if ends_the_run {
        Some(record.emitted_unix_ms)
    } else {
        None
    };
    set_run_state(transaction, &record.run_id, state_name, finished_unix_ms)
}

/// One `run_metrics` row per value, keyed by the record's sequence.
fn index_run_metrics(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    metrics: &RunMetrics,
) -> Result<(), StoreError> {
    for (name, value) in &metrics.values {
        let metric = MetricValue {
            run_id: &record.run_id,
            sequence: record.sequence,
            step: metrics.step,
            name,
            value: *value,
            emitted_unix_ms: record.emitted_unix_ms,
        };
        upsert_run_metric(transaction, &metric)?;
    }
    Ok(())
}

/// `checkpoints`, and the `checkpoint_of` lineage link from the run.
fn index_checkpoint(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    checkpoint: &CheckpointSaved,
) -> Result<(), StoreError> {
    let saved = SavedCheckpoint {
        checkpoint_id: &checkpoint.checkpoint_id,
        run_id: &record.run_id,
        step: checkpoint.step,
        folder: &checkpoint.folder,
        is_best: checkpoint.is_best,
    };
    upsert_checkpoint(transaction, &saved)?;
    insert_lineage(
        transaction,
        &record.run_id,
        &checkpoint.checkpoint_id,
        CHECKPOINT_OF_RELATION,
    )
}

/// `artifacts`, the home node's `artifact_copies` row, and one `lineage` row per parent.
fn index_artifact(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    artifact: &ArtifactCreated,
) -> Result<(), StoreError> {
    let created = created_artifact(record, artifact);
    upsert_artifact(transaction, &created)?;
    for parent in &artifact.parents {
        insert_lineage(transaction, &parent.id, &artifact.artifact_id, &parent.relation)?;
    }
    Ok(())
}

/// The artifact row a record makes; its time is the record's.
fn created_artifact<'a>(record: &EventRecord, artifact: &'a ArtifactCreated) -> CreatedArtifact<'a> {
    CreatedArtifact {
        artifact_id: &artifact.artifact_id,
        kind: &artifact.kind,
        content_hash: &artifact.content_hash,
        size_bytes: artifact.size_bytes,
        home_node_id: &artifact.home_node_id,
        created_unix_ms: record.emitted_unix_ms,
    }
}

/// `pinned` on the run, checkpoint or artifact the record names; another subject kind changes nothing.
fn index_pin(transaction: &SqliteTransaction<'_>, pin: &PinChanged) -> Result<(), StoreError> {
    match pin.subject_kind.as_str() {
        PIN_SUBJECT_RUN => set_run_pinned(transaction, &pin.subject_id, pin.pinned),
        PIN_SUBJECT_CHECKPOINT => set_checkpoint_pinned(transaction, &pin.subject_id, pin.pinned),
        PIN_SUBJECT_ARTIFACT => set_artifact_pinned(transaction, &pin.subject_id, pin.pinned),
        _ => Ok(()),
    }
}

fn index_node_state(transaction: &SqliteTransaction<'_>, change: &NodeStateChanged) -> Result<(), StoreError> {
    match NodeState::try_from(change.to_state) {
        Ok(state) if state != NodeState::Unspecified => {
            set_node_state(transaction, &change.node_id, node_state_name(state))
        }
        _ => Ok(()),
    }
}

/// `documents`: the new content hash, at the record's time.
fn index_document(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    change: &DocumentChanged,
) -> Result<(), StoreError> {
    set_document_hash(
        transaction,
        &change.document_id,
        &change.content_hash,
        record.emitted_unix_ms,
    )
}

/// `command_journal`'s finished columns, at the record's time.
fn index_command_finished(
    transaction: &SqliteTransaction<'_>,
    record: &EventRecord,
    finished: &CommandFinished,
) -> Result<(), StoreError> {
    let command = FinishedCommand {
        journal_id: &finished.journal_id,
        command_id: &finished.command_id,
        principal: &finished.principal,
        ok: finished.ok,
        code: &finished.code,
        finished_unix_ms: record.emitted_unix_ms,
    };
    record_command_finished(transaction, &command)
}

/// The job state a record moves to, or `None` for a number this build doesn't know or `UNSPECIFIED`.
fn known_job_state(number: i32) -> Option<JobState> {
    match JobState::try_from(number) {
        Ok(JobState::Unspecified) | Err(_) => None,
        Ok(state) => Some(state),
    }
}

/// The new state's name and whether it ends the run, for a training run's or an eval run's state; `None` when the
/// record carries no state this build knows.
fn run_state_of(change: &RunStateChanged) -> Option<(&'static str, bool)> {
    match change.state? {
        State::RunState(number) => match RunState::try_from(number) {
            Ok(RunState::Unspecified) | Err(_) => None,
            Ok(state) => Some((run_state_name(state), RUN_STATES_THAT_END_A_RUN.contains(&state))),
        },
        State::EvalRunState(number) => match EvalRunState::try_from(number) {
            Ok(EvalRunState::Unspecified) | Err(_) => None,
            Ok(state) => Some((eval_run_state_name(state), is_terminal_eval_run_state(state))),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;

    use rastai_proto::events::v1::{ArtifactParent, EventPayload};

    use super::*;
    use crate::index::reads::{list_table_columns, list_table_rows, open_index_reader};
    use crate::index::runs::{RunRow, upsert_run};
    use crate::reader_pool::{ReaderPool, open_reader_pool};
    use crate::sqlite::{SqliteConnection, begin_transaction, commit_transaction, open_database_for_writing};
    use crate::store::{INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS};

    const RUN_ID: &str = "run_a";

    fn test_folder(test_name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("rastai-store-indexer-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn record(sequence: u64, payload: Payload) -> EventRecord {
        EventRecord {
            format_version: 1,
            sequence,
            run_id: RUN_ID.to_string(),
            emitted_unix_ms: 100 + sequence as i64,
            payload: Some(EventPayload { payload: Some(payload) }),
            ..Default::default()
        }
    }

    fn checkpoint(sequence: u64, checkpoint_id: &str, is_best: bool) -> EventRecord {
        let saved = CheckpointSaved {
            checkpoint_id: checkpoint_id.to_string(),
            step: sequence,
            folder: format!("checkpoints/{checkpoint_id}"),
            is_best,
            ..Default::default()
        };
        record(sequence, Payload::CheckpointSaved(saved))
    }

    fn records() -> Vec<EventRecord> {
        let mut values = BTreeMap::new();
        values.insert("train.loss".to_string(), 0.5);
        let artifact = ArtifactCreated {
            artifact_id: "art_a".to_string(),
            kind: "dataset_snapshot".to_string(),
            content_hash: "blake3:01".to_string(),
            size_bytes: 3,
            home_node_id: "node_local".to_string(),
            parents: vec![ArtifactParent {
                id: "conv_a".to_string(),
                relation: "dataset_of".to_string(),
            }],
        };
        vec![
            record(1, Payload::RunMetrics(RunMetrics { step: 1, values })),
            checkpoint(2, "ckpt_a", true),
            record(3, Payload::ArtifactCreated(artifact)),
        ]
    }

    fn open_index(folder: &std::path::Path) -> (SqliteConnection, ReaderPool) {
        let path = folder.join("rastai.db");
        let connection = open_database_for_writing(&path, &INDEX_DATABASE_SETTINGS, INDEX_MIGRATIONS).unwrap();
        let transaction = begin_transaction(&connection).unwrap();
        let run = RunRow {
            run_id: RUN_ID.to_string(),
            kind: "train".to_string(),
            state: "running".to_string(),
            project_id: String::new(),
            node_id: "node_local".to_string(),
            parent_run_id: String::new(),
            record_hash: String::new(),
            acked_sequence: 0,
            created_unix_ms: 1,
            finished_unix_ms: None,
            pinned: false,
            archive_state: String::new(),
        };
        upsert_run(&transaction, &run).unwrap();
        commit_transaction(transaction).unwrap();
        let pool = open_reader_pool(&path, 1).unwrap();
        (connection, pool)
    }

    fn apply_all(connection: &SqliteConnection, records: &[EventRecord]) {
        let transaction = begin_transaction(connection).unwrap();
        for record in records {
            apply_event_record(&transaction, record).unwrap();
        }
        commit_transaction(transaction).unwrap();
    }

    fn every_row(pool: &ReaderPool) -> Vec<Vec<Vec<String>>> {
        let reader = open_index_reader(pool).unwrap();
        let mut tables = Vec::new();
        for table in list_table_columns(&reader).unwrap() {
            tables.push(list_table_rows(&reader, &table.table_name).unwrap());
        }
        tables
    }

    // Without the ack's help: the rebuild calls apply_event_record on its own, so the upserts alone must absorb a
    // record applied twice.
    #[test]
    fn a_record_applied_twice_leaves_the_same_rows() {
        let folder = test_folder("twice");
        let (connection, pool) = open_index(&folder);
        apply_all(&connection, &records());
        let rows_after_first = every_row(&pool);

        apply_all(&connection, &records());

        assert_eq!(every_row(&pool), rows_after_first);
        drop(pool);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_new_best_checkpoint_takes_the_place_of_the_old_one() {
        let folder = test_folder("best");
        let (connection, pool) = open_index(&folder);

        apply_all(
            &connection,
            &[checkpoint(1, "ckpt_a", true), checkpoint(2, "ckpt_b", true)],
        );

        let reader = open_index_reader(&pool).unwrap();
        let rows = list_table_rows(&reader, "checkpoints").unwrap();
        // Column 5 is is_best (contracts 8.2), and rows sort by checkpoint_id.
        assert_eq!(rows[0][5], "0");
        assert_eq!(rows[1][5], "1");
        drop(reader);
        drop(pool);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_record_without_a_payload_makes_no_rows() {
        let folder = test_folder("no-payload");
        let (connection, pool) = open_index(&folder);
        let rows_before = every_row(&pool);
        let empty = EventRecord {
            sequence: 1,
            run_id: RUN_ID.to_string(),
            ..Default::default()
        };

        apply_all(&connection, &[empty]);

        assert_eq!(every_row(&pool), rows_before);
        drop(pool);
        drop(connection);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
