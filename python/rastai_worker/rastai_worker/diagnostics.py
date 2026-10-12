"""The worker's edge: how a job ends becomes final records, a Diagnostic and an exit code (contracts 9.1 and 10;
python layout 7.5; platform flow 5.4).

`run_entry_with_diagnostics` runs the job's entry and then writes the run's final `run.state_changed`, fsynced, so
the node and the daemon learn the outcome from the log even when they are down at that moment:

| How the entry ended | Final records | Exit |
|---|---|---|
| returned, after SIGTERM | run preempted | 75 |
| returned, after CANCEL or SIGINT | run cancelled | 3 |
| returned, after STOP | run completed, reason "stopped" | 0 |
| returned | run completed | 0 |
| raised | diagnostic.reported, then run failed | 1 |
| the log can't be written | none (stderr.log only) | 4 |

Entries never write the final run state themselves; this edge is its one writer.

A failure always becomes a coded Diagnostic, never a bare traceback (contracts 10): an error that carries a catalog
code keeps it (every package's error class has `code` and `message`), a GPU out-of-memory error is RRS-H3007, and
anything else is RRS-J1005 with the error's type. The traceback goes to stderr.log only.
"""

import sys
import traceback
from collections.abc import Callable

from rastai_plugin.catalog import find_catalog_entry
from rastai_plugin.catalog.generated import RRS_H3007, RRS_J1005

from rastai_plugin import JobControl, WorkerHost
from rastai_worker.errors import WorkerError
from rastai_worker.exit_codes import EXIT_CANCELLED, EXIT_EVENT_LOG_UNWRITABLE, EXIT_FAILED, EXIT_OK, EXIT_PREEMPTED
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.job_file import WorkerLaunch

# A catalog severity and the Diagnostic.severity enum name it becomes (protobuf JSON takes enums by name).
SEVERITY_NAMES = {"error": "SEVERITY_ERROR", "warning": "SEVERITY_WARNING", "info": "SEVERITY_INFO"}
SEVERITY_ERROR = "SEVERITY_ERROR"
# A third-party plugin's codes come from its own catalog (contracts 10), which this process doesn't load.
PLUGIN_CODE_PREFIX = "PLG-"
DIAGNOSTIC_REPORTED = "diagnostic.reported"
RUN_STATE_CHANGED = "run.state_changed"
RUN_STATE_COMPLETED = "RUN_STATE_COMPLETED"
RUN_STATE_FAILED = "RUN_STATE_FAILED"
RUN_STATE_CANCELLED = "RUN_STATE_CANCELLED"
RUN_STATE_PREEMPTED = "RUN_STATE_PREEMPTED"
# The reason of a run completed by STOP (contracts 4.4 and 9.5).
STOPPED_REASON = "stopped"
# torch's module name: its out-of-memory error (torch.OutOfMemoryError, the same class as
# torch.cuda.OutOfMemoryError) can only have been raised once torch is imported.
TORCH_MODULE_NAME = "torch"

# A job entry: run_job(job_spec, host, events, control), contracts 12.3 and python layout 1.5.
JobEntry = Callable[..., None]


def run_entry_with_diagnostics(
    entry: JobEntry, launch: WorkerLaunch, host: WorkerHost, events: WorkerJobEvents, control: JobControl
) -> int:
    """Runs the job's entry, writes the final records and returns the exit code (see the table at the top)."""
    failure = None
    try:
        # A job asked to end before it began (SIGTERM during the start) ends without running.
        if not (control.preempt_requested or control.cancel_requested):
            entry(launch.job_spec, host, events, control)
    except Exception as error:
        failure = error
    return finish_job(launch, events, control, failure)


def finish_job(launch: WorkerLaunch, events: WorkerJobEvents, control: JobControl, failure: Exception | None) -> int:
    """Writes the final records. When the log can't take them (it failed during the job, or fails now), stderr.log
    is the only place left for the reason, and the exit code is 4."""
    if failure is not None:
        print_traceback(failure)
    try:
        if failure is not None:
            return report_failure(launch, events, failure)
        return report_end(events, control)
    except WorkerError:
        if not events.log_is_unwritable:
            raise
        print_diagnostic_line(error_line(events.unwritable_error))
        return EXIT_EVENT_LOG_UNWRITABLE


def report_failure(launch: WorkerLaunch, events: WorkerJobEvents, failure: Exception) -> int:
    """The coded diagnostic and the failed run state to the log; the traceback is already in stderr.log."""
    diagnostic = diagnostic_from_error(failure, launch.job_id)
    events.emit_durable(DIAGNOSTIC_REPORTED, {"diagnostic": diagnostic, "subject_id": launch.job_id})
    events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_FAILED})
    return EXIT_FAILED


def report_end(events: WorkerJobEvents, control: JobControl) -> int:
    """The final run state of an entry that returned, from the flags it was asked to honor."""
    if control.preempt_requested:
        events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_PREEMPTED})
        return EXIT_PREEMPTED
    if control.cancel_requested:
        events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_CANCELLED})
        return EXIT_CANCELLED
    final_state = {"run_state": RUN_STATE_COMPLETED}
    if control.stop_requested:
        final_state["reason"] = STOPPED_REASON
    events.emit_durable(RUN_STATE_CHANGED, final_state)
    return EXIT_OK


# ---- Errors to Diagnostics ----


def diagnostic_from_error(error: BaseException, job_id: str) -> dict:
    """The coded Diagnostic for an error, as the dict payload_from_dict takes (proto field names, the severity by
    its enum name). The message never carries the traceback."""
    code = getattr(error, "code", None)
    message = getattr(error, "message", None)
    if isinstance(code, str) and isinstance(message, str) and is_known_code(code):
        return coded_diagnostic(code, message)
    if is_gpu_out_of_memory(error):
        return coded_diagnostic(RRS_H3007, f"Out of GPU memory during the job's step: {first_line(str(error))}")
    error_type = type(error).__name__
    return coded_diagnostic(RRS_J1005, f"The worker of job {job_id} stopped on an unexpected {error_type}.")


def coded_diagnostic(code: str, message: str) -> dict:
    """A Diagnostic with the severity and fix text of the code's catalog entry, when this process has one."""
    entry = find_catalog_entry(code)
    if entry is None:
        return {"code": code, "severity": SEVERITY_ERROR, "message": message}
    diagnostic = {"code": code, "severity": SEVERITY_NAMES.get(entry.severity, SEVERITY_ERROR), "message": message}
    if entry.fix_text:
        diagnostic["fix_text"] = entry.fix_text
    return diagnostic


def is_known_code(code: str) -> bool:
    return find_catalog_entry(code) is not None or code.startswith(PLUGIN_CODE_PREFIX)


def is_gpu_out_of_memory(error: BaseException) -> bool:
    """torch's out-of-memory error. torch is looked up among the imported modules instead of imported: a job that
    never imported it can't have raised its error, and a bad launch shouldn't pay a second for importing it."""
    torch = sys.modules.get(TORCH_MODULE_NAME)
    if torch is None:
        return False
    return isinstance(error, torch.OutOfMemoryError)


def first_line(text: str) -> str:
    lines = text.strip().splitlines()
    if not lines:
        return ""
    return lines[0]


# ---- stderr.log ----


def error_line(error: BaseException) -> dict:
    """A coded error as a diagnostic for stderr.log, keeping its own code."""
    return {"code": getattr(error, "code", RRS_J1005), "message": getattr(error, "message", str(error))}


def print_diagnostic_line(diagnostic: dict) -> None:
    """One line, `<code>: <message>`: the node reads the last line of stderr.log as the reason of a bad launch
    (RRS-J2001) and shows the lines of an exit 5 (RRS-J3006)."""
    message = " ".join(diagnostic["message"].split())
    print(f"{diagnostic['code']}: {message}", file=sys.stderr, flush=True)


def print_traceback(error: BaseException) -> None:
    traceback.print_exception(error, file=sys.stderr)
    sys.stderr.flush()
