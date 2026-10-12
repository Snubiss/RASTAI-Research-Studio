"""WorkerJobEvents' metrics flusher (job_events.py; contracts 7.1; python layout 7.5; platform flow 6.2): batched
run.metrics are fsynced within about a second even when the job emits nothing more, and an fsync that fails makes
the job exit 4 instead of losing its metrics quietly.
"""

import errno
import json
import time
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_S3005
from rastai_worker.diagnostics import run_entry_with_diagnostics
from rastai_worker.event_log import METRICS_FSYNC_INTERVAL_SECONDS, EventLogWriter, write_error
from rastai_worker.exit_codes import EXIT_EVENT_LOG_UNWRITABLE
from rastai_worker.job_events import FLUSHER_THREAD_NAME, WorkerJobEvents
from rastai_worker.job_file import WorkerLaunch, read_job_file
from rastai_worker.live_stream import LiveStream

from rastai_plugin import JobControl

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
METRICS = {"step": 3, "values": {"train.loss": 1.25}}
# The flusher must have fsynced well within this; the promise is about a second.
FLUSH_WAIT_SECONDS = 3 * METRICS_FSYNC_INTERVAL_SECONDS
POLL_SECONDS = 0.01


def open_events(tmp_path: Path) -> WorkerJobEvents:
    log = EventLogWriter(tmp_path / "events.log", RUN_ID, JOB_ID)
    return WorkerJobEvents(log, LiveStream(), JobControl(), JOB_ID)


def wait_until(condition) -> bool:
    deadline = time.monotonic() + FLUSH_WAIT_SECONDS
    while not condition() and time.monotonic() < deadline:
        time.sleep(POLL_SECONDS)
    return condition()


def test_the_flusher_fsyncs_metrics_while_the_job_is_quiet(tmp_path: Path) -> None:
    events = open_events(tmp_path)
    events.start_flusher()
    try:
        # Batched: the last fsync was just now, so the append leaves its bytes unsynced.
        events.log.last_sync_seconds = time.monotonic()
        events.emit_durable("run.metrics", METRICS)
        assert events.log.has_unsynced_bytes
        # No other append comes, yet the metrics get their fsync.
        assert wait_until(lambda: not events.log.has_unsynced_bytes)
    finally:
        events.stop_flusher()
    assert not events.flusher.is_alive()
    assert events.flusher.name == FLUSHER_THREAD_NAME
    events.log.close()


def test_a_failed_flush_makes_the_job_exit_4(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys) -> None:
    events = open_events(tmp_path)

    def failing_sync() -> None:
        raise write_error(events.log.path, OSError(errno.EIO, "fake fsync failure"))

    monkeypatch.setattr(events.log, "sync", failing_sync)
    events.start_flusher()
    try:
        # The append itself doesn't fsync (the last fsync was just now); the flusher's fsync is the one that fails.
        events.log.last_sync_seconds = time.monotonic()
        events.emit_durable("run.metrics", METRICS)
        assert wait_until(lambda: events.log_is_unwritable)
        # The flusher gave up on the log, so it ends by itself.
        events.flusher.join(timeout=FLUSH_WAIT_SECONDS)
        assert not events.flusher.is_alive()
    finally:
        events.stop_flusher()
    exit_code = run_entry_with_diagnostics(quiet_entry, write_launch(tmp_path), None, events, events.control)
    assert exit_code == EXIT_EVENT_LOG_UNWRITABLE
    assert capsys.readouterr().err.strip().splitlines()[-1].startswith(f"{RRS_S3005}: ")
    monkeypatch.undo()
    events.log.close()


def test_stopping_a_flusher_that_never_started_does_nothing(tmp_path: Path) -> None:
    events = open_events(tmp_path)
    events.stop_flusher()
    assert events.flusher is None
    events.log.close()


def quiet_entry(job, host, events, control) -> None:
    pass


def write_launch(folder: Path) -> WorkerLaunch:
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
