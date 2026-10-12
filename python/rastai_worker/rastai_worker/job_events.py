"""WorkerJobEvents: the JobEvents a job emits through, over the run's event log and the live stream (python layout
1.5 and 7.1).

Engines and plugin jobs emit `(event_type, dict)` and never see protobuf (python layout 1.4 rule 7):
- `emit_durable` appends to `events.log` through the one EventLogWriter. Metrics are batched (fsynced at least once
  a second); state changes, checkpoints, artifacts and diagnostics are fsynced at once.
- `emit_ephemeral` puts a LiveEvent on the LiveStream for WorkerService.WatchLive. It may be dropped.

A full disk pauses the run instead of failing it (`pause_for_full_disk`), and the append is tried again after
RESUME. When the log can't be written at all, the writer is marked unwritable and the error goes up: the job can't
record anything more, so the edge (diagnostics.py) exits with EXIT_EVENT_LOG_UNWRITABLE (4) instead of trying to
log the failure.

Only the job's own thread emits; gRPC threads only read the live stream.
"""

from rastai_plugin.catalog.generated import RRS_S3002
from rastai_proto.events.v1.payloads_pb2 import EventPayload
from rastai_proto.node.v1.live_pb2 import LiveEvent

from rastai_plugin import JobControl, JobEvents
from rastai_worker.control import job_is_ending
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import EventLogWriter, current_unix_ms, pause_for_full_disk
from rastai_worker.event_payloads import payload_from_dict
from rastai_worker.live_stream import LiveStream

# Batched and fsynced at least once a second; every other durable event is fsynced at once (python layout 7.5).
BATCHED_EVENT_TYPES = ("run.metrics",)
# The ephemeral event types a worker emits, and the LiveEvent oneof field that carries each (contracts 4.4 and 7.3).
# The field names are EventPayload's too, so the payload converts with payload_from_dict like a durable one.
LIVE_EVENT_PAYLOAD_FIELDS = {
    "job.progress": "job_progress",
}


class WorkerJobEvents(JobEvents):
    """A job's events: durable ones to the run's log, ephemeral ones to the live stream."""

    def __init__(self, log: EventLogWriter, live: LiveStream, control: JobControl, job_id: str) -> None:
        self.log = log
        self.live = live
        self.control = control
        self.job_id = job_id
        # Set once the log can't be appended any more; the edge then exits 4 without writing.
        self.unwritable_error: WorkerError | None = None

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
        """Appends, pausing the run on a full disk and trying again after RESUME. Raises the log's WorkerError, with
        the writer marked unwritable, when the log can't take the record: any failure but a full disk, the pause
        records failing too, or a full disk once the job has been asked to end."""
        if self.unwritable_error is not None:
            raise self.unwritable_error
        while True:
            try:
                self.log.append(event_type, record_payload, blob_hashes, fsync_now)
                return
            except WorkerError as error:
                if error.code != RRS_S3002 or job_is_ending(self.control):
                    self.unwritable_error = error
                    raise
            try:
                pause_for_full_disk(self.log, self.control)
            except WorkerError as error:
                self.unwritable_error = error
                raise
