"""JobControl: the plain flags a job's engine polls (layout frontend-and-plugins.md 11.2; python.md 1.6).

The worker's gRPC and signal handlers set them (rastai_worker/control.py); engines poll them between steps, never
inside a captured step. CPython assigns a bool or an int atomically, so neither side needs a lock. There is one flag
per signal because each ends a job differently (contracts 4.4, JobSignal).
"""

from dataclasses import dataclass


@dataclass
class JobControl:
    """What the node has asked of a running job."""

    # SIGTERM, and only SIGTERM: checkpoint if it fits the preemption budget, then exit 75.
    preempt_requested: bool = False
    # STOP: a final checkpoint, then exit 0.
    stop_requested: bool = False
    # CANCEL or SIGINT: stop at the next step boundary without a checkpoint, then exit 3.
    cancel_requested: bool = False
    # CHECKPOINT_NOW: save at the next step boundary, then clear the flag.
    checkpoint_requested: bool = False
    # PAUSE sets it and RESUME clears it.
    paused: bool = False
    # STEP: the steps left before pausing again; None runs freely.
    step_budget: int | None = None
