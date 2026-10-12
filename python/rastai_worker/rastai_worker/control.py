"""Control: signals and WorkerService.Control calls become JobControl flags (contracts 4.4; python layout 7.6).

The handlers only set flags. The job's thread (the main thread, where the model lives) polls them between steps
and decides what to do; nothing here touches the model, the log or the GPU. Each signal has its own flag, because
each ends a job differently (the JobSignal table):

- SIGTERM: `preempt_requested`, and only that one. Checkpoint if it fits, then exit 75, for every kind.
- SIGINT: `cancel_requested`, the same as Control CANCEL (the node sends it when the CANCEL call fails).
- SIGHUP: ignored. The node starts the worker in its own session, so a dead node can't hang it up.
- Control CANCEL, STOP, PAUSE, RESUME, CHECKPOINT_NOW and STEP set or clear their flags. TERMINATE and KILL are
  never Control values: they are OS signals, so any node version can stop any worker version.

CPython assigns a bool or an int atomically, so neither side takes a lock.
"""

import signal
import time

from rastai_plugin.catalog.generated import RRS_J0004

from rastai_plugin import JobControl
from rastai_worker.errors import WorkerError

# The Control values a worker takes, by their JobSignal names without the JOB_SIGNAL_ prefix.
CANCEL = "CANCEL"
STOP = "STOP"
PAUSE = "PAUSE"
RESUME = "RESUME"
CHECKPOINT_NOW = "CHECKPOINT_NOW"
STEP = "STEP"
CONTROL_ACTIONS = (CANCEL, STOP, PAUSE, RESUME, CHECKPOINT_NOW, STEP)
# How often a paused job looks at its flags again. RESUME takes effect within this time.
PAUSE_POLL_SECONDS = 0.1


def install_control() -> JobControl:
    """The job's JobControl, with the signal handlers that set its flags. Call it on the main thread: Python runs
    signal handlers only there."""
    control = JobControl()
    signal.signal(signal.SIGTERM, preempt_handler(control))
    signal.signal(signal.SIGINT, cancel_handler(control))
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    return control


def preempt_handler(control: JobControl):
    def handle(_signal_number, _frame) -> None:
        control.preempt_requested = True

    return handle


def cancel_handler(control: JobControl):
    def handle(_signal_number, _frame) -> None:
        control.cancel_requested = True

    return handle


def apply_control_action(control: JobControl, action: str) -> None:
    """Sets the flag of one Control value (its JobSignal name without the prefix). Anything that isn't one of the
    six Control values is refused with RRS-J0004 and changes nothing."""
    if action == CANCEL:
        control.cancel_requested = True
    elif action == STOP:
        control.stop_requested = True
    elif action == PAUSE:
        control.paused = True
    elif action == RESUME:
        control.step_budget = None
        control.paused = False
    elif action == CHECKPOINT_NOW:
        control.checkpoint_requested = True
    elif action == STEP:
        # One more step, then pause again. Steps add up, so two quick STEPs run two steps.
        control.step_budget = (control.step_budget or 0) + 1
        control.paused = False
    else:
        raise WorkerError(RRS_J0004, f"Signal {action} wasn't delivered to the job: it isn't a Control value")


def job_is_ending(control: JobControl) -> bool:
    """True once anything asked the job to end: STOP, CANCEL or SIGINT, or SIGTERM."""
    return control.stop_requested or control.cancel_requested or control.preempt_requested


def wait_for_resume(control: JobControl) -> None:
    """Waits while the job is paused. Returns on RESUME or STEP, or as soon as the job is asked to end, so a paused
    job still stops, cancels or honors SIGTERM."""
    while control.paused and not job_is_ending(control):
        time.sleep(PAUSE_POLL_SECONDS)
