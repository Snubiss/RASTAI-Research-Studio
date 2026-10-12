"""LiveStream: the job's ephemeral events, kept for WorkerService.WatchLive (contracts 4.9; python layout 7.5).

Ephemeral events (progress now; peek steps and live packets later) never touch the event log. The job's thread puts
them here and the node reads them over WatchLive. The buffer is bounded: when nobody reads, the oldest events go,
so a node that is away (restarting, or slow) can never make a job run out of memory. Each event gets the next
index, 1, 2, 3 ..., so a reader resumes after the last index it saw and sees a gap, never a repeat.
"""

import threading
from collections import deque

from rastai_proto.node.v1.live_pb2 import LiveEvent

# Ephemeral events kept for WatchLive before the oldest are dropped (python layout 7.2).
LIVE_STREAM_CAPACITY = 4096


class LiveStream:
    """A bounded, indexed buffer of live events, written by the job's thread and read by gRPC threads."""

    def __init__(self, capacity: int = LIVE_STREAM_CAPACITY) -> None:
        # (index, event_type, event), oldest first; a full deque drops from the left as it appends.
        self.entries: deque[tuple[int, str, LiveEvent]] = deque(maxlen=capacity)
        self.last_index = 0
        self.is_closed = False
        # One lock for the buffer; readers wait on it for the next put or the close.
        self.changed = threading.Condition()

    def put(self, event_type: str, event: LiveEvent) -> None:
        """Adds one event and wakes every waiting reader. Never blocks the job for long: no I/O under the lock."""
        with self.changed:
            self.last_index += 1
            self.entries.append((self.last_index, event_type, event))
            self.changed.notify_all()

    def read(self, after_index: int, timeout_seconds: float) -> list[tuple[int, str, LiveEvent]]:
        """The events after `after_index` that are still kept, oldest first. Waits up to `timeout_seconds` for one
        when there are none, and returns an empty list on the timeout or once the stream is closed."""
        with self.changed:
            self.changed.wait_for(lambda: self.has_news(after_index), timeout=timeout_seconds)
            return [entry for entry in self.entries if entry[0] > after_index]

    def close(self) -> None:
        """Ends every WatchLive stream: the job is exiting."""
        with self.changed:
            self.is_closed = True
            self.changed.notify_all()

    def has_news(self, after_index: int) -> bool:
        return self.last_index > after_index or self.is_closed
