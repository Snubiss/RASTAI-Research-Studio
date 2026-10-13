"""The worker's half of the test-only `RASTAI_TEST_PAUSE_AT` hook (contracts 15.9; flows/platform.md 6.5).

The kill -9 matrix (`tests/crash/`) names a point, and the process that reaches it stops itself there with SIGSTOP,
so the test's SIGKILL lands at that point every time. Every thread stops at once; the test sees the stop in
`/proc/<pid>/stat` (state `T`) and kills the worker, or lets it go on with SIGCONT. A worker stops at its point once:
let go, it runs past the point the next time.

The variable reaches a worker only from a test or from a debug build of the node, which passes it on through the
cleared job environment (`rastai-node/src/pause_points.rs`, the Rust half); a release node never does. The worker's
points are listed in contracts 15.9:

- `worker.event_log.after_metrics_frame`: the first `run.metrics` frame is written whole (event_log.py);
- `worker.event_log.mid_metrics_frame`: half of the second `run.metrics` frame is written, after a whole one
  (event_log.py).
"""

import os
import signal
import sys

# The variable that names the point (contracts 15.9, tests only).
TEST_PAUSE_VARIABLE = "RASTAI_TEST_PAUSE_AT"

# The points this process has stopped at already, so a worker let go with SIGCONT doesn't stop there again.
paused_points: set[str] = set()


def is_pause_point(point: str) -> bool:
    """Whether the variable names `point` and this process hasn't stopped there yet."""
    configured = os.environ.get(TEST_PAUSE_VARIABLE, "")
    return configured != "" and configured == point and point not in paused_points


def pause_at(point: str) -> None:
    """Stops this process here when the variable names `point` (once). Returns when it is let go with SIGCONT."""
    if not is_pause_point(point):
        return
    paused_points.add(point)
    # For the test's output: a failing row shows where its processes stopped.
    print(f"{TEST_PAUSE_VARIABLE}: process {os.getpid()} paused at {point}", file=sys.stderr, flush=True)
    os.kill(os.getpid(), signal.SIGSTOP)
