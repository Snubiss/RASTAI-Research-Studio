"""rastai_worker: the process every Python job runs in (layout python.md section 7).

So far it holds the run's event-log writer and the conversion from an engine's (event_type, dict) to the typed
payload (M1-39). The job process, WorkerService, the host connection and control arrive with M1-40.
"""

from rastai_worker.errors import WorkerError
from rastai_worker.event_log import EVENT_LOG_RESERVE_BYTES, METRICS_FSYNC_INTERVAL_SECONDS, EventLogWriter
from rastai_worker.event_payloads import payload_from_dict

__all__ = [
    "EVENT_LOG_RESERVE_BYTES",
    "METRICS_FSYNC_INTERVAL_SECONDS",
    "EventLogWriter",
    "WorkerError",
    "payload_from_dict",
]
