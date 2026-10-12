"""WorkerJobEvents: the JobEvents a job emits through, over the run's event log and the live stream (python layout
1.5 and 7.1).

Engines and plugin jobs emit `(event_type, dict)` and never see protobuf (python layout 1.4 rule 7):
- `emit_durable` appends to `events.log` through the one EventLogWriter. Metrics are batched; state changes,
  checkpoints, artifacts and diagnostics are fsynced at once.
- `emit_ephemeral` puts a LiveEvent on the LiveStream for WorkerService.WatchLive. It may be dropped.

Batched metrics are fsynced at least once a second even when the job goes quiet (contracts 7.1; platform flow 6.2):
the flusher thread (`start_flusher`, `stop_flusher`) calls the writer's `flush_if_due` every
FLUSH_CHECK_SECONDS. The writer isn't thread-safe, so the job's thread and the flusher both take `log_lock` around
every call into it. An append that waits out a full disk holds the lock meanwhile; its pause records were fsynced,
so the flusher has nothing to do until RESUME.

A full disk pauses the run instead of failing it (`pause_for_full_disk`), and the append is tried again after
RESUME. A job that has been asked to end doesn't wait for RESUME: it frees the reserve and tries the append once
more, so its final records can still land (platform flow 6.1). Only a write that failed is tried again: when the
frame was written and its fsync failed (even with ENOSPC), the record is already in the file, so a retry would
repeat it. When the log can't be written at all (any failure but a full disk, a failed fsync, the pause records
failing too, or an append that fails after the reserve was freed), the writer is
marked unwritable and the error goes up: the job can't record anything more, so the edge (diagnostics.py) exits with
EXIT_EVENT_LOG_UNWRITABLE (4) instead of trying to log the failure.

Only the job's own thread emits; the flusher only fsyncs, and gRPC threads only read the live stream.
"""

import threading

from rastai_plugin.catalog.generated import RRS_S3002
from rastai_proto.events.v1.payloads_pb2 import EventPayload
from rastai_proto.node.v1.live_pb2 import LiveEvent

from rastai_plugin import JobControl, JobEvents
from rastai_worker.control import job_is_ending
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import (
    METRICS_FSYNC_INTERVAL_SECONDS,
    EventLogWriter,
    current_unix_ms,
    pause_for_full_disk,
)
from rastai_worker.event_payloads import payload_from_dict
from rastai_worker.live_stream import LiveStream

# Batched and fsynced at least once a second; every other durable event is fsynced at once (python layout 7.5).
BATCHED_EVENT_TYPES = ("run.metrics",)
# The ephemeral event types a worker emits, and the LiveEvent oneof field that carries each (contracts 4.4 and 7.3).
# The field names are EventPayload's too, so the payload converts with payload_from_dict like a durable one.
LIVE_EVENT_PAYLOAD_FIELDS = {
    "job.progress": "job_progress",
}
# How often the flusher looks for batched metrics to fsync. A quarter of the fsync interval, so metrics wait at most
# about 1.25 s for their fsync instead of up to 2 s when a check lands just before one is due.
FLUSH_CHECK_SECONDS = METRICS_FSYNC_INTERVAL_SECONDS / 4
FLUSHER_THREAD_NAME = "rastai-metrics-flusher"


class WorkerJobEvents(JobEvents):
    """A job's events: durable ones to the run's log, ephemeral ones to the live stream."""

    def __init__(self, log: EventLogWriter, live: LiveStream, control: JobControl, job_id: str) -> None:
        self.log = log
        self.live = live
        self.control = control
        self.job_id = job_id
        # Set once the log can't be appended any more; the edge then exits 4 without writing.
        self.unwritable_error: WorkerError | None = None
        # The job's thread and the flusher both call the writer, which isn't thread-safe.
        self.log_lock = threading.Lock()
        self.flusher_stop = threading.Event()
        self.flusher: threading.Thread | None = None

    def emit_durable(self, event_type: str, payload: dict, blob_hashes: list[str] | None = None) -> None:
        """Appends one event to the run's log. An event type a worker doesn't emit or a payload that doesn't match
        its message raises ValueError: a bug in the calling code."""
        record_payload = payload_from_dict(event_type, payload)
        fsync_now = event_type not in BATCHED_EVENT_TYPES
        self.append(event_type, record_payload, blob_hashes or [], fsync_now)

    def emit_ephemeral(self, event_type: str, payload: dict) -> None:
        """Puts one live event on the stream for WatchLive."""
        field_name = LIVE_EVENT_PAYLOAD_FIELDS.get(event_type)
        if field_name is None:
            raise ValueError(f"a worker doesn't stream the event type {event_type!r} live (contracts 7.3)")
        event_payload = payload_from_dict(event_type, payload)
        live_event = LiveEvent(job_id=self.job_id, emitted_unix_ms=current_unix_ms())
        getattr(live_event, field_name).CopyFrom(getattr(event_payload, field_name))
        self.live.put(event_type, live_event)

    @property
    def log_is_unwritable(self) -> bool:
        return self.unwritable_error is not None

    def append(self, event_type: str, record_payload: EventPayload, blob_hashes: list[str], fsync_now: bool) -> None:
        """Appends, making room on a full disk (see make_room_or_give_up) and trying again when the frame wasn't
        written. Raises the log's WorkerError, with the writer marked unwritable, when the log can't take the record
        or its fsync failed."""
        with self.log_lock:
            if self.unwritable_error is not None:
                raise self.unwritable_error
            while True:
                sequence_before = self.log.next_sequence
                try:
                    self.log.append(event_type, record_payload, blob_hashes, fsync_now)
                    return
                except WorkerError as error:
                    if self.log.next_sequence != sequence_before:
                        # The frame is in the file and only its fsync failed: appending again would write the
                        # record twice, and after a failed fsync Linux may already have dropped the dirty pages,
                        # so a second fsync proves nothing. Give up, the same as the flusher does.
                        self.give_up(error)
                    self.make_room_or_give_up(error)

    def make_room_or_give_up(self, error: WorkerError) -> None:
        """After a failed append: returns when the append is worth trying again, raises when it isn't.
        - Any failure but a full disk can't be waited out: give up.
        - A job asked to end frees the reserve so its final records fit, once; if the reserve is already gone (freed
          by a pause or an earlier try), there is nothing left to free: give up.
        - Otherwise pause the run until RESUME and space come back."""
        if error.code != RRS_S3002:
            self.give_up(error)
        if job_is_ending(self.control):
            if not self.log.reserve_path.exists():
                self.give_up(error)
            self.log.reserve_path.unlink(missing_ok=True)
            return
        try:
            pause_for_full_disk(self.log, self.control)
        except WorkerError as pause_error:
            self.give_up(pause_error)

    def give_up(self, error: WorkerError) -> None:
        """Marks the log unwritable and raises: every later append raises the same error at once."""
        self.unwritable_error = error
        raise error

    # ---- The flusher ----

    def start_flusher(self) -> None:
        """Starts the thread that fsyncs batched metrics while the job is quiet. Call stop_flusher before the log
        closes."""
        self.flusher = threading.Thread(target=self.flush_until_stopped, name=FLUSHER_THREAD_NAME, daemon=True)
        self.flusher.start()

    def stop_flusher(self) -> None:
        self.flusher_stop.set()
        if self.flusher is not None:
            self.flusher.join()

    def flush_until_stopped(self) -> None:
        while not self.flusher_stop.wait(FLUSH_CHECK_SECONDS):
            if not self.flush_if_due():
                return

    def flush_if_due(self) -> bool:
        """fsyncs the batched metrics once they are due. A failed fsync marks the log unwritable, so the job's next
        append (at the latest its final record) raises and the edge exits 4. False once the log is unwritable."""
        with self.log_lock:
            if self.unwritable_error is not None:
                return False
            try:
                self.log.flush_if_due()
            except WorkerError as error:
                self.unwritable_error = error
                return False
        return True
