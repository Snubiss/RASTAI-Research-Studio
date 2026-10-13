"""The daemon the three-surfaces test drives: `rastai serve --headless` in a temporary RASTAI_HOME, in its own process
group, with the repository's cpu environment linked in for its embedded node. Stopping it also kills every process
that still works in that home, job workers included, on every exit path (issue #244): the node starts each worker
in a process group of its own and the daemon's shutdown leaves running jobs alone, so killing the daemon's group
alone would leave a worker training for hours."""

import os
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

# A short folder name keeps `<home>/runtime/jobs/<job_id>.sock` within the 107 bytes a Unix socket address holds.
HOME_PREFIX = "r3s-"
HOME_VARIABLE = "RASTAI_HOME"
# The `daemon.port` setting's environment layer (contracts 11.2). 0 picks any free port, so the test never fights a
# real daemon or another test over 47820-47829.
PORT_VARIABLE = "RASTAI_DAEMON__PORT"
ANY_FREE_PORT = "0"
# Variables a developer's shell may hold that would point a surface at another daemon or another token.
VARIABLES_TO_DROP = ("RASTAI_TOKEN", "RASTAI_URL", "RASTAI_DAEMON_SOCKET")
# `<data>/envs/<environment_id>-<lock_hash12>/`: the embedded node runs jobs in the environments it finds there,
# until uv environments land (M2-16). The same stand-in as crates/rastai-server/tests/train_jobs.rs.
ENVIRONMENT_FOLDER = "data/envs/cpu-000000000000"
ENVIRONMENT_LAUNCHER_MODE = 0o755
PORT_FILE = "runtime/daemon.port"
OWNER_TOKEN_FILE = "config/auth/owner.token"
SERVE_LOG_FILE = "serve.log"
HEALTH_URL = "http://127.0.0.1:{port}/api/health"
HTTP_OK = 200
# Startup opens the database, makes the identities and connects the embedded node; a loaded machine is slow.
START_LIMIT_SECONDS = 180
SHUTDOWN_LIMIT_SECONDS = 30
CLI_LIMIT_SECONDS = 120
# After SIGKILL the kernel reaps a process quickly, but /proc can show it for a moment longer.
KILL_LIMIT_SECONDS = 30
POLL_SECONDS = 0.2
REQUEST_LIMIT_SECONDS = 5
# How much of the daemon's log a failure shows.
LOG_TAIL_CHARACTERS = 4000


@dataclass
class RunningDaemon:
    """A started daemon, and what the surfaces need to reach it."""

    binary: Path
    home: Path
    process: subprocess.Popen
    port: int
    owner_token: str


def surface_environment(home: Path) -> dict:
    """The environment every `rastai` the test runs gets: this test's home, any free port, and no stray token."""
    environment = dict(os.environ)
    for name in VARIABLES_TO_DROP:
        environment.pop(name, None)
    environment[HOME_VARIABLE] = str(home)
    environment[PORT_VARIABLE] = ANY_FREE_PORT
    return environment


def make_home() -> Path:
    return Path(tempfile.mkdtemp(prefix=HOME_PREFIX))


def link_cpu_environment(home: Path, cpu_python: Path) -> None:
    """`<data>/envs/cpu-.../bin/python` runs the cpu environment's interpreter. A script, not a link: Python finds
    its environment from the path it was started by, which must stay the environment's own `bin/python`."""
    bin_folder = home / ENVIRONMENT_FOLDER / "bin"
    bin_folder.mkdir(parents=True)
    launcher = bin_folder / "python"
    launcher.write_text(f"#!/bin/sh\nexec '{cpu_python}' \"$@\"\n")
    launcher.chmod(ENVIRONMENT_LAUNCHER_MODE)


def spawn_daemon(binary: Path, home: Path) -> subprocess.Popen:
    """Starts `rastai serve --headless` in its own process group, its output in `<home>/serve.log`."""
    with open(home / SERVE_LOG_FILE, "wb") as log:
        return subprocess.Popen(
            [str(binary), "serve", "--headless"],
            cwd=home,
            env=surface_environment(home),
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )


def wait_until_healthy(binary: Path, home: Path, process: subprocess.Popen) -> RunningDaemon:
    """Waits until the daemon wrote its port and its health endpoint answers there."""
    deadline = time.monotonic() + START_LIMIT_SECONDS
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"rastai serve exited with {process.returncode}:\n{serve_log_tail(home)}")
        port = read_port(home)
        if port and health_answers(port):
            owner_token = (home / OWNER_TOKEN_FILE).read_text().strip()
            return RunningDaemon(binary=binary, home=home, process=process, port=port, owner_token=owner_token)
        time.sleep(POLL_SECONDS)
    raise AssertionError(f"rastai serve wasn't healthy within {START_LIMIT_SECONDS} s:\n{serve_log_tail(home)}")


def read_port(home: Path) -> int:
    """The port the daemon wrote, or 0 before it wrote one."""
    try:
        text = (home / PORT_FILE).read_text().strip()
    except FileNotFoundError:
        return 0
    return int(text) if text.isdigit() else 0


def health_answers(port: int) -> bool:
    try:
        with urllib.request.urlopen(HEALTH_URL.format(port=port), timeout=REQUEST_LIMIT_SECONDS) as response:
            return response.status == HTTP_OK
    except (urllib.error.URLError, ConnectionError, TimeoutError):
        return False


def serve_log_tail(home: Path) -> str:
    try:
        text = (home / SERVE_LOG_FILE).read_text(errors="replace")
    except FileNotFoundError:
        return "(no serve.log)"
    return text[-LOG_TAIL_CHARACTERS:]


def stop_daemon(binary: Path, home: Path, process: subprocess.Popen | None) -> list[str]:
    """Stops the daemon, if it started, and kills whatever still works in its home. Returns the processes that
    outlived SIGKILL, which the caller reports; it never raises, so the cleanup after it always runs."""
    if process is not None:
        ask_daemon_to_shut_down(binary, home, process)
        wait_for_exit(process, SHUTDOWN_LIMIT_SECONDS)
        kill_group(process.pid)
        wait_for_exit(process, KILL_LIMIT_SECONDS)  # reaps it, so no zombie is left
    return kill_processes_of_home(home)


def ask_daemon_to_shut_down(binary: Path, home: Path, process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    command_line = [str(binary), "system", "shutdown", "--no-start", "--json"]
    try:
        subprocess.run(
            command_line,
            env=surface_environment(home),
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=CLI_LIMIT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        pass  # SIGKILL to its group ends it anyway


def wait_for_exit(process: subprocess.Popen, limit_seconds: float) -> None:
    try:
        process.wait(timeout=limit_seconds)
    except subprocess.TimeoutExpired:
        pass  # SIGKILL to its group follows


def kill_processes_of_home(home: Path) -> list[str]:
    """SIGKILL to the process group of every process that works in `home`, then waits until none is left. Returns
    the ones still there after the wait, as `pid command line`."""
    deadline = time.monotonic() + KILL_LIMIT_SECONDS
    while True:
        left = processes_of_home(home)
        if not left:
            return []
        if time.monotonic() > deadline:
            return [f"{process_id} {command_line}" for process_id, command_line in left]
        for process_id, _command_line in left:
            kill_group(process_id)
        time.sleep(POLL_SECONDS)


def processes_of_home(home: Path) -> list[tuple[int, str]]:
    """The processes whose environment names this RASTAI_HOME (the daemon, a CLI, `rastai mcp`) or whose command line
    names a file in it (a job's worker: the node clears a worker's environment but passes its job file). Linux only:
    it reads /proc."""
    home_text = str(home)
    home_variable = f"{HOME_VARIABLE}={home_text}".encode()
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        command_line = read_proc_file(entry / "cmdline").replace(b"\0", b" ").decode(errors="replace")
        environment = read_proc_file(entry / "environ").split(b"\0")
        if home_variable in environment or f"{home_text}/" in command_line:
            found.append((int(entry.name), command_line.strip()))
    return found


def read_proc_file(path: Path) -> bytes:
    """A process can end, or belong to another user, between listing /proc and reading it."""
    try:
        return path.read_bytes()
    except OSError:
        return b""


def kill_group(process_id: int) -> None:
    """SIGKILL to the process's group, and to the process itself in case it left that group. Never this test's own
    group. A process can end between being found and being signalled, which is what was wanted anyway."""
    try:
        group_id = os.getpgid(process_id)
    except ProcessLookupError:
        return
    if group_id != os.getpgrp():
        try:
            os.killpg(group_id, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
    try:
        os.kill(process_id, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def remove_home(home: Path) -> None:
    shutil.rmtree(home, ignore_errors=True)
