"""The worker's half of the kill -9 tests' `RASTAI_TEST_PAUSE_AT` hook (pause_points.py; contracts 15.9): a point
pauses only when the variable names it, only once per process, and the event log's two points stop the writer where
the crash matrix needs it: after the first whole `run.metrics` frame, and halfway through the second. Here the stop
itself (SIGSTOP to this process) is replaced by a recorder; tests/crash/ stops real workers."""

from pathlib import Path

import pytest
from rastai_worker.event_log import EVENT_LOG_HEADER, EventLogWriter
from rastai_worker.event_payloads import payload_from_dict

from rastai_worker import pause_points

pytestmark = pytest.mark.posix_only

RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
AFTER_METRICS_FRAME = "worker.event_log.after_metrics_frame"
MID_METRICS_FRAME = "worker.event_log.mid_metrics_frame"


@pytest.fixture
def stops(monkeypatch: pytest.MonkeyPatch) -> list[tuple[int, int]]:
    """Records each signal the hook sends instead of sending it, and forgets the points paused at so far."""
    sent = []
    monkeypatch.setattr(pause_points.os, "kill", lambda pid, signal_number: sent.append((pid, signal_number)))
    monkeypatch.setattr(pause_points, "paused_points", set())
    return sent


def test_only_the_named_point_pauses_and_only_once(monkeypatch: pytest.MonkeyPatch, stops: list) -> None:
    pause_points.pause_at(AFTER_METRICS_FRAME)
    assert stops == []
    monkeypatch.setenv(pause_points.TEST_PAUSE_VARIABLE, AFTER_METRICS_FRAME)
    pause_points.pause_at(MID_METRICS_FRAME)
    assert stops == []
    pause_points.pause_at(AFTER_METRICS_FRAME)
    pause_points.pause_at(AFTER_METRICS_FRAME)
    assert stops == [(pause_points.os.getpid(), pause_points.signal.SIGSTOP)]


def metrics_payload(step: int):
    return payload_from_dict("run.metrics", {"step": step, "values": {"train.loss": 1.0}})


def test_the_after_point_comes_once_the_first_metrics_frame_is_whole(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, stops: list
) -> None:
    monkeypatch.setenv(pause_points.TEST_PAUSE_VARIABLE, AFTER_METRICS_FRAME)
    log = EventLogWriter(tmp_path / "events.log", RUN_ID, JOB_ID)
    sizes_at_stop = []
    monkeypatch.setattr(pause_points.os, "kill", lambda *_: sizes_at_stop.append(log.path.stat().st_size))
    log.append("run.metrics", metrics_payload(10), [], fsync_now=False)
    # Stopped with the first frame whole on disk, and never again.
    assert sizes_at_stop == [log.path.stat().st_size]
    assert sizes_at_stop[0] > len(EVENT_LOG_HEADER)
    log.append("run.metrics", metrics_payload(20), [], fsync_now=False)
    assert len(sizes_at_stop) == 1
    log.close()


def test_the_mid_point_stops_with_half_of_the_second_metrics_frame_written(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, stops: list
) -> None:
    monkeypatch.setenv(pause_points.TEST_PAUSE_VARIABLE, MID_METRICS_FRAME)
    log = EventLogWriter(tmp_path / "events.log", RUN_ID, JOB_ID)
    log.append("run.metrics", metrics_payload(10), [], fsync_now=False)
    whole_end = log.path.stat().st_size
    sizes_at_stop = []
    monkeypatch.setattr(pause_points.os, "kill", lambda *_: sizes_at_stop.append(log.path.stat().st_size))
    log.append("run.metrics", metrics_payload(20), [], fsync_now=False)
    full_end = log.path.stat().st_size
    second_frame_length = full_end - whole_end
    # Stopped with half of the second frame on disk: a kill there leaves a torn tail after one whole frame.
    assert sizes_at_stop == [whole_end + second_frame_length // 2]
    log.close()
    # Let go instead, the whole frame was written over the half, and the log reads whole.
    reopened = EventLogWriter(tmp_path / "events.log", RUN_ID, JOB_ID)
    assert (reopened.truncated_tail_bytes, reopened.next_sequence) == (0, 3)
    reopened.close()
