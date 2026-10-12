"""Control (control.py; contracts 4.4; python layout 7.6 and 7.9): SIGTERM sets only preempt_requested, SIGINT only
cancel_requested, SIGHUP nothing; each Control value sets its own flag, STOP never preempt_requested; TERMINATE and
KILL are refused as Control values.
"""

import os
import signal
import threading
import time
from collections.abc import Iterator
from dataclasses import asdict

import pytest
from rastai_plugin.catalog.generated import RRS_J0004
from rastai_worker.control import apply_control_action, install_control, job_is_ending, wait_for_resume
from rastai_worker.errors import WorkerError

from rastai_plugin import JobControl

pytestmark = pytest.mark.posix_only

# How long a signal sent to this process may take to reach its Python handler.
SIGNAL_WAIT_SECONDS = 2.0
SIGNAL_POLL_SECONDS = 0.01
HANDLED_SIGNALS = (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)


@pytest.fixture
def control() -> Iterator[JobControl]:
    """install_control on this test process, with pytest's own handlers put back afterwards."""
    previous_handlers = {number: signal.getsignal(number) for number in HANDLED_SIGNALS}
    yield install_control()
    for number, handler in previous_handlers.items():
        signal.signal(number, handler)


def flags_after_signal(control: JobControl, number: int, flag_name: str) -> dict:
    os.kill(os.getpid(), number)
    # Python runs the handler on the main thread at its next instruction; give it a moment.
    deadline = time.monotonic() + SIGNAL_WAIT_SECONDS
    while flag_name and not getattr(control, flag_name) and time.monotonic() < deadline:
        time.sleep(SIGNAL_POLL_SECONDS)
    time.sleep(SIGNAL_POLL_SECONDS)
    return asdict(control)


def only_set(flag_name: str, value: object = True) -> dict:
    flags = asdict(JobControl())
    flags[flag_name] = value
    return flags


def test_sigterm_sets_only_preempt_requested(control: JobControl) -> None:
    assert flags_after_signal(control, signal.SIGTERM, "preempt_requested") == only_set("preempt_requested")


def test_sigint_sets_only_cancel_requested(control: JobControl) -> None:
    """No KeyboardInterrupt either: the handler replaced Python's default one, so the job keeps its thread."""
    assert flags_after_signal(control, signal.SIGINT, "cancel_requested") == only_set("cancel_requested")


def test_sighup_is_ignored(control: JobControl) -> None:
    assert flags_after_signal(control, signal.SIGHUP, "") == asdict(JobControl())


@pytest.mark.parametrize(
    ("action", "flag_name", "value"),
    [
        ("CANCEL", "cancel_requested", True),
        ("STOP", "stop_requested", True),
        ("PAUSE", "paused", True),
        ("CHECKPOINT_NOW", "checkpoint_requested", True),
    ],
)
def test_each_control_value_sets_its_own_flag(action: str, flag_name: str, value: object) -> None:
    control = JobControl()
    apply_control_action(control, action)
    assert asdict(control) == only_set(flag_name, value)


def test_resume_clears_pause_and_step_budget() -> None:
    control = JobControl(paused=True, step_budget=2)
    apply_control_action(control, "RESUME")
    assert asdict(control) == asdict(JobControl())


def test_step_runs_one_more_step_each_time() -> None:
    control = JobControl(paused=True)
    apply_control_action(control, "STEP")
    assert (control.paused, control.step_budget) == (False, 1)
    apply_control_action(control, "STEP")
    assert control.step_budget == 2


@pytest.mark.parametrize("action", ["TERMINATE", "KILL", "UNSPECIFIED", "number 42"])
def test_os_signals_and_unknown_values_are_refused(action: str) -> None:
    control = JobControl()
    with pytest.raises(WorkerError) as raised:
        apply_control_action(control, action)
    assert raised.value.code == RRS_J0004
    assert asdict(control) == asdict(JobControl())


def test_a_job_is_ending_after_stop_cancel_or_sigterm_only() -> None:
    assert not job_is_ending(JobControl(paused=True, checkpoint_requested=True))
    for flag_name in ("stop_requested", "cancel_requested", "preempt_requested"):
        assert job_is_ending(JobControl(**{flag_name: True}))


def test_a_paused_job_waits_for_resume_or_its_end() -> None:
    for action in ("RESUME", "CANCEL"):
        control = JobControl(paused=True)
        waiter = threading.Thread(target=wait_for_resume, args=(control,), daemon=True)
        waiter.start()
        time.sleep(0.2)
        assert waiter.is_alive()
        apply_control_action(control, action)
        waiter.join(timeout=SIGNAL_WAIT_SECONDS)
        assert not waiter.is_alive()
