"""rastai_worker: the process every Python job runs in (layout python.md section 7).

`python -m rastai_worker run --job-file <job.json>` reads the launch file, opens the run's event log (the one
writer), serves WorkerService on the job socket, registers with the node, runs the job kind's entry and maps how it
ended to final records and an exit code. The job modules (`jobs/`) land with their build steps (layout 7.11).
"""

from rastai_worker.dispatch import JOB_ENTRIES
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import (
    EVENT_LOG_RESERVE_BYTES,
    METRICS_FSYNC_INTERVAL_SECONDS,
    EventLogWriter,
    pause_for_full_disk,
)
from rastai_worker.event_payloads import payload_from_dict
from rastai_worker.exit_codes import (
    EXIT_BAD_LAUNCH,
    EXIT_CANCELLED,
    EXIT_EVENT_LOG_LOCKED,
    EXIT_EVENT_LOG_UNWRITABLE,
    EXIT_FAILED,
    EXIT_OK,
    EXIT_PREEMPTED,
)
from rastai_worker.host_connection import HOST_RECONNECT_TIMEOUT_SECONDS, REGISTER_TIMEOUT_SECONDS
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.launch import run_job_process
from rastai_worker.live_stream import LIVE_STREAM_CAPACITY, LiveStream
from rastai_worker.plugin_module import run_plugin_process
from rastai_worker.worker_server import WORKER_PROTOCOL_VERSION

__all__ = [
    "EVENT_LOG_RESERVE_BYTES",
    "EXIT_BAD_LAUNCH",
    "EXIT_CANCELLED",
    "EXIT_EVENT_LOG_LOCKED",
    "EXIT_EVENT_LOG_UNWRITABLE",
    "EXIT_FAILED",
    "EXIT_OK",
    "EXIT_PREEMPTED",
    "HOST_RECONNECT_TIMEOUT_SECONDS",
    "JOB_ENTRIES",
    "LIVE_STREAM_CAPACITY",
    "METRICS_FSYNC_INTERVAL_SECONDS",
    "REGISTER_TIMEOUT_SECONDS",
    "WORKER_PROTOCOL_VERSION",
    "EventLogWriter",
    "LiveStream",
    "WorkerError",
    "WorkerJobEvents",
    "pause_for_full_disk",
    "payload_from_dict",
    "run_job_process",
    "run_plugin_process",
]
