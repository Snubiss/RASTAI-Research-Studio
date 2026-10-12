"""The job's connection to its node: WorkerHostService.Register, with retry, on start and after a node restart
(contracts 3.2 and 4.5; python layout 7.5).

The client is rastai_plugin's WorkerHost, which sends the job token in the `rastai-job-token` metadata key on every
call and raises a PluginError with a catalog code: RRS-J2001 when the node can't be reached, the node's own code
(RRS-J2002 for a wrong token) when it refused. Only an unreachable node is worth another try.

- **On start** the worker registers right after it serves its own socket, because the node calls Hello as soon as
  Register arrives. A node that is still binding its socket gets retries with backoff for up to
  REGISTER_TIMEOUT_SECONDS, the time the node itself waits before it fails the job (RRS-J0002).
- **After a node restart** the node's `worker-host.sock` is a new file. `HostWatcher` notices the new file and
  registers again, retrying for up to HOST_RECONNECT_TIMEOUT_SECONDS, the time the node waits for a job to come back
  (RRS-J3003). The job keeps running meanwhile: only calls that need the node wait.
"""

import os
import sys
import threading
import time

from rastai_plugin.catalog.generated import RRS_J2001

from rastai_plugin import PluginError, WorkerHost
from rastai_worker.job_file import WorkerLaunch

# The node fails a job that hasn't registered by then (contracts 9.1, WORKER_REGISTER_TIMEOUT_SECONDS).
REGISTER_TIMEOUT_SECONDS = 60
# How long a job keeps trying to register with a restarted node (contracts 15.10).
HOST_RECONNECT_TIMEOUT_SECONDS = 300
# Backoff between tries: short at first (a node binds its socket in milliseconds), doubling up to a few seconds.
FIRST_RETRY_DELAY_SECONDS = 0.05
MAX_RETRY_DELAY_SECONDS = 5.0
# How often the watcher looks at the node's socket file. A restarted node calls nothing until the job registers,
# so this is how long a reattach waits at most before it starts.
SOCKET_WATCH_INTERVAL_SECONDS = 1.0


def connect_to_host(launch: WorkerLaunch, job_token: str) -> WorkerHost:
    """The job's WorkerHost, registered with the node. Raises the last PluginError when the node refused the job or
    stayed unreachable for REGISTER_TIMEOUT_SECONDS."""
    host = WorkerHost(launch.worker_host_socket_path, launch.job_id, launch.job_socket_path, job_token, launch.job_file)
    register_with_retry(host, REGISTER_TIMEOUT_SECONDS)
    return host


def register_with_retry(host: WorkerHost, timeout_seconds: float) -> None:
    """Calls Register until the node answers it. An unreachable node (RRS-J2001) gets another try after a growing
    delay while `timeout_seconds` last; a refusal is final at once."""
    deadline = time.monotonic() + timeout_seconds
    delay = FIRST_RETRY_DELAY_SECONDS
    while True:
        try:
            host.register()
            return
        except PluginError as error:
            if error.code != RRS_J2001 or time.monotonic() + delay > deadline:
                raise
        time.sleep(delay)
        delay = min(delay * 2, MAX_RETRY_DELAY_SECONDS)


def socket_identity(path: str) -> tuple[int, int, int] | None:
    """Which file is at `path` now: device, inode and change time. A restarted node's new socket differs in at least
    one of them, even when the file system hands out the old inode number again. None when nothing is there."""
    try:
        status = os.stat(path)
    except OSError:
        return None
    return status.st_dev, status.st_ino, status.st_ctime_ns


class HostWatcher:
    """Registers the job again whenever the node's worker-host socket is replaced (a node restart). It runs on its
    own thread and never touches the job's model or log."""

    def __init__(self, host: WorkerHost, interval_seconds: float = SOCKET_WATCH_INTERVAL_SECONDS) -> None:
        self.host = host
        self.interval_seconds = interval_seconds
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.watch, name="rastai-host-watcher", daemon=True)

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        """Stops watching. It doesn't wait for a Register that is being retried: the thread is a daemon thread, so it
        never keeps the process from exiting."""
        self.stopped.set()

    def watch(self) -> None:
        known_socket = socket_identity(self.host.worker_host_socket_path)
        while not self.stopped.wait(self.interval_seconds):
            current_socket = socket_identity(self.host.worker_host_socket_path)
            # Missing means the node is restarting: wait for its new socket rather than retrying into nothing.
            if current_socket is None or current_socket == known_socket:
                continue
            known_socket = current_socket
            self.register_again()

    def register_again(self) -> None:
        try:
            register_with_retry(self.host, HOST_RECONNECT_TIMEOUT_SECONDS)
        except PluginError as error:
            # The job keeps running and writing its log; the node decides what a job that didn't come back is
            # (RRS-J3003). stderr.log keeps the reason.
            print(f"{error.code}: {error.message}", file=sys.stderr, flush=True)
