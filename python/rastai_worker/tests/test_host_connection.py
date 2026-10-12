"""The host connection (host_connection.py; python layout 7.5): Register is retried while the node can't be reached
and never after a refusal, and a replaced worker-host socket (a node restart) makes the job register again.

The fake host stands in for rastai_plugin's WorkerHost, whose own test (rastai_plugin/tests/test_host.py) covers
the call itself; test_worker_server.py runs the real one against a fake node.
"""

import os
import time
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_J2001, RRS_J2002
from rastai_worker.host_connection import HostWatcher, register_with_retry, socket_identity

from rastai_plugin import PluginError

WAIT_SECONDS = 5.0
POLL_SECONDS = 0.01
WATCH_INTERVAL_SECONDS = 0.02


class FakeHost:
    """Fails its first `failures` Register calls with `failure_code`, then answers."""

    def __init__(self, worker_host_socket_path: str = "", failures: int = 0, failure_code: str = RRS_J2001) -> None:
        self.worker_host_socket_path = worker_host_socket_path
        self.failures = failures
        self.failure_code = failure_code
        self.calls = 0

    def register(self) -> None:
        self.calls += 1
        if self.calls <= self.failures:
            raise PluginError(self.failure_code, f"Register call {self.calls} failed")


def wait_until(condition) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while not condition() and time.monotonic() < deadline:
        time.sleep(POLL_SECONDS)
    assert condition()


def test_an_unreachable_node_gets_retries_until_it_answers() -> None:
    host = FakeHost(failures=3)
    register_with_retry(host, timeout_seconds=WAIT_SECONDS)
    assert host.calls == 4


def test_a_refusal_is_final_at_once() -> None:
    host = FakeHost(failures=10, failure_code=RRS_J2002)
    with pytest.raises(PluginError) as raised:
        register_with_retry(host, timeout_seconds=WAIT_SECONDS)
    assert raised.value.code == RRS_J2002
    assert host.calls == 1


def test_a_node_that_never_answers_fails_after_the_timeout() -> None:
    host = FakeHost(failures=1_000_000)
    started = time.monotonic()
    with pytest.raises(PluginError) as raised:
        register_with_retry(host, timeout_seconds=0.3)
    assert raised.value.code == RRS_J2001
    assert host.calls > 1
    assert time.monotonic() - started < WAIT_SECONDS


@pytest.mark.posix_only
def test_a_replaced_socket_makes_the_job_register_again(tmp_path: Path) -> None:
    socket_path = tmp_path / "worker-host.sock"
    socket_path.write_text("the first node")
    host = FakeHost(str(socket_path))
    watcher = HostWatcher(host, interval_seconds=WATCH_INTERVAL_SECONDS)
    watcher.start()
    try:
        time.sleep(WATCH_INTERVAL_SECONDS * 5)
        assert host.calls == 0
        # The node restarts: its socket goes away, then a new one appears at the same path.
        first_identity = socket_identity(str(socket_path))
        os.remove(socket_path)
        time.sleep(WATCH_INTERVAL_SECONDS * 3)
        socket_path.write_text("the restarted node")
        assert socket_identity(str(socket_path)) != first_identity
        wait_until(lambda: host.calls == 1)
        time.sleep(WATCH_INTERVAL_SECONDS * 5)
        assert host.calls == 1
    finally:
        watcher.stop()


def test_a_missing_socket_has_no_identity(tmp_path: Path) -> None:
    assert socket_identity(str(tmp_path / "missing.sock")) is None
