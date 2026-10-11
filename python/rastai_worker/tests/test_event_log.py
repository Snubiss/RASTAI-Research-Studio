"""event_log.py against the shared fixtures in fixtures/event-logs/ (contracts 7.1 and 12.5; python layout 7.9).

The Rust store's tests (crates/rastai-store/tests/event_log_fixtures.rs) built good.log, torn_tail.log,
corrupt_middle.log and sequence_gap.log. Here the Python writer writes the same bytes, gives each file the Rust
reader's verdict, and passes the Rust reader's edge cases. It also writes from_python.log, which
crates/rastai-store/tests/python_log_compat.rs reads frame for frame and writes again with the Rust writer.

After a deliberate change to the from_python.log recipe below, rewrite the file from the repository root with:
    .dev/envs/cpu/bin/python python/rastai_worker/tests/test_event_log.py
"""

import errno
import os
import random
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

import pytest
from rastai_proto.common.v1.states_pb2 import EVENT_CLASS_DURABLE, RUN_STATE_COMPLETED
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.events.v1.payloads_pb2 import EventPayload, RunStateChanged
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import (
    EVENT_LOG_HEADER,
    EVENT_LOG_RESERVE_BYTES,
    FRAME_HEADER_BYTES,
    EventLogWriter,
    encode_frame,
)
from rastai_worker.event_payloads import payload_from_dict

from rastai_worker import event_log

# flock, pread and pwrite are POSIX; Linux runs these tests in test-python (justfile).
pytestmark = pytest.mark.posix_only

EVENT_LOGS_FOLDER = Path("event-logs")
GOOD_LOG = "good.log"
TORN_TAIL_LOG = "torn_tail.log"
CORRUPT_MIDDLE_LOG = "corrupt_middle.log"
SEQUENCE_GAP_LOG = "sequence_gap.log"
FROM_PYTHON_LOG = "from_python.log"
# Where `python test_event_log.py` rewrites from_python.log: fixtures/ beside python/.
FROM_PYTHON_LOG_PATH = Path(__file__).resolve().parents[3] / "fixtures" / EVENT_LOGS_FOLDER / FROM_PYTHON_LOG
LOG_FILE_NAME = "events.log"

# The ids and clock of the Rust recipes (event_log_fixtures.rs), so the bytes are the same.
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2S"
JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q"
CHECKPOINT_ID = "ckpt_01JA2B3C4D5E6F7G8H9J0K1M2T"
FIRST_EMITTED_UNIX_MS = 1_791_700_000_000
EMITTED_STEP_MS = 1000
MODEL_FILE_HASH = "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"

# good.log's four events, as an engine emits them: what a short training run writes.
GOOD_EVENTS = [
    ("run.state_changed", {"run_state": "RUN_STATE_RUNNING"}),
    ("run.metrics", {"step": 1, "values": {"train.loss": 2.31}}),
    ("run.metrics", {"step": 2, "values": {"train.loss": 1.87}}),
    (
        "checkpoint.saved",
        {
            "checkpoint_id": CHECKPOINT_ID,
            "step": 2,
            "folder": "checkpoints/step_00000002",
            "file_hashes": {"model.safetensors": MODEL_FILE_HASH},
            "is_best": True,
        },
    ),
]

# from_python.log: every durable event type a worker writes, with the cases where Python's own protobuf serializer
# and prost disagree (a metric of 0.0, metric names where one is the start of another) and text that isn't ASCII.
# Each row: event type, payload, blob hashes, fsync at once. python_log_compat.rs builds the same records in Rust.
OPTIMIZER_FILE_HASH = "blake3:0c5a3e9a7d1b4f2e8c6a0b9d7e5f3a1c2b4d6e8f0a1c3e5a7b9d1f3e5c7a9b0d"
EXPORT_HASH = "blake3:7f3e9b1d5c2a8e4f6b0d9c7a5e3f1b2d4c6e8a0f2b4d6c8e0a2c4e6b8d0f2a4c"
FROM_PYTHON_EVENTS = [
    ("run.state_changed", {"run_state": "RUN_STATE_RUNNING"}, [], True),
    (
        "run.metrics",
        {
            "step": 1,
            "values": {
                "train.loss": 2.31,
                "train.grad_norm": 0.0,
                "train.learning_rate": 0.0003,
                "classification.accuracy": 0.25,
                "classification.accuracy_top_3": 0.5,
            },
        },
        [],
        False,
    ),
    ("run.metrics", {"step": 2, "values": {"train.loss": 1.87, "train.samples_seen": 64.0}}, [], False),
    (
        "checkpoint.saved",
        {
            "checkpoint_id": CHECKPOINT_ID,
            "step": 2,
            "folder": "checkpoints/step_00000002",
            "file_hashes": {"optimizer.safetensors": OPTIMIZER_FILE_HASH, "model.safetensors": MODEL_FILE_HASH},
        },
        [],
        True,
    ),
    (
        "artifact.created",
        {
            "artifact_id": "art_01JA2B3C4D5E6F7G8H9J0K1M2V",
            "kind": "checkpoint_export",
            "content_hash": EXPORT_HASH,
            "size_bytes": 1048576,
            "home_node_id": "node_local",
            "parents": [{"id": CHECKPOINT_ID, "relation": "exported_as"}],
        },
        [EXPORT_HASH],
        True,
    ),
    (
        "diagnostic.reported",
        {
            "diagnostic": {
                "code": "RRS-S2001",
                "severity": "SEVERITY_INFO",
                "message": "Cut 138 bytes off the torn tail of events.log — the writer died mid-write.",
            },
            "subject_id": RUN_ID,
        },
        [],
        True,
    ),
    ("run.state_changed", {"reason": "stopped", "run_state": "RUN_STATE_COMPLETED"}, [], True),
]

# The crafted tail of the CRC budget test, as in the Rust test: this 4-byte pattern, repeated over the 1 MiB search
# window, makes a frame length that fits (and so costs a CRC-32) at about half the byte offsets.
CRAFTED_TAIL_PATTERN = bytes([0x00, 0x00, 0x08, 0x00])
CRAFTED_TAIL_BYTES = 1024 * 1024
CRAFTED_TAIL_SECONDS_ALLOWED = 10
# The zeros a power cut can leave after a torn frame: one file-system block.
ZEROED_BLOCK_BYTES = 4096
# The mutation test: how many mutated copies of good.log, and the seed that makes them the same every run.
MUTATION_CASES = 300
MUTATION_SEED = "event_log/1"
MUTATION_CODES = {"RRS-S2002", "RRS-S2003"}


# ---- Helpers ----


def use_fixed_clock(patch: pytest.MonkeyPatch, first_unix_ms: int) -> None:
    """Makes the writer's clock return first_unix_ms, then one second more on each call, as the Rust recipes do."""
    times = iter(range(first_unix_ms, first_unix_ms + 10_000 * EMITTED_STEP_MS, EMITTED_STEP_MS))
    patch.setattr(event_log, "current_unix_ms", lambda: next(times))


def good_record(index: int) -> EventRecord:
    """good.log's record number index + 1, built the way the Rust recipe builds it."""
    event_type, values = GOOD_EVENTS[index]
    return EventRecord(
        format_version=1,
        sequence=index + 1,
        run_id=RUN_ID,
        job_id=JOB_ID,
        emitted_unix_ms=FIRST_EMITTED_UNIX_MS + EMITTED_STEP_MS * index,
        event_type=event_type,
        event_class=EVENT_CLASS_DURABLE,
        payload=payload_from_dict(event_type, values),
    )


def log_of(frames: list[bytes]) -> bytes:
    return EVENT_LOG_HEADER + b"".join(frames)


def good_frames() -> list[bytes]:
    return [encode_frame(good_record(index)) for index in range(len(GOOD_EVENTS))]


def frame_ends(frames: list[bytes]) -> list[int]:
    """Where each frame ends in a log that holds exactly these frames."""
    ends = []
    end = len(EVENT_LOG_HEADER)
    for frame in frames:
        end += len(frame)
        ends.append(end)
    return ends


def split_frames(log_bytes: bytes) -> list[bytes]:
    """The whole frames of a log, by their declared lengths."""
    frames = []
    position = len(EVENT_LOG_HEADER)
    while position < len(log_bytes):
        (body_length,) = struct.unpack_from("<I", log_bytes, position)
        end = position + FRAME_HEADER_BYTES + body_length
        frames.append(log_bytes[position:end])
        position = end
    return frames


def fixture_bytes(fixtures_folder: Path, name: str) -> bytes:
    return (fixtures_folder / EVENT_LOGS_FOLDER / name).read_bytes()


def write_log(folder: Path, log_bytes: bytes) -> Path:
    path = folder / LOG_FILE_NAME
    path.write_bytes(log_bytes)
    return path


def append_good_event(writer: EventLogWriter, index: int, fsync_now: bool) -> int:
    event_type, values = GOOD_EVENTS[index]
    return writer.append(event_type, payload_from_dict(event_type, values), [], fsync_now)


def write_from_python_log(path: Path) -> None:
    """The recipe of fixtures/event-logs/from_python.log: the writer appends FROM_PYTHON_EVENTS with a fixed clock."""
    with pytest.MonkeyPatch.context() as patch:
        use_fixed_clock(patch, FIRST_EMITTED_UNIX_MS)
        writer = EventLogWriter(path, RUN_ID, JOB_ID)
        for event_type, values, blob_hashes, fsync_now in FROM_PYTHON_EVENTS:
            writer.append(event_type, payload_from_dict(event_type, values), blob_hashes, fsync_now)
        writer.close()


def assert_refused_and_left_alone(folder: Path, log_bytes: bytes, code: str) -> None:
    path = write_log(folder, log_bytes)
    with pytest.raises(WorkerError) as refusal:
        EventLogWriter(path, RUN_ID, JOB_ID)
    assert refusal.value.code == code
    assert path.read_bytes() == log_bytes, "the refused open changed the file"
    assert not event_log.reserve_path_for(path).exists()


def assert_torn_and_cut(folder: Path, log_bytes: bytes, valid_bytes: int, next_sequence: int) -> None:
    path = write_log(folder, log_bytes)
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    assert writer.truncated_tail_bytes == len(log_bytes) - valid_bytes
    assert writer.next_sequence == next_sequence
    writer.close()
    assert path.read_bytes() == log_bytes[:valid_bytes]


# ---- The shared fixtures ----


def test_frames_and_checksums_equal_the_rust_fixture(fixtures_folder: Path) -> None:
    committed_frames = split_frames(fixture_bytes(fixtures_folder, GOOD_LOG))
    assert len(committed_frames) == len(GOOD_EVENTS)
    for index, committed in enumerate(committed_frames):
        assert encode_frame(good_record(index)) == committed, f"frame {index + 1} differs"
        body_length, checksum = struct.unpack_from("<II", committed)
        assert body_length == len(committed) - FRAME_HEADER_BYTES
        assert checksum == zlib.crc32(committed[FRAME_HEADER_BYTES:])


def test_a_new_log_writes_the_good_fixture_byte_for_byte(
    tmp_path: Path, fixtures_folder: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    use_fixed_clock(monkeypatch, FIRST_EMITTED_UNIX_MS)
    path = tmp_path / LOG_FILE_NAME
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    for index in range(len(GOOD_EVENTS)):
        assert append_good_event(writer, index, fsync_now=index % 2 == 0) == index + 1
    writer.close()
    assert path.read_bytes() == fixture_bytes(fixtures_folder, GOOD_LOG)


def test_the_writer_writes_the_committed_from_python_log(tmp_path: Path, fixtures_folder: Path) -> None:
    path = tmp_path / LOG_FILE_NAME
    write_from_python_log(path)
    committed = fixture_bytes(fixtures_folder, FROM_PYTHON_LOG)
    assert path.read_bytes() == committed, "from_python.log differs from its recipe; see the top of this file"
    assert len(split_frames(committed)) == len(FROM_PYTHON_EVENTS)


def test_a_torn_tail_is_truncated_on_open(
    tmp_path: Path, fixtures_folder: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    torn = fixture_bytes(fixtures_folder, TORN_TAIL_LOG)
    whole_frames_end = frame_ends(good_frames())[2]
    path = write_log(tmp_path, torn)
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    assert writer.truncated_tail_bytes == len(torn) - whole_frames_end
    assert path.stat().st_size == whole_frames_end
    # The fourth record again, at its original time: the torn bytes are gone, so the file is exactly good.log.
    use_fixed_clock(monkeypatch, FIRST_EMITTED_UNIX_MS + 3 * EMITTED_STEP_MS)
    assert append_good_event(writer, 3, fsync_now=True) == 4
    writer.close()
    assert path.read_bytes() == fixture_bytes(fixtures_folder, GOOD_LOG)


def test_a_corrupt_middle_is_rrs_s2002_and_the_file_is_left_alone(tmp_path: Path, fixtures_folder: Path) -> None:
    assert_refused_and_left_alone(tmp_path, fixture_bytes(fixtures_folder, CORRUPT_MIDDLE_LOG), "RRS-S2002")


def test_a_sequence_gap_is_rrs_s2003_and_the_file_is_left_alone(tmp_path: Path, fixtures_folder: Path) -> None:
    assert_refused_and_left_alone(tmp_path, fixture_bytes(fixtures_folder, SEQUENCE_GAP_LOG), "RRS-S2003")


def test_appending_to_a_good_log_continues_its_sequence(tmp_path: Path, fixtures_folder: Path) -> None:
    good = fixture_bytes(fixtures_folder, GOOD_LOG)
    path = write_log(tmp_path, good)
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    assert writer.truncated_tail_bytes == 0
    assert append_good_event(writer, 1, fsync_now=False) == 5
    writer.close()
    frames = split_frames(path.read_bytes())
    assert len(frames) == 5
    assert path.read_bytes().startswith(good)


# ---- One writer ----


def test_flock_blocks_a_second_writer(tmp_path: Path) -> None:
    path = tmp_path / LOG_FILE_NAME
    first = EventLogWriter(path, RUN_ID, JOB_ID)
    with pytest.raises(WorkerError) as refusal:
        EventLogWriter(path, RUN_ID, JOB_ID)
    assert refusal.value.code == "RRS-J3006"
    # The refused writer touched nothing: the first one keeps appending, and its reserve is still there.
    assert append_good_event(first, 0, fsync_now=True) == 1
    assert event_log.reserve_path_for(path).exists()
    first.close()
    second = EventLogWriter(path, RUN_ID, JOB_ID)
    assert second.next_sequence == 2
    second.close()


def test_flock_blocks_a_writer_in_another_process(tmp_path: Path) -> None:
    path = tmp_path / LOG_FILE_NAME
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    script = (
        "import sys\n"
        "from pathlib import Path\n"
        "from rastai_worker import EventLogWriter, WorkerError\n"
        "try:\n"
        "    EventLogWriter(Path(sys.argv[1]), 'run', 'job')\n"
        "except WorkerError as error:\n"
        "    print(error.code)\n"
    )
    result = subprocess.run(
        [sys.executable, "-c", script, str(path)], capture_output=True, text=True, timeout=60, check=True
    )
    writer.close()
    assert result.stdout.strip() == "RRS-J3006"


# ---- fsync policy and the reserve ----


def test_state_changes_fsync_at_once_and_metrics_at_least_once_a_second(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    writer = EventLogWriter(tmp_path / LOG_FILE_NAME, RUN_ID, JOB_ID)
    fsync_calls = []
    real_fsync = os.fsync

    def counting_fsync(file_descriptor: int) -> None:
        fsync_calls.append(file_descriptor)
        real_fsync(file_descriptor)

    monkeypatch.setattr(event_log.os, "fsync", counting_fsync)
    append_good_event(writer, 0, fsync_now=True)
    assert len(fsync_calls) == 1
    append_good_event(writer, 1, fsync_now=False)
    writer.flush_if_due()
    assert len(fsync_calls) == 1, "a batched append fsynced before a second had passed"
    # A second later, the next flush (or append) fsyncs the batch.
    writer.last_sync_seconds -= event_log.METRICS_FSYNC_INTERVAL_SECONDS
    writer.flush_if_due()
    assert len(fsync_calls) == 2
    writer.flush_if_due()
    assert len(fsync_calls) == 2, "nothing new to fsync"
    writer.last_sync_seconds -= event_log.METRICS_FSYNC_INTERVAL_SECONDS
    append_good_event(writer, 2, fsync_now=False)
    assert len(fsync_calls) == 3
    writer.close()


def test_the_reserve_exists_while_open_and_is_gone_after_close(tmp_path: Path) -> None:
    path = tmp_path / LOG_FILE_NAME
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    reserve = tmp_path / "events.log.reserve"
    assert reserve.read_bytes() == bytes(EVENT_LOG_RESERVE_BYTES)
    writer.close()
    assert not reserve.exists()
    writer.close()


def test_a_relaunch_that_takes_the_lock_as_close_releases_it_keeps_its_reserve(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / LOG_FILE_NAME
    reserve = event_log.reserve_path_for(path)
    closing = EventLogWriter(path, RUN_ID, JOB_ID)
    relaunched = []
    real_close = os.close

    def close_then_relaunch(file_descriptor: int) -> None:
        # The relaunch opens in the moment the closing writer's lock is released, before close returns.
        real_close(file_descriptor)
        if file_descriptor == closing.file_descriptor and not relaunched:
            relaunched.append(EventLogWriter(path, RUN_ID, JOB_ID))

    monkeypatch.setattr(event_log.os, "close", close_then_relaunch)
    closing.close()
    monkeypatch.undo()
    assert len(relaunched) == 1
    assert reserve.read_bytes() == bytes(EVENT_LOG_RESERVE_BYTES), "the closing writer deleted the new one's reserve"
    relaunched[0].close()
    assert not reserve.exists()


def test_a_closed_writer_refuses_to_append(tmp_path: Path) -> None:
    writer = EventLogWriter(tmp_path / LOG_FILE_NAME, RUN_ID, JOB_ID)
    writer.close()
    with pytest.raises(ValueError, match="closed"):
        append_good_event(writer, 0, fsync_now=True)


# ---- Failed writes ----


def failing_pwrite(error_number: int):
    """A pwrite that writes half of what it is given, then fails the way a full or broken disk does."""
    real_pwrite = os.pwrite

    def write_half_then_fail(file_descriptor: int, data: bytes, offset: int) -> int:
        real_pwrite(file_descriptor, bytes(data[: len(data) // 2]), offset)
        raise OSError(error_number, os.strerror(error_number))

    return write_half_then_fail


@pytest.mark.parametrize(("error_number", "code"), [(errno.ENOSPC, "RRS-S3002"), (errno.EIO, "RRS-S3005")])
def test_a_failed_append_cuts_its_partial_frame_off(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, error_number: int, code: str
) -> None:
    path = tmp_path / LOG_FILE_NAME
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    append_good_event(writer, 0, fsync_now=True)
    length_before = path.stat().st_size
    monkeypatch.setattr(event_log.os, "pwrite", failing_pwrite(error_number))
    with pytest.raises(WorkerError) as failure:
        append_good_event(writer, 1, fsync_now=True)
    assert failure.value.code == code
    assert path.stat().st_size == length_before
    monkeypatch.undo()
    # The sequence wasn't used up: the next append is record 2.
    assert append_good_event(writer, 1, fsync_now=True) == 2
    writer.close()


def test_a_log_in_a_missing_folder_is_a_failed_write(tmp_path: Path) -> None:
    with pytest.raises(WorkerError) as failure:
        EventLogWriter(tmp_path / "missing" / LOG_FILE_NAME, RUN_ID, JOB_ID)
    assert failure.value.code == "RRS-S3005"


# ---- The Rust reader's edge cases, with the same verdicts ----


def test_a_new_log_starts_with_the_header_and_sequence_1(tmp_path: Path) -> None:
    path = tmp_path / LOG_FILE_NAME
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    assert writer.next_sequence == 1
    assert writer.truncated_tail_bytes == 0
    writer.close()
    assert path.read_bytes() == EVENT_LOG_HEADER


@pytest.mark.parametrize("start_bytes", [EVENT_LOG_HEADER[:3], bytes(16)])
def test_a_header_the_writer_never_finished_is_restarted(tmp_path: Path, start_bytes: bytes) -> None:
    # A prefix of RRSEVLG1, or the zeros a power cut leaves before the header's block reached the disk.
    path = write_log(tmp_path, start_bytes)
    writer = EventLogWriter(path, RUN_ID, JOB_ID)
    assert writer.truncated_tail_bytes == len(start_bytes)
    append_good_event(writer, 0, fsync_now=True)
    writer.close()
    assert len(split_frames(path.read_bytes())) == 1
    assert path.read_bytes().startswith(EVENT_LOG_HEADER)


def test_a_file_that_is_not_an_event_log_is_corrupt(tmp_path: Path) -> None:
    assert_refused_and_left_alone(tmp_path, b"NOTALOG!and some more bytes", "RRS-S2002")


def test_a_bad_checksum_on_the_last_frame_is_a_torn_tail(tmp_path: Path) -> None:
    frames = good_frames()
    last = bytearray(frames[3])
    last[FRAME_HEADER_BYTES + 1] ^= 0x40
    frames[3] = bytes(last)
    assert_torn_and_cut(tmp_path, log_of(frames), frame_ends(frames)[2], next_sequence=4)


def test_a_damaged_length_with_whole_frames_after_it_is_corrupt(tmp_path: Path) -> None:
    log_bytes = bytearray(log_of(good_frames()))
    log_bytes[frame_ends(good_frames())[0] + 3] ^= 0x80
    assert_refused_and_left_alone(tmp_path, bytes(log_bytes), "RRS-S2002")


def test_a_damaged_length_on_the_last_frame_is_a_torn_tail(tmp_path: Path) -> None:
    ends = frame_ends(good_frames())
    log_bytes = bytearray(log_of(good_frames()))
    log_bytes[ends[2] + 3] ^= 0x80
    assert_torn_and_cut(tmp_path, bytes(log_bytes), ends[2], next_sequence=4)


def test_an_earlier_frame_after_a_damaged_last_length_is_not_proof(tmp_path: Path) -> None:
    ends = frame_ends(good_frames())
    log_bytes = bytearray(log_of(good_frames()))
    log_bytes[ends[2] + 3] ^= 0x80
    log_bytes += good_frames()[0]
    assert_torn_and_cut(tmp_path, bytes(log_bytes), ends[2], next_sequence=4)


@pytest.mark.parametrize("index", [1, 3])
def test_a_valid_checksum_on_another_format_version_is_corrupt(tmp_path: Path, index: int) -> None:
    # A matching checksum means the writer wrote these bytes, so a crash didn't: never a torn tail, even last.
    frames = good_frames()
    record = good_record(index)
    record.format_version = 2
    frames[index] = encode_frame(record)
    assert_refused_and_left_alone(tmp_path, log_of(frames), "RRS-S2002")


def test_a_zero_filled_tail_is_a_torn_tail(tmp_path: Path) -> None:
    good = log_of(good_frames())
    assert_torn_and_cut(tmp_path, good + bytes(100), len(good), next_sequence=5)


def test_a_damaged_header_and_body_start_with_whole_frames_after_it_is_corrupt(tmp_path: Path) -> None:
    second_frame_start = frame_ends(good_frames())[0]
    log_bytes = bytearray(log_of(good_frames()))
    for position in range(second_frame_start, second_frame_start + FRAME_HEADER_BYTES + 4):
        log_bytes[position] = 0xFF
    assert_refused_and_left_alone(tmp_path, bytes(log_bytes), "RRS-S2002")


def test_a_whole_header_with_a_zeroed_body_and_zeros_after_it_is_a_torn_tail(tmp_path: Path) -> None:
    ends = frame_ends(good_frames())
    log_bytes = bytearray(log_of(good_frames()))
    for position in range(ends[2] + FRAME_HEADER_BYTES, len(log_bytes)):
        log_bytes[position] = 0
    log_bytes += bytes(ZEROED_BLOCK_BYTES)
    assert_torn_and_cut(tmp_path, bytes(log_bytes), ends[2], next_sequence=4)


def test_a_zeroed_body_with_a_record_after_it_is_still_corrupt(tmp_path: Path) -> None:
    ends = frame_ends(good_frames())
    log_bytes = bytearray(log_of(good_frames()))
    for position in range(ends[1] + FRAME_HEADER_BYTES, ends[2]):
        log_bytes[position] = 0
    assert_refused_and_left_alone(tmp_path, bytes(log_bytes), "RRS-S2002")


def ascii_frame_for_sequence(sequence: int) -> bytes:
    """A whole, checksum-valid frame for `sequence` whose every byte is ASCII, so a record's text can quote it."""
    attempt = 0
    while True:
        record = EventRecord(format_version=1, sequence=sequence, run_id=f"run_quoted_{attempt}")
        record.event_type = "run.metrics"
        frame = encode_frame(record)
        if all(byte < 0x80 for byte in frame):
            return frame
        attempt += 1


def test_a_frame_quoted_inside_a_torn_record_is_not_proof(tmp_path: Path) -> None:
    # Record 4's text quotes whole frames with later sequences (5, and 4 itself). They sit inside the torn record's
    # own body, so they say nothing about its length: the frame is the torn tail, not corruption.
    quote = (ascii_frame_for_sequence(5) + ascii_frame_for_sequence(4)).decode("ascii")
    fourth = good_record(0)
    fourth.sequence = 4
    fourth.payload.CopyFrom(
        EventPayload(run_state_changed=RunStateChanged(reason=quote, run_state=RUN_STATE_COMPLETED))
    )
    frames = good_frames()[:3] + [encode_frame(fourth)]
    whole = log_of(frames)
    assert_torn_and_cut(tmp_path, whole, len(whole), next_sequence=5)
    quote_end = frames[3].index(ascii_frame_for_sequence(4)) + len(ascii_frame_for_sequence(4))
    kept = quote_end + 1
    assert kept < len(frames[3])
    torn = log_of(frames[:3]) + frames[3][:kept]
    assert_torn_and_cut(tmp_path, torn, frame_ends(frames)[2], next_sequence=4)


def test_a_crafted_tail_cannot_make_the_open_slow(tmp_path: Path) -> None:
    # A first frame that claims almost 4 GiB, then bytes whose every 4-byte step reads as a length that fits the
    # search window. Without the CRC budget one open would run for minutes.
    crafted = bytearray(EVENT_LOG_HEADER + struct.pack("<I", 0xFFFF_FFF0) + bytes(4))
    while len(crafted) < len(EVENT_LOG_HEADER) + FRAME_HEADER_BYTES + CRAFTED_TAIL_BYTES:
        crafted += CRAFTED_TAIL_PATTERN
    started = time.monotonic()
    assert_torn_and_cut(tmp_path, bytes(crafted), len(EVENT_LOG_HEADER), next_sequence=1)
    assert time.monotonic() - started < CRAFTED_TAIL_SECONDS_ALLOWED


@pytest.mark.fuzz
def test_mutated_logs_open_or_fail_with_a_log_code(tmp_path: Path, fixtures_folder: Path) -> None:
    # The open parses whatever is on disk, so it gets the seeded mutation test of python layout 1.4 rule 13: every
    # mutation either opens (after cutting a torn tail) or is refused with RRS-S2002 or RRS-S2003, never anything else.
    good = fixture_bytes(fixtures_folder, GOOD_LOG)
    generator = random.Random(MUTATION_SEED)
    for case in range(MUTATION_CASES):
        mutated = mutate(good, generator)
        path = write_log(tmp_path, mutated)
        try:
            writer = EventLogWriter(path, RUN_ID, JOB_ID)
        except WorkerError as error:
            assert error.code in MUTATION_CODES, f"case {case}: {error}"
            assert path.read_bytes() == mutated
            continue
        # The open cut a torn tail at most; a file cut back to nothing starts again with its header.
        valid_bytes = len(mutated) - writer.truncated_tail_bytes
        writer.close()
        assert path.read_bytes() == (mutated[:valid_bytes] if valid_bytes else EVENT_LOG_HEADER), f"case {case}"


def mutate(data: bytes, generator: random.Random) -> bytes:
    """One of: flip a bit, overwrite a byte, cut the file short, or insert a few random bytes."""
    mutated = bytearray(data)
    position = generator.randrange(len(mutated))
    choice = generator.randrange(4)
    if choice == 0:
        mutated[position] ^= 1 << generator.randrange(8)
    elif choice == 1:
        mutated[position] = generator.randrange(256)
    elif choice == 2:
        del mutated[position:]
    else:
        mutated[position:position] = bytes(generator.randrange(256) for _ in range(generator.randrange(1, 9)))
    return bytes(mutated)


if __name__ == "__main__":
    # Rewrites fixtures/event-logs/from_python.log from its recipe (see the top of this file).
    FROM_PYTHON_LOG_PATH.unlink(missing_ok=True)
    write_from_python_log(FROM_PYTHON_LOG_PATH)
    print(f"wrote {FROM_PYTHON_LOG_PATH}")
