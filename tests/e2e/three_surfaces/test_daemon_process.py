"""The cleanup the three-surfaces test relies on (issue #244): whatever still works in the test's home is killed,
with its whole process group, and nothing is left. A stand-in worker shows it: like a job's worker, it leads a
process group of its own, names its job file in the home on its command line, and has a child of its own."""

import os
import subprocess
import sys
import time
from pathlib import Path

import pytest

from daemon_process import kill_group, kill_processes_of_home, make_home, processes_of_home, remove_home

# Long enough to outlive the test by far: only the cleanup can end it.
STAND_IN_SECONDS = 600
# The stand-in starts a child in its own group, then sleeps; the child sleeps too.
STAND_IN_PROGRAM = (
    "import subprocess, sys, time\n"
    "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep({seconds})'])\n"
    "time.sleep({seconds})\n"
)
START_LIMIT_SECONDS = 30
POLL_SECONDS = 0.05
# The stand-in and its child.
STAND_IN_PROCESS_COUNT = 2

# The daemon reads /proc and signals process groups, and the node's sockets are Unix ones. pytest reads
# `pytestmark` only in a test module (never in conftest.py), so each test module carries it.
pytestmark = pytest.mark.skipif(sys.platform != "linux", reason="the daemon's job tests need Linux")


def start_stand_in_worker(job_file: str) -> subprocess.Popen:
    program = STAND_IN_PROGRAM.format(seconds=STAND_IN_SECONDS)
    return subprocess.Popen([sys.executable, "-c", program, job_file], start_new_session=True)


def wait_for_processes_in_group(group_id: int, count: int) -> None:
    deadline = time.monotonic() + START_LIMIT_SECONDS
    while len(group_members(group_id)) < count:
        assert time.monotonic() < deadline, "the stand-in worker didn't start its child"
        time.sleep(POLL_SECONDS)


def wait_until_group_is_gone(group_id: int) -> None:
    """A killed process can show in the process table for a moment, until its parent reaps it."""
    deadline = time.monotonic() + START_LIMIT_SECONDS
    while group_members(group_id):
        assert time.monotonic() < deadline, f"left in the group: {group_members(group_id)}"
        time.sleep(POLL_SECONDS)


def group_members(group_id: int) -> list[int]:
    """The processes in the group, from /proc."""
    members = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit() and process_group_of(int(entry.name)) == group_id:
            members.append(int(entry.name))
    return members


def process_group_of(process_id: int) -> int | None:
    try:
        return os.getpgid(process_id)
    except ProcessLookupError:
        return None


def test_every_process_group_working_in_the_home_is_killed():
    home = make_home()
    stand_in = None
    try:
        stand_in = start_stand_in_worker(str(home / "data" / "runs" / "run_x" / "job.json"))
        wait_for_processes_in_group(stand_in.pid, STAND_IN_PROCESS_COUNT)
        assert [process_id for process_id, _ in processes_of_home(home)] == [stand_in.pid]
        assert kill_processes_of_home(home) == []
        stand_in.wait(timeout=START_LIMIT_SECONDS)
        # The child named nothing of the home; it went with its group.
        wait_until_group_is_gone(stand_in.pid)
    finally:
        if stand_in is not None:
            kill_group(stand_in.pid)
            stand_in.wait(timeout=START_LIMIT_SECONDS)
        remove_home(home)
