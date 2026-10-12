"""The job process from start to exit (python layout 7.2 and 7.5; platform flow 5.1).

The order matters:
1. Signal handlers first, so a SIGTERM during the start is a flag, not a dead process.
2. job.json and the job's entry: a file the worker can't use, or a kind it can't run, exits 2 before anything is
   written.
3. The run's event log: its flock is the single-writer backstop. Another live writer means exit 5 with the
   diagnostic in stderr.log only, before Register and before a byte of the log or its reserve is touched. The
   metrics flusher (job_events.py) starts once the log is open and stops before it closes.
4. WorkerService on the job socket, then Register: the node calls Hello as soon as Register arrives, so the socket
   must already answer.
5. The entry, under the edge that writes the final records and picks the exit code (diagnostics.py).

Plugin mode (plugin_module.py) runs the same steps with its own entry.
"""

import os
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_J2001, RRS_J3006, RRS_S2001, RRS_S3002, RRS_S3005
from rastai_plugin.host import JOB_TOKEN_VARIABLE

from rastai_plugin import JobControl, PluginError
from rastai_worker.control import install_control
from rastai_worker.diagnostics import (
    JobEntry,
    coded_diagnostic,
    error_line,
    print_diagnostic_line,
    run_entry_with_diagnostics,
)
from rastai_worker.dispatch import find_job_entry
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import EventLogWriter
from rastai_worker.exit_codes import EXIT_BAD_LAUNCH, EXIT_EVENT_LOG_LOCKED, EXIT_EVENT_LOG_UNWRITABLE
from rastai_worker.host_connection import HostWatcher, connect_to_host
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.job_file import WorkerLaunch, read_job_file
from rastai_worker.live_stream import LiveStream
from rastai_worker.worker_server import start_worker_server, stop_worker_server

DIAGNOSTIC_REPORTED = "diagnostic.reported"
# The log's open errors that mean it can't be written (exit 4); a refused lock is exit 5 and anything else (a log
# that can't be read, or is corrupt before its end) means the job can't start (exit 2).
UNWRITABLE_LOG_CODES = (RRS_S3002, RRS_S3005)


def run_job_process(job_file: Path) -> int:
    """`python -m rastai_worker run --job-file <job.json>`: runs the job and returns the exit code."""
    control = install_control()
    try:
        launch = read_job_file(job_file)
        entry = find_job_entry(launch.job_spec)
    except WorkerError as error:
        return report_start_failure(error, EXIT_BAD_LAUNCH)
    return run_launched_job(launch, entry, control)


def run_launched_job(launch: WorkerLaunch, entry: JobEntry, control: JobControl) -> int:
    """Steps 3 to 5 above, for a job whose launch and entry are known. Shared by run and plugin mode."""
    try:
        job_token = job_token_from_environment(launch)
        log = EventLogWriter(launch.event_log_path, launch.run_id, launch.job_id)
    except WorkerError as error:
        return report_start_failure(error, exit_code_for_start_error(error))
    try:
        return run_with_event_log(launch, entry, control, log, job_token)
    finally:
        close_event_log(log)


def run_with_event_log(
    launch: WorkerLaunch, entry: JobEntry, control: JobControl, log: EventLogWriter, job_token: str
) -> int:
    live = LiveStream()
    events = WorkerJobEvents(log, live, control, launch.job_id)
    # The flusher fsyncs batched metrics while the job is quiet; it stops before the log closes.
    events.start_flusher()
    try:
        return serve_and_run(launch, entry, control, events, job_token)
    finally:
        events.stop_flusher()


def serve_and_run(
    launch: WorkerLaunch, entry: JobEntry, control: JobControl, events: WorkerJobEvents, job_token: str
) -> int:
    try:
        report_torn_tail(events.log, events)
        server = start_worker_server(launch, control, events.live, job_token)
    except WorkerError as error:
        return report_start_failure(error, exit_code_for_start_error(error))
    try:
        return run_registered(launch, entry, control, events, job_token)
    finally:
        stop_worker_server(server, events.live)


def run_registered(
    launch: WorkerLaunch, entry: JobEntry, control: JobControl, events: WorkerJobEvents, job_token: str
) -> int:
    try:
        host = connect_to_host(launch, job_token)
    except PluginError as error:
        return report_start_failure(error, EXIT_BAD_LAUNCH)
    watcher = HostWatcher(host)
    watcher.start()
    try:
        return run_entry_with_diagnostics(entry, launch, host, events, control)
    finally:
        watcher.stop()


def job_token_from_environment(launch: WorkerLaunch) -> str:
    """The job token the node passed in RASTAI_JOB_TOKEN (contracts 3.2). It is never in job.json or a log."""
    job_token = os.environ.get(JOB_TOKEN_VARIABLE, "")
    if not job_token:
        reason = f"the worker of job {launch.job_id} has no {JOB_TOKEN_VARIABLE}, so it can't prove it is that job"
        raise WorkerError(RRS_J2001, reason)
    return job_token


def report_torn_tail(log: EventLogWriter, events: WorkerJobEvents) -> None:
    """The open cut a torn record off the end of the log (a crash mid-append): say so, as the store does
    (RRS-S2001, info)."""
    if log.truncated_tail_bytes == 0:
        return
    message = f"A torn record at the end of {log.path} was ignored ({log.truncated_tail_bytes} bytes)."
    events.emit_durable(DIAGNOSTIC_REPORTED, {"diagnostic": coded_diagnostic(RRS_S2001, message)})


def exit_code_for_start_error(error: WorkerError) -> int:
    if error.code == RRS_J3006:
        return EXIT_EVENT_LOG_LOCKED
    if error.code in UNWRITABLE_LOG_CODES:
        return EXIT_EVENT_LOG_UNWRITABLE
    return EXIT_BAD_LAUNCH


def report_start_failure(error: WorkerError | PluginError, exit_code: int) -> int:
    """A job that couldn't start has no log to tell: its reason is one line in stderr.log, which the node reads."""
    print_diagnostic_line(error_line(error))
    return exit_code


def close_event_log(log: EventLogWriter) -> None:
    """Deletes the reserve and releases the lock. The final records were fsynced when they were written, so a
    failure here only goes to stderr.log."""
    try:
        log.close()
    except WorkerError as error:
        print_diagnostic_line(error_line(error))
