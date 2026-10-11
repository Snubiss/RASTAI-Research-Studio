//! The Python worker's event log in Rust (contracts 7.1 and 12.5; plan M1-39): `fixtures/event-logs/from_python.log`
//! is written by `rastai_worker`'s `EventLogWriter` (its recipe is in `python/rastai_worker/tests/test_event_log.py`,
//! which checks that the writer still writes these bytes). Here the Rust reader reads it frame for frame, and the
//! Rust writer, given the same records, writes the same file byte for byte. So a run log moves between the worker,
//! the node, the daemon's mirror and the archive without a byte changing.
//!
//! The records are the ones a worker writes, chosen where Python's own protobuf serializer and prost disagree: a
//! metric of 0.0 (prost leaves a default map value out) and metric names where one is the start of another (prost
//! writes map keys in byte order). The recipe below and the Python one must change together.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rastai_proto::common::v1::{BlobRef, Diagnostic, EventClass, RunState, Severity};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{
    ArtifactCreated, ArtifactParent, CheckpointSaved, DiagnosticReported, EventPayload, EventRecord, RunMetrics,
    RunStateChanged,
};
use rastai_store::event_log::frame::{EVENT_LOG_HEADER, encode_frame};
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame, reader_offset};
use rastai_store::event_log::writer::{
    SyncPolicy, append_event, create_event_log, open_event_log_for_append, sync_event_log, truncated_tail_bytes,
};

const FROM_PYTHON_LOG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/event-logs/from_python.log");

// The ids, hashes and clock of the Python recipe (FROM_PYTHON_EVENTS in test_event_log.py).
const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2S";
const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q";
const CHECKPOINT_ID: &str = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2T";
const ARTIFACT_ID: &str = "art_01JA2B3C4D5E6F7G8H9J0K1M2V";
const FIRST_EMITTED_UNIX_MS: i64 = 1_791_700_000_000;
const EMITTED_STEP_MS: i64 = 1000;
const MODEL_FILE_HASH: &str = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
const OPTIMIZER_FILE_HASH: &str = "blake3:0c5a3e9a7d1b4f2e8c6a0b9d7e5f3a1c2b4d6e8f0a1c3e5a7b9d1f3e5c7a9b0d";
const EXPORT_HASH: &str = "blake3:7f3e9b1d5c2a8e4f6b0d9c7a5e3f1b2d4c6e8a0f2b4d6c8e0a2c4e6b8d0f2a4c";
const EXPORT_SIZE_BYTES: u64 = 1_048_576;
const TORN_TAIL_MESSAGE: &str = "Cut 138 bytes off the torn tail of events.log \u{2014} the writer died mid-write.";

// ---- The recipe ----

/// The seven records of from_python.log, sequences 1 to 7, with the blobs the worker attached and whether it
/// fsynced at once (state changes, checkpoints, artifacts and diagnostics) or batched (metrics).
fn python_records() -> Vec<(EventRecord, SyncPolicy)> {
    let events: Vec<(&str, Payload, &[&str], SyncPolicy)> = vec![
        ("run.state_changed", run_running(), &[], SyncPolicy::Now),
        ("run.metrics", first_metrics(), &[], SyncPolicy::Batched),
        ("run.metrics", second_metrics(), &[], SyncPolicy::Batched),
        ("checkpoint.saved", checkpoint_saved(), &[], SyncPolicy::Now),
        ("artifact.created", artifact_created(), &[EXPORT_HASH], SyncPolicy::Now),
        ("diagnostic.reported", torn_tail_diagnostic(), &[], SyncPolicy::Now),
        ("run.state_changed", run_stopped(), &[], SyncPolicy::Now),
    ];
    let mut records = Vec::new();
    for (index, (event_type, payload, blob_hashes, sync)) in events.into_iter().enumerate() {
        let record = EventRecord {
            format_version: 1,
            sequence: index as u64 + 1,
            run_id: RUN_ID.to_string(),
            job_id: JOB_ID.to_string(),
            emitted_unix_ms: FIRST_EMITTED_UNIX_MS + EMITTED_STEP_MS * index as i64,
            event_type: event_type.to_string(),
            event_class: EventClass::Durable as i32,
            payload: Some(EventPayload { payload: Some(payload) }),
            blobs: blob_hashes.iter().map(|hash| blob_ref(hash)).collect(),
        };
        records.push((record, sync));
    }
    records
}

fn run_running() -> Payload {
    Payload::RunStateChanged(RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(RunState::Running as i32)),
    })
}

/// A run completed by train.stop: its state alone doesn't say so, the reason does (contracts 9.5).
fn run_stopped() -> Payload {
    Payload::RunStateChanged(RunStateChanged {
        reason: "stopped".to_string(),
        state: Some(State::RunState(RunState::Completed as i32)),
    })
}

fn first_metrics() -> Payload {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), 2.31);
    values.insert("train.grad_norm".to_string(), 0.0);
    values.insert("train.learning_rate".to_string(), 0.0003);
    values.insert("classification.accuracy".to_string(), 0.25);
    values.insert("classification.accuracy_top_3".to_string(), 0.5);
    Payload::RunMetrics(RunMetrics { step: 1, values })
}

fn second_metrics() -> Payload {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), 1.87);
    values.insert("train.samples_seen".to_string(), 64.0);
    Payload::RunMetrics(RunMetrics { step: 2, values })
}

fn checkpoint_saved() -> Payload {
    let mut file_hashes = BTreeMap::new();
    file_hashes.insert("optimizer.safetensors".to_string(), OPTIMIZER_FILE_HASH.to_string());
    file_hashes.insert("model.safetensors".to_string(), MODEL_FILE_HASH.to_string());
    Payload::CheckpointSaved(CheckpointSaved {
        checkpoint_id: CHECKPOINT_ID.to_string(),
        step: 2,
        folder: "checkpoints/step_00000002".to_string(),
        file_hashes,
        is_best: false,
    })
}

fn artifact_created() -> Payload {
    Payload::ArtifactCreated(ArtifactCreated {
        artifact_id: ARTIFACT_ID.to_string(),
        kind: "checkpoint_export".to_string(),
        content_hash: EXPORT_HASH.to_string(),
        size_bytes: EXPORT_SIZE_BYTES,
        home_node_id: "node_local".to_string(),
        parents: vec![ArtifactParent {
            id: CHECKPOINT_ID.to_string(),
            relation: "exported_as".to_string(),
        }],
    })
}

fn torn_tail_diagnostic() -> Payload {
    Payload::DiagnosticReported(DiagnosticReported {
        diagnostic: Some(Diagnostic {
            code: "RRS-S2001".to_string(),
            severity: Severity::Info as i32,
            message: TORN_TAIL_MESSAGE.to_string(),
            ..Diagnostic::default()
        }),
        subject_id: RUN_ID.to_string(),
    })
}

fn blob_ref(hash: &str) -> BlobRef {
    BlobRef {
        hash: hash.to_string(),
        ..BlobRef::default()
    }
}

// ---- Helpers ----

/// A fresh folder for one test under the system temp folder, removed again when it is dropped.
struct ScratchFolder {
    path: PathBuf,
}

impl ScratchFolder {
    fn new(test_name: &str) -> ScratchFolder {
        let path = std::env::temp_dir().join(format!("rastai-store-python-{}-{test_name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        ScratchFolder { path }
    }
}

impl Drop for ScratchFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn read_every_record(path: &Path) -> (Vec<EventRecord>, FrameRead, u64) {
    let mut reader = open_event_log(path).unwrap();
    let mut records = Vec::new();
    loop {
        match read_next_frame(&mut reader).unwrap() {
            FrameRead::Record(record) => records.push(record),
            other => return (records, other, reader_offset(&reader)),
        }
    }
}

// ---- The tests ----

#[test]
fn the_python_written_log_reads_frame_for_frame() {
    let (records, outcome, offset) = read_every_record(Path::new(FROM_PYTHON_LOG));
    let expected: Vec<EventRecord> = python_records().into_iter().map(|(record, _)| record).collect();
    assert_eq!(records, expected);
    assert_eq!(outcome, FrameRead::EndOfFile);
    assert_eq!(offset, std::fs::metadata(FROM_PYTHON_LOG).unwrap().len());
}

#[test]
fn every_python_frame_is_the_frame_prost_writes() {
    // Frame by frame, so a difference names the record it is in.
    let file = std::fs::read(FROM_PYTHON_LOG).unwrap();
    let mut position = EVENT_LOG_HEADER.len();
    assert_eq!(&file[..position], EVENT_LOG_HEADER);
    for (record, _) in python_records() {
        let frame = encode_frame(&record).unwrap();
        let end = position + frame.len();
        assert!(
            file.get(position..end) == Some(frame.as_slice()),
            "record {} ({}) differs from the Python writer's frame",
            record.sequence,
            record.event_type
        );
        position = end;
    }
    assert_eq!(position, file.len());
}

#[test]
fn the_rust_writer_writes_the_same_file() {
    let scratch = ScratchFolder::new("same_file");
    let path = scratch.path.join("events.log");
    let mut writer = create_event_log(&path).unwrap();
    for (mut record, sync) in python_records() {
        let sequence = record.sequence;
        assert_eq!(append_event(&mut writer, &mut record, sync).unwrap(), sequence);
    }
    sync_event_log(&mut writer).unwrap();
    drop(writer);
    assert!(std::fs::read(&path).unwrap() == std::fs::read(FROM_PYTHON_LOG).unwrap());
}

#[test]
fn the_rust_writer_continues_a_python_written_log() {
    let scratch = ScratchFolder::new("continue");
    let path = scratch.path.join("events.log");
    std::fs::copy(FROM_PYTHON_LOG, &path).unwrap();
    let mut writer = open_event_log_for_append(&path).unwrap();
    assert_eq!(truncated_tail_bytes(&writer), 0);
    let (mut next, _) = python_records().remove(1);
    assert_eq!(append_event(&mut writer, &mut next, SyncPolicy::Now).unwrap(), 8);
    drop(writer);
    let (records, outcome, _) = read_every_record(&path);
    assert_eq!(records.len(), 8);
    assert_eq!(outcome, FrameRead::EndOfFile);
}
