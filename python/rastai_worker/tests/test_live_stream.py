"""LiveStream (live_stream.py; python layout 7.9): the oldest event is dropped at capacity; readers resume by index;
a reader waits for the next event, its timeout or the close.
"""

import threading
import time

from rastai_proto.events.v1.payloads_pb2 import JobProgress
from rastai_proto.node.v1.live_pb2 import LiveEvent
from rastai_worker.live_stream import LIVE_STREAM_CAPACITY, LiveStream

PROGRESS = "job.progress"
SHORT_WAIT_SECONDS = 0.05
LONG_WAIT_SECONDS = 5.0


def progress_event(fraction: float) -> LiveEvent:
    return LiveEvent(job_id="job_a", job_progress=JobProgress(fraction=fraction))


def indexes(entries: list) -> list[int]:
    return [index for index, _event_type, _event in entries]


def test_the_capacity_is_the_layouts() -> None:
    assert LIVE_STREAM_CAPACITY == 4096


def test_events_get_indexes_and_readers_resume_after_the_last_one_they_saw() -> None:
    live = LiveStream()
    for step in range(3):
        live.put(PROGRESS, progress_event(step / 10))
    first_read = live.read(0, SHORT_WAIT_SECONDS)
    assert indexes(first_read) == [1, 2, 3]
    assert first_read[0][1] == PROGRESS
    live.put(PROGRESS, progress_event(0.5))
    assert indexes(live.read(3, SHORT_WAIT_SECONDS)) == [4]
    assert live.read(4, SHORT_WAIT_SECONDS) == []


def test_the_oldest_event_is_dropped_at_capacity() -> None:
    live = LiveStream(capacity=3)
    for step in range(5):
        live.put(PROGRESS, progress_event(step / 10))
    entries = live.read(0, SHORT_WAIT_SECONDS)
    # A reader that fell behind sees the gap (1 and 2 are gone), never a repeat.
    assert indexes(entries) == [3, 4, 5]
    assert entries[0][2].job_progress.fraction == progress_event(0.2).job_progress.fraction


def test_a_reader_waits_for_the_next_event() -> None:
    live = LiveStream()
    results = []
    reader = threading.Thread(target=lambda: results.append(live.read(0, LONG_WAIT_SECONDS)), daemon=True)
    reader.start()
    time.sleep(SHORT_WAIT_SECONDS)
    live.put(PROGRESS, progress_event(0.1))
    reader.join(timeout=LONG_WAIT_SECONDS)
    assert indexes(results[0]) == [1]


def test_a_read_times_out_empty_and_the_close_wakes_readers() -> None:
    live = LiveStream()
    started = time.monotonic()
    assert live.read(0, SHORT_WAIT_SECONDS) == []
    assert time.monotonic() - started >= SHORT_WAIT_SECONDS
    results = []
    reader = threading.Thread(target=lambda: results.append(live.read(0, LONG_WAIT_SECONDS)), daemon=True)
    reader.start()
    time.sleep(SHORT_WAIT_SECONDS)
    live.close()
    reader.join(timeout=LONG_WAIT_SECONDS)
    assert results == [[]]
    assert live.is_closed
