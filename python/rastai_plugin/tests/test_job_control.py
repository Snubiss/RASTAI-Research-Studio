"""JobControl (rastai_plugin/job_control.py): plain flags, set by the worker's handler threads and polled by the
job's main thread without a lock."""

import threading
import time

from rastai_plugin import JobControl

# Long enough for any machine, short enough that a hang fails the test quickly.
WAIT_SECONDS = 10.0
# How often the polling loop looks, like an engine between steps.
POLL_SECONDS = 0.001


def test_flags_start_cleared() -> None:
    control = JobControl()
    assert not control.preempt_requested
    assert not control.stop_requested
    assert not control.cancel_requested
    assert not control.checkpoint_requested
    assert not control.paused
    assert control.step_budget is None


def test_flags_set_from_another_thread_are_seen_by_the_polling_thread() -> None:
    control = JobControl()

    def handle_signals() -> None:
        # What rastai_worker's Control handler and SIGTERM handler do: flip flags, nothing more. The budget is set
        # before the flag the loop waits for, so the loop sees both.
        control.step_budget = 3
        control.preempt_requested = True
        control.stop_requested = True

    handler_thread = threading.Thread(target=handle_signals)
    handler_thread.start()
    # The job's loop: poll between steps until a flag says stop.
    deadline = time.monotonic() + WAIT_SECONDS
    while not control.stop_requested and time.monotonic() < deadline:
        time.sleep(POLL_SECONDS)
    handler_thread.join(WAIT_SECONDS)
    assert control.stop_requested
    assert control.preempt_requested
    assert control.step_budget == 3
    assert not control.cancel_requested
