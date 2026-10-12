"""A full disk and an unwritable log (event_log.pause_for_full_disk, job_events.py, diagnostics.py; platform flow
6.1; python layout 7.5 and 7.9).

A fake file that raises ENOSPC stands in for the full disk: the reserve is freed, diagnostic.reported (RRS-S3002)
and run.state_changed paused are written, the job waits for RESUME and the reserve comes back, then the event that
failed is written. A job asked to end doesn't pause: it frees the reserve and tries once more, so it still ends with
its final record. When the pause records fail too, the record fails after the reserve was freed, a written record's
fsync fails (even with ENOSPC), or the log fails for any other reason, the exit code is 4.
"""

import errno
import json
import os
import struct
import threading
import time
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_S3002, RRS_S3005
from rastai_proto.common.v1.states_pb2 import (
    RUN_STATE_CANCELLED,
    RUN_STATE_COMPLETED,
    RUN_STATE_PAUSED,
    RUN_STATE_PREEMPTED,
    RUN_STATE_RUNNING,
)
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_worker.control import apply_control_action
from rastai_worker.diagnostics import run_entry_with_diagnostics
from rastai_worker.event_log import EVENT_LOG_HEADER, EVENT_LOG_RESERVE_BYTES, FRAME_HEADER_BYTES, EventLogWriter
from rastai_worker.exit_codes import EXIT_CANCELLED, EXIT_EVENT_LOG_UNWRITABLE, EXIT_OK, EXIT_PREEMPTED
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.job_file import read_job_file
from rastai_worker.live_stream import LiveStream

from rastai_plugin import JobControl
from rastai_worker import event_log, job_events

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
FRAME_HEADER_FORMAT = "<II"
METRICS = {"step": 3, "values": {"train.loss": 1.25}}
# How long the test waits for the job's thread to reach the pause, and for it to finish after RESUME.
WAIT_SECONDS = 5.0
POLL_SECONDS = 0.01


class FullDisk:
    """Replaces event_log.write_all_at: the writes numbered in `failing_writes` (1 is the first after install)
    raise `error_number`, the others write as usual."""

    def __init__(self, failing_writes: set[int] | None, error_number: int = errno.ENOSPC) -> None:
        self.failing_writes = failing_writes
        self.error_number = error_number
        self.write_count = 0
        self.real_write_all_at = event_log.write_all_at

    def write_all_at(self, file_descriptor: int, data: bytes, offset: int) -> None:
        self.write_count += 1
        if self.failing_writes is None or self.write_count in self.failing_writes:
            raise OSError(self.error_number, "fake write failure")
        self.real_write_all_at(file_descriptor, data, offset)


class FailingFsync:
    """Replaces os.fsync: the first call after install raises ENOSPC, the others fsync as usual. The frame is
    already written by then, so only its fsync fails."""

    def __init__(self) -> None:
        self.call_count = 0
        self.real_fsync = os.fsync

    def fsync(self, file_descriptor: int) -> None:
        self.call_count += 1
        if self.call_count == 1:
            raise OSError(errno.ENOSPC, "fake fsync failure")
        self.real_fsync(file_descriptor)


def open_events(tmp_path: Path, control: JobControl) -> WorkerJobEvents:
    log = EventLogWriter(tmp_path / "events.log", RUN_ID, JOB_ID)
    return WorkerJobEvents(log, LiveStream(), control, JOB_ID)


def read_records(log_path: Path) -> list[EventRecord]:
    data = log_path.read_bytes()
    records = []
    offset = len(EVENT_LOG_HEADER)
    while offset < len(data):
        body_length, _checksum = struct.unpack_from(FRAME_HEADER_FORMAT, data, offset)
        body_start = offset + FRAME_HEADER_BYTES
        records.append(EventRecord.FromString(data[body_start : body_start + body_length]))
        offset = body_start + body_length
    return records


def wait_until(condition) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while not condition() and time.monotonic() < deadline:
        time.sleep(POLL_SECONDS)
    assert condition()


def test_a_full_disk_pauses_the_run_until_resume(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    reserve_path = events.log.reserve_path
    assert reserve_path.stat().st_size == EVENT_LOG_RESERVE_BYTES
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1}).write_all_at)
    emitter = threading.Thread(target=events.emit_durable, args=("run.metrics", METRICS), daemon=True)
    emitter.start()
    wait_until(lambda: control.paused)
    # Paused: the reserve is gone, the two pause records are on disk, and the job's thread is still waiting.
    assert not reserve_path.exists()
    paused_records = read_records(tmp_path / "events.log")
    assert [record.event_type for record in paused_records] == ["diagnostic.reported", "run.state_changed"]
    assert paused_records[0].payload.diagnostic_reported.diagnostic.code == RRS_S3002
    assert paused_records[1].payload.run_state_changed.run_state == RUN_STATE_PAUSED
    assert emitter.is_alive()
    apply_control_action(control, "RESUME")
    emitter.join(timeout=WAIT_SECONDS)
    assert not emitter.is_alive()
    # Resumed: the reserve is back, the run is running again, and the metrics that failed are written.
    assert reserve_path.stat().st_size == EVENT_LOG_RESERVE_BYTES
    records = read_records(tmp_path / "events.log")
    assert [record.event_type for record in records[2:]] == ["run.state_changed", "run.metrics"]
    assert records[2].payload.run_state_changed.run_state == RUN_STATE_RUNNING
    assert records[3].payload.run_metrics.step == METRICS["step"]
    assert not events.log_is_unwritable
    events.log.close()


def test_a_resume_before_space_is_freed_keeps_the_run_paused(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1}).write_all_at)
    real_write_reserve_file = event_log.write_reserve_file
    reserve_attempts = []

    def write_reserve_file_on_a_full_disk(reserve_path: Path) -> None:
        reserve_attempts.append(reserve_path)
        if len(reserve_attempts) == 1:
            raise event_log.write_error(reserve_path, OSError(errno.ENOSPC, "No space left on device"))
        real_write_reserve_file(reserve_path)

    monkeypatch.setattr(event_log, "write_reserve_file", write_reserve_file_on_a_full_disk)
    emitter = threading.Thread(target=events.emit_durable, args=("run.metrics", METRICS), daemon=True)
    emitter.start()
    wait_until(lambda: control.paused)
    apply_control_action(control, "RESUME")
    # The reserve can't come back yet, so the job pauses itself again and keeps waiting.
    wait_until(lambda: len(reserve_attempts) == 1 and control.paused)
    assert emitter.is_alive()
    apply_control_action(control, "RESUME")
    emitter.join(timeout=WAIT_SECONDS)
    assert not emitter.is_alive()
    assert len(reserve_attempts) == 2
    assert read_records(tmp_path / "events.log")[-1].event_type == "run.metrics"
    events.log.close()


def test_a_cancel_while_paused_ends_the_job_with_its_final_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A job cancelled on a full disk doesn't wait for RESUME: it tries the event once more and ends cancelled, with
    its final record in the room the reserve left."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1}).write_all_at)
    canceller = threading.Thread(target=cancel_once_paused, args=(control,), daemon=True)
    canceller.start()
    exit_code = run_entry_with_diagnostics(metrics_entry, launch, None, events, control)
    canceller.join()
    assert exit_code == EXIT_CANCELLED
    records = read_records(tmp_path / "events.log")
    assert [record.event_type for record in records][-2:] == ["run.metrics", "run.state_changed"]
    assert records[-1].payload.run_state_changed.run_state == RUN_STATE_CANCELLED
    events.log.close()


def test_a_cancel_while_the_disk_stays_full_exits_4(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """The same, but the event still doesn't fit: the log can't be appended even after the reserve was freed."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    # The first append fails, the two pause records fit, and the retry after CANCEL fails again.
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1, 4}).write_all_at)
    canceller = threading.Thread(target=cancel_once_paused, args=(control,), daemon=True)
    canceller.start()
    exit_code = run_entry_with_diagnostics(metrics_entry, launch, None, events, control)
    canceller.join()
    assert exit_code == EXIT_EVENT_LOG_UNWRITABLE
    assert read_records(tmp_path / "events.log")[-1].payload.run_state_changed.run_state == RUN_STATE_PAUSED
    events.log.close()


@pytest.mark.parametrize(
    ("flag_name", "expected_exit", "expected_state"),
    [
        ("preempt_requested", EXIT_PREEMPTED, (RUN_STATE_PREEMPTED, "")),
        ("cancel_requested", EXIT_CANCELLED, (RUN_STATE_CANCELLED, "")),
        ("stop_requested", EXIT_OK, (RUN_STATE_COMPLETED, "stopped")),
    ],
)
def test_an_ending_job_whose_final_record_meets_a_full_disk_frees_the_reserve(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, flag_name: str, expected_exit: int, expected_state: tuple
) -> None:
    """A job asked to end doesn't pause for RESUME: its final record fails once on the full disk, the reserve is
    freed, and the record lands in that room (platform flow 6.1; exit 4 only after the reserve was freed)."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1}).write_all_at)

    def ending_entry(job, host, entry_events, entry_control) -> None:
        setattr(entry_control, flag_name, True)

    exit_code = run_entry_with_diagnostics(ending_entry, launch, None, events, control)
    assert exit_code == expected_exit
    records = read_records(tmp_path / "events.log")
    assert [record.event_type for record in records] == ["run.state_changed"]
    final = records[0].payload.run_state_changed
    assert (final.run_state, final.reason) == expected_state
    assert not control.paused
    assert not events.log.reserve_path.exists()
    assert not events.log_is_unwritable
    events.log.close()


def test_an_ending_job_whose_final_record_fails_after_the_reserve_is_freed_exits_4(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys
) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    # The final record fails, and so does its one more try once the reserve is gone.
    monkeypatch.setattr(event_log, "write_all_at", FullDisk({1, 2}).write_all_at)

    def preempted_entry(job, host, entry_events, entry_control) -> None:
        entry_control.preempt_requested = True

    assert run_entry_with_diagnostics(preempted_entry, launch, None, events, control) == EXIT_EVENT_LOG_UNWRITABLE
    assert read_records(tmp_path / "events.log") == []
    assert not events.log.reserve_path.exists()
    assert capsys.readouterr().err.strip().splitlines()[-1].startswith(f"{RRS_S3002}: ")
    events.log.close()


def test_an_ending_job_whose_final_record_fsync_fails_writes_it_once_and_exits_4(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The final record is in the file and only its fsync fails with ENOSPC: freeing the reserve and appending again
    would repeat the record under the next sequence, so the job exits 4 with one copy."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    failing_fsync = FailingFsync()
    monkeypatch.setattr(os, "fsync", failing_fsync.fsync)

    def preempted_entry(job, host, entry_events, entry_control) -> None:
        entry_control.preempt_requested = True

    assert run_entry_with_diagnostics(preempted_entry, launch, None, events, control) == EXIT_EVENT_LOG_UNWRITABLE
    assert failing_fsync.call_count == 1
    records = read_records(tmp_path / "events.log")
    assert [record.sequence for record in records] == [1]
    assert records[0].payload.run_state_changed.run_state == RUN_STATE_PREEMPTED
    assert events.log_is_unwritable
    events.log.close()


def test_a_metrics_record_whose_fsync_fails_is_written_once_and_exits_4(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The same mid-job: a run.metrics append whose batched fsync is due fails only in the fsync. The run doesn't
    pause and append the step again (no repeated steps, platform flow 6), and no final record follows."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    # The last fsync was long ago, so this append's batched fsync is due.
    events.log.last_sync_seconds = 0.0
    failing_fsync = FailingFsync()
    monkeypatch.setattr(os, "fsync", failing_fsync.fsync)
    # A pause here would wait for a RESUME that never comes; fail at once instead (the edge reports it, exit 1).
    monkeypatch.setattr(job_events, "pause_for_full_disk", refuse_to_pause)
    assert run_entry_with_diagnostics(metrics_entry, launch, None, events, control) == EXIT_EVENT_LOG_UNWRITABLE
    assert not control.paused
    records = read_records(tmp_path / "events.log")
    assert [record.event_type for record in records] == ["run.metrics"]
    assert records[0].payload.run_metrics.step == METRICS["step"]
    assert events.log.reserve_path.exists()
    events.log.close()


def refuse_to_pause(log, control) -> None:
    raise AssertionError("the run paused, so the record it already wrote would be appended again")


def cancel_once_paused(control: JobControl) -> None:
    wait_until(lambda: control.paused)
    apply_control_action(control, "CANCEL")


def test_when_the_pause_records_fail_too_the_exit_code_is_4(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys
) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    log_before = (tmp_path / "events.log").read_bytes()
    monkeypatch.setattr(event_log, "write_all_at", FullDisk(None).write_all_at)
    exit_code = run_entry_with_diagnostics(metrics_entry, launch, None, events, control)
    assert exit_code == EXIT_EVENT_LOG_UNWRITABLE
    assert (tmp_path / "events.log").read_bytes() == log_before
    assert capsys.readouterr().err.strip().splitlines()[-1].startswith(f"{RRS_S3002}: ")
    events.log.close()


def test_an_event_log_write_that_fails_exits_4(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys) -> None:
    """Any failure but a full disk can't be waited out: no pause, no final record, exit 4 with RRS-S3005 in
    stderr.log."""
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    monkeypatch.setattr(event_log, "write_all_at", FullDisk(None, errno.EIO).write_all_at)
    exit_code = run_entry_with_diagnostics(metrics_entry, launch, None, events, control)
    assert exit_code == EXIT_EVENT_LOG_UNWRITABLE
    assert not control.paused
    assert read_records(tmp_path / "events.log") == []
    assert capsys.readouterr().err.strip().splitlines()[-1].startswith(f"{RRS_S3005}: ")
    events.log.close()


def test_an_entry_that_swallows_the_log_error_still_exits_4(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    launch = write_launch(tmp_path)
    monkeypatch.setattr(event_log, "write_all_at", FullDisk(None, errno.EIO).write_all_at)

    def swallowing_entry(job, host, entry_events, entry_control) -> None:
        try:
            entry_events.emit_durable("run.metrics", METRICS)
        except Exception:
            pass

    assert run_entry_with_diagnostics(swallowing_entry, launch, None, events, control) == EXIT_EVENT_LOG_UNWRITABLE
    events.log.close()


def test_a_log_that_writes_ends_with_exit_0(tmp_path: Path) -> None:
    control = JobControl()
    events = open_events(tmp_path, control)
    assert run_entry_with_diagnostics(metrics_entry, write_launch(tmp_path), None, events, control) == EXIT_OK
    events.log.close()


def metrics_entry(job, host, events, control) -> None:
    events.emit_durable("run.metrics", METRICS)


def write_launch(folder: Path):
    fields = {
        "job_id": JOB_ID,
        "run_id": RUN_ID,
        "job_spec": {"job_id": JOB_ID, "run_id": RUN_ID, "kind": "JOB_KIND_TRAIN"},
        "job_socket_path": str(folder / "job.sock"),
        "worker_host_socket_path": str(folder / "worker-host.sock"),
        "run_folder": str(folder),
    }
    job_file = folder / "job.json"
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return read_job_file(job_file)
