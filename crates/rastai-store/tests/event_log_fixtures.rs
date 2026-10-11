//! The event-log reader and writer against the shared fixtures in `fixtures/event-logs/` (layout:
//! rust-control-plane.md 6.7; contracts 7.1 and 12.5): a good log, a torn tail, a corrupt middle and a sequence gap.
//! The Python worker's writer (M1-39) reads the same files, so both languages agree on the bytes.
//!
//! The fixtures are built by the recipes below and committed. `committed_fixtures_match_their_recipes` fails if the
//! frame format or the recipes change without the files; after a deliberate change, rewrite them with
//! `cargo test -p rastai-store --test event_log_fixtures -- --ignored write_event_log_fixtures`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rastai_proto::common::v1::{EventClass, RunState};
use rastai_proto::events::v1::event_payload::Payload;
use rastai_proto::events::v1::run_state_changed::State;
use rastai_proto::events::v1::{CheckpointSaved, EventPayload, EventRecord, RunMetrics, RunStateChanged};
use rastai_store::error::StoreError;
use rastai_store::event_log::frame::{EVENT_LOG_HEADER, FRAME_HEADER_BYTES, encode_frame};
use rastai_store::event_log::reader::{FrameRead, open_event_log, read_next_frame, reader_offset};
use rastai_store::event_log::writer::{
    SyncPolicy, append_event, create_event_log, open_event_log_for_append, sync_event_log, truncated_tail_bytes,
};

const FIXTURES_FOLDER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/event-logs");
const GOOD_LOG: &str = "good.log";
const TORN_TAIL_LOG: &str = "torn_tail.log";
const CORRUPT_MIDDLE_LOG: &str = "corrupt_middle.log";
const SEQUENCE_GAP_LOG: &str = "sequence_gap.log";
const ALL_FIXTURES: [&str; 4] = [GOOD_LOG, TORN_TAIL_LOG, CORRUPT_MIDDLE_LOG, SEQUENCE_GAP_LOG];

// Ids and times in the form the proto goldens use (fixtures/proto-golden/events/EventRecord.json).
const RUN_ID: &str = "run_01JA2B3C4D5E6F7G8H9J0K1M2S";
const JOB_ID: &str = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q";
const CHECKPOINT_ID: &str = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2T";
const FIRST_EMITTED_UNIX_MS: i64 = 1_791_700_000_000;
const MODEL_FILE_HASH: &str = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
/// The record whose body corrupt_middle.log damages: the second of four, so whole frames follow it.
const CORRUPTED_RECORD_INDEX: usize = 1;
/// The record sequence_gap.log leaves out: the third, so the log jumps from 2 to 4.
const MISSING_RECORD_INDEX: usize = 2;

// ---- The recipes ----

/// The four records of good.log, sequences 1 to 4: what a short training run writes.
fn good_records() -> Vec<EventRecord> {
    let payloads = vec![
        ("run.state_changed", run_running_payload()),
        ("run.metrics", metrics_payload(1, 2.31)),
        ("run.metrics", metrics_payload(2, 1.87)),
        ("checkpoint.saved", checkpoint_payload(2)),
    ];
    let mut records = Vec::new();
    for (index, (event_type, payload)) in payloads.into_iter().enumerate() {
        records.push(EventRecord {
            format_version: 1,
            sequence: index as u64 + 1,
            run_id: RUN_ID.to_string(),
            job_id: JOB_ID.to_string(),
            emitted_unix_ms: FIRST_EMITTED_UNIX_MS + 1000 * index as i64,
            event_type: event_type.to_string(),
            event_class: EventClass::Durable as i32,
            payload: Some(EventPayload { payload: Some(payload) }),
            blobs: Vec::new(),
        });
    }
    records
}

fn run_running_payload() -> Payload {
    Payload::RunStateChanged(RunStateChanged {
        reason: String::new(),
        state: Some(State::RunState(RunState::Running as i32)),
    })
}

fn metrics_payload(step: u64, loss: f64) -> Payload {
    let mut values = BTreeMap::new();
    values.insert("train.loss".to_string(), loss);
    Payload::RunMetrics(RunMetrics { step, values })
}

fn checkpoint_payload(step: u64) -> Payload {
    let mut file_hashes = BTreeMap::new();
    file_hashes.insert("model.safetensors".to_string(), MODEL_FILE_HASH.to_string());
    Payload::CheckpointSaved(CheckpointSaved {
        checkpoint_id: CHECKPOINT_ID.to_string(),
        step,
        folder: format!("checkpoints/step_{step:08}"),
        file_hashes,
        is_best: true,
    })
}

fn frames_of(records: &[EventRecord]) -> Vec<Vec<u8>> {
    records.iter().map(|record| encode_frame(record).unwrap()).collect()
}

fn log_of(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = EVENT_LOG_HEADER.to_vec();
    for frame in frames {
        bytes.extend_from_slice(frame);
    }
    bytes
}

/// Every record whole.
fn good_log() -> Vec<u8> {
    log_of(&frames_of(&good_records()))
}

/// The writer died halfway through the last frame's body.
fn torn_tail_log() -> Vec<u8> {
    let frames = frames_of(&good_records());
    let mut bytes = log_of(&frames);
    let last_body_length = frames[frames.len() - 1].len() - FRAME_HEADER_BYTES;
    bytes.truncate(bytes.len() - last_body_length / 2);
    bytes
}

/// One bit flipped in the second frame's body, with two whole frames after it: bit rot, not a crash.
fn corrupt_middle_log() -> Vec<u8> {
    let mut frames = frames_of(&good_records());
    frames[CORRUPTED_RECORD_INDEX][FRAME_HEADER_BYTES + 2] ^= 0x01;
    log_of(&frames)
}

/// Records 1, 2 and 4: the third is missing, every frame is whole.
fn sequence_gap_log() -> Vec<u8> {
    let mut records = good_records();
    records.remove(MISSING_RECORD_INDEX);
    log_of(&frames_of(&records))
}

fn fixture_recipe(name: &str) -> Vec<u8> {
    match name {
        GOOD_LOG => good_log(),
        TORN_TAIL_LOG => torn_tail_log(),
        CORRUPT_MIDDLE_LOG => corrupt_middle_log(),
        SEQUENCE_GAP_LOG => sequence_gap_log(),
        _ => panic!("no recipe for {name}"),
    }
}

// ---- Helpers ----

fn fixture_path(name: &str) -> PathBuf {
    Path::new(FIXTURES_FOLDER).join(name)
}

/// A fresh folder for one test, under the system temp folder.
fn scratch_folder(test_name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rastai-store-{}-{test_name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// A copy of a fixture in a scratch folder, for the tests that write.
fn copy_fixture(name: &str, test_name: &str) -> PathBuf {
    let path = scratch_folder(test_name).join(name);
    std::fs::copy(fixture_path(name), &path).unwrap();
    path
}

/// Reads records until something else comes back.
fn read_until_not_a_record(path: &Path) -> (Vec<EventRecord>, Result<FrameRead, StoreError>, u64) {
    let mut reader = open_event_log(path).unwrap();
    let mut records = Vec::new();
    loop {
        match read_next_frame(&mut reader) {
            Ok(FrameRead::Record(record)) => records.push(record),
            other => return (records, other, reader_offset(&reader)),
        }
    }
}

/// Where each frame of `records` ends in a log that holds exactly those frames.
fn frame_ends(records: &[EventRecord]) -> Vec<u64> {
    let mut end = EVENT_LOG_HEADER.len() as u64;
    let mut ends = Vec::new();
    for frame in frames_of(records) {
        end += frame.len() as u64;
        ends.push(end);
    }
    ends
}

// ---- The fixtures ----

#[test]
fn committed_fixtures_match_their_recipes() {
    for name in ALL_FIXTURES {
        let committed = std::fs::read(fixture_path(name)).unwrap();
        assert!(committed == fixture_recipe(name), "{name} differs from its recipe");
    }
}

/// Rewrites the committed fixtures from the recipes. Ignored, so a plain test run never changes them.
#[test]
#[ignore]
fn write_event_log_fixtures() {
    for name in ALL_FIXTURES {
        std::fs::write(fixture_path(name), fixture_recipe(name)).unwrap();
    }
}

#[test]
fn the_good_log_reads_every_record_then_end_of_file() {
    let path = fixture_path(GOOD_LOG);
    let (records, outcome, offset) = read_until_not_a_record(&path);
    assert_eq!(records, good_records());
    assert_eq!(outcome, Ok(FrameRead::EndOfFile));
    assert_eq!(offset, std::fs::metadata(&path).unwrap().len());
}

#[test]
fn a_torn_tail_reads_as_incomplete_after_the_whole_frames() {
    let (records, outcome, _) = read_until_not_a_record(&fixture_path(TORN_TAIL_LOG));
    let whole_frames_end = frame_ends(&good_records())[2];
    assert_eq!(records, good_records()[..3].to_vec());
    assert_eq!(
        outcome,
        Ok(FrameRead::Incomplete {
            valid_bytes: whole_frames_end
        })
    );
}

#[test]
fn a_corrupt_middle_reads_as_corrupt_rrs_s2002() {
    let (records, outcome, _) = read_until_not_a_record(&fixture_path(CORRUPT_MIDDLE_LOG));
    let corrupt_frame_start = frame_ends(&good_records())[0];
    assert_eq!(records, good_records()[..1].to_vec());
    assert_eq!(
        outcome,
        Ok(FrameRead::Corrupt {
            offset: corrupt_frame_start
        })
    );
    let error = StoreError::EventLogCorrupt {
        path: fixture_path(CORRUPT_MIDDLE_LOG),
        offset: corrupt_frame_start,
    };
    assert_eq!(error.code(), "RRS-S2002");
}

#[test]
fn a_sequence_gap_is_rrs_s2003() {
    let path = fixture_path(SEQUENCE_GAP_LOG);
    let (records, outcome, offset) = read_until_not_a_record(&path);
    assert_eq!(records, good_records()[..2].to_vec());
    let error = outcome.unwrap_err();
    assert_eq!(
        error,
        StoreError::EventSequenceGap {
            path,
            expected_sequence: 3,
            found_sequence: 4,
        }
    );
    assert_eq!(error.code(), "RRS-S2003");
    // The reader stays in front of the record that broke the sequence.
    assert_eq!(offset, frame_ends(&good_records())[1]);
}

// ---- The writer ----

#[test]
fn appending_after_a_torn_tail_truncates_it_first() {
    let path = copy_fixture(TORN_TAIL_LOG, "append_after_torn_tail");
    let torn_length = std::fs::metadata(&path).unwrap().len();
    let whole_frames_end = frame_ends(&good_records())[2];

    let mut writer = open_event_log_for_append(&path).unwrap();
    assert_eq!(truncated_tail_bytes(&writer), torn_length - whole_frames_end);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), whole_frames_end);

    let mut fourth = good_records()[3].clone();
    fourth.sequence = 0;
    assert_eq!(append_event(&mut writer, &mut fourth, SyncPolicy::Now).unwrap(), 4);
    drop(writer);

    // The torn bytes are gone, not left in front of the new frame: the file is now exactly good.log.
    assert_eq!(std::fs::read(&path).unwrap(), good_log());
    let (records, outcome, _) = read_until_not_a_record(&path);
    assert_eq!(records, good_records());
    assert_eq!(outcome, Ok(FrameRead::EndOfFile));
}

#[test]
fn appending_to_a_corrupt_or_gapped_log_is_refused_and_changes_nothing() {
    let corrupt = copy_fixture(CORRUPT_MIDDLE_LOG, "append_refused_corrupt");
    let error = open_event_log_for_append(&corrupt).unwrap_err();
    assert_eq!(error.code(), "RRS-S2002");
    assert_eq!(std::fs::read(&corrupt).unwrap(), corrupt_middle_log());

    let gapped = copy_fixture(SEQUENCE_GAP_LOG, "append_refused_gap");
    let error = open_event_log_for_append(&gapped).unwrap_err();
    assert_eq!(error.code(), "RRS-S2003");
    assert_eq!(std::fs::read(&gapped).unwrap(), sequence_gap_log());
}

#[test]
fn appending_to_a_good_log_continues_its_sequence() {
    let path = copy_fixture(GOOD_LOG, "append_to_good");
    let mut writer = open_event_log_for_append(&path).unwrap();
    assert_eq!(truncated_tail_bytes(&writer), 0);
    let mut fifth = good_records()[1].clone();
    assert_eq!(append_event(&mut writer, &mut fifth, SyncPolicy::Batched).unwrap(), 5);
    sync_event_log(&mut writer).unwrap();
    let (records, outcome, _) = read_until_not_a_record(&path);
    assert_eq!(records.len(), 5);
    assert_eq!(records[4].sequence, 5);
    assert_eq!(outcome, Ok(FrameRead::EndOfFile));
}

#[test]
fn a_new_log_writes_the_same_bytes_as_the_good_fixture() {
    let path = scratch_folder("new_log").join("events.log");
    let mut writer = create_event_log(&path).unwrap();
    for (index, mut record) in good_records().into_iter().enumerate() {
        // The writer, not the caller, numbers the records and stamps the format version.
        record.sequence = 99;
        record.format_version = 0;
        let sync = if index % 2 == 0 {
            SyncPolicy::Now
        } else {
            SyncPolicy::Batched
        };
        assert_eq!(append_event(&mut writer, &mut record, sync).unwrap(), index as u64 + 1);
    }
    sync_event_log(&mut writer).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), good_log());
}

#[test]
fn creating_a_log_never_overwrites_one() {
    let path = copy_fixture(GOOD_LOG, "create_refuses_existing");
    let error = create_event_log(&path).unwrap_err();
    assert_eq!(error.code(), "RRS-S3005");
    assert_eq!(std::fs::read(&path).unwrap(), good_log());
}

#[test]
fn a_header_the_writer_never_finished_is_restarted() {
    let path = scratch_folder("torn_header").join("events.log");
    std::fs::write(&path, &EVENT_LOG_HEADER[..3]).unwrap();
    let (records, outcome, _) = read_until_not_a_record(&path);
    assert!(records.is_empty());
    assert_eq!(outcome, Ok(FrameRead::Incomplete { valid_bytes: 0 }));

    let mut writer = open_event_log_for_append(&path).unwrap();
    assert_eq!(truncated_tail_bytes(&writer), 3);
    let mut first = good_records()[0].clone();
    assert_eq!(append_event(&mut writer, &mut first, SyncPolicy::Now).unwrap(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), log_of(&frames_of(&good_records()[..1])));
}

// ---- The reader's edge cases ----

#[test]
fn a_file_that_isnt_an_event_log_is_corrupt_at_offset_0() {
    let path = scratch_folder("wrong_header").join("events.log");
    std::fs::write(&path, b"NOTALOG!and some more bytes").unwrap();
    let (_, outcome, _) = read_until_not_a_record(&path);
    assert_eq!(outcome, Ok(FrameRead::Corrupt { offset: 0 }));
    assert_eq!(open_event_log_for_append(&path).unwrap_err().code(), "RRS-S2002");
}

#[test]
fn a_bad_checksum_on_the_last_frame_is_a_torn_tail() {
    // The whole frame is there but its body is damaged: a crash that kept the file size but not every block.
    let mut frames = frames_of(&good_records());
    let last = frames.len() - 1;
    frames[last][FRAME_HEADER_BYTES + 1] ^= 0x40;
    let path = scratch_folder("bad_last_checksum").join("events.log");
    std::fs::write(&path, log_of(&frames)).unwrap();
    let (records, outcome, _) = read_until_not_a_record(&path);
    assert_eq!(records.len(), 3);
    assert_eq!(
        outcome,
        Ok(FrameRead::Incomplete {
            valid_bytes: frame_ends(&good_records())[2]
        })
    );
}

#[test]
fn a_zero_filled_tail_is_a_torn_tail() {
    // A power cut can leave the end of a file as zeros. An all-zero frame header even has a matching checksum
    // (CRC-32 of nothing is 0), so the zeros are recognized as zeros, not trusted as a frame.
    let mut bytes = good_log();
    let good_length = bytes.len() as u64;
    bytes.extend_from_slice(&[0u8; 100]);
    let path = scratch_folder("zero_tail").join("events.log");
    std::fs::write(&path, &bytes).unwrap();
    let (records, outcome, _) = read_until_not_a_record(&path);
    assert_eq!(records, good_records());
    assert_eq!(
        outcome,
        Ok(FrameRead::Incomplete {
            valid_bytes: good_length
        })
    );

    let writer = open_event_log_for_append(&path).unwrap();
    assert_eq!(truncated_tail_bytes(&writer), 100);
    assert_eq!(std::fs::read(&path).unwrap(), good_log());
}

#[test]
fn a_tailer_reads_a_frame_once_the_writer_has_finished_it() {
    // A live tailer sees the torn state first, then the record, without reopening the file.
    let path = scratch_folder("tailer").join("events.log");
    let full = good_log();
    let one_frame_end = frame_ends(&good_records())[0] as usize;
    std::fs::write(&path, &full[..one_frame_end + 5]).unwrap();

    let mut reader = open_event_log(&path).unwrap();
    assert!(matches!(read_next_frame(&mut reader), Ok(FrameRead::Record(_))));
    assert_eq!(
        read_next_frame(&mut reader),
        Ok(FrameRead::Incomplete {
            valid_bytes: one_frame_end as u64
        })
    );
    std::fs::write(&path, &full).unwrap();
    let Ok(FrameRead::Record(second)) = read_next_frame(&mut reader) else {
        panic!("the finished frame should read as a record");
    };
    assert_eq!(second.sequence, 2);
}
