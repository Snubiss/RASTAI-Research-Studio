"""Fixtures of the three-surfaces test: one daemon for the whole session in a temporary RASTAI_HOME, the owner's
HTTP and CLI surfaces, the scoped tokens, and an MCP bridge per token. Every process the session started is stopped
when it ends, however it ends, and none may be left (issue #244)."""

import os
import subprocess
from collections.abc import Iterator
from pathlib import Path

import pytest

from daemon_process import (
    RunningDaemon,
    link_cpu_environment,
    make_home,
    remove_home,
    spawn_daemon,
    stop_daemon,
    wait_until_healthy,
)
from surfaces import CliSurface, HttpSurface, McpSurface

# This file sits in tests/e2e/three_surfaces/, three folders below the repository root.
REPO_ROOT = Path(__file__).resolve().parents[3]
# `just e2e-surfaces` builds the binary first. CARGO_TARGET_DIR is cargo's own variable for a target folder elsewhere.
TARGET_FOLDER = Path(os.environ.get("CARGO_TARGET_DIR", REPO_ROOT / "target"))
RASTAI_BINARY = (REPO_ROOT / TARGET_FOLDER / "debug" / "rastai").absolute()
# Never resolved: the cpu environment's bin/python is a link whose own path makes Python use the environment.
CPU_PYTHON = REPO_ROOT / ".dev" / "envs" / "cpu" / "bin" / "python"
# The scoped tokens the test makes with token.create, by the name it gives them. `train` is what an agent gets to
# train with (mcp-default lacks `training`, contracts 6.3); `chat-only` and `read` lack `training`.
TOKEN_ARGUMENTS = {
    "train": {"name": "three-surfaces-train", "scopes": ["read", "training"]},
    "chat-only": {"name": "three-surfaces-chat", "preset": "chat-only"},
    "read": {"name": "three-surfaces-read", "scopes": ["read"]},
}
MCP_LOG_FILE = "mcp-{token}.log"


@pytest.fixture(scope="session")
def repo_root() -> Path:
    return REPO_ROOT


@pytest.fixture(scope="session")
def daemon() -> Iterator[RunningDaemon]:
    assert RASTAI_BINARY.exists(), f"{RASTAI_BINARY} is missing: run `just e2e-surfaces`, which builds it"
    assert CPU_PYTHON.exists(), f"{CPU_PYTHON} is missing: run `just env-cpu`"
    home = make_home()
    process = None
    try:
        link_cpu_environment(home, CPU_PYTHON)
        process = spawn_daemon(RASTAI_BINARY, home)
        yield wait_until_healthy(RASTAI_BINARY, home, process)
    except BaseException as error:
        # The session's own error stays the one reported: a process it left behind goes in a note on that error.
        left = stop_and_remove(home, process)
        if left:
            error.add_note(f"processes outlived the test: {left}")
        raise
    left = stop_and_remove(home, process)
    assert left == [], f"processes outlived the test: {left}"


def stop_and_remove(home: Path, process: subprocess.Popen | None) -> list[str]:
    """Stops the daemon and whatever works in its home, removes the home, and returns the processes left over."""
    left = stop_daemon(RASTAI_BINARY, home, process)
    remove_home(home)
    return left


@pytest.fixture(scope="session")
def owner_http(daemon: RunningDaemon) -> HttpSurface:
    """HTTP with the owner token, so it is the same principal as the CLI (the owner, by daemon.sock's peer)."""
    return HttpSurface(daemon.port, daemon.owner_token)


@pytest.fixture(scope="session")
def cli(daemon: RunningDaemon) -> CliSurface:
    return CliSurface(daemon.binary, daemon.home)


@pytest.fixture(scope="session")
def tokens(owner_http: HttpSurface) -> dict:
    """Each scoped token's token.create result (`token_id`, `scopes`, `token`), by its name above."""
    created = {}
    for name, arguments in TOKEN_ARGUMENTS.items():
        outcome = owner_http.execute("token.create", arguments)
        assert outcome.ok, outcome
        created[name] = outcome.result
    return created


def started_mcp(daemon: RunningDaemon, token: dict, token_name: str) -> McpSurface:
    log_path = daemon.home / MCP_LOG_FILE.format(token=token_name)
    surface = McpSurface(binary=daemon.binary, home=daemon.home, token=token["token"], log_path=log_path)
    surface.start()
    return surface


@pytest.fixture(scope="session")
def train_mcp(daemon: RunningDaemon, tokens: dict) -> Iterator[McpSurface]:
    surface = started_mcp(daemon, tokens["train"], "train")
    yield surface
    surface.stop()


@pytest.fixture(scope="session")
def chat_only_mcp(daemon: RunningDaemon, tokens: dict) -> Iterator[McpSurface]:
    surface = started_mcp(daemon, tokens["chat-only"], "chat-only")
    yield surface
    surface.stop()


@pytest.fixture(scope="session")
def read_mcp(daemon: RunningDaemon, tokens: dict) -> Iterator[McpSurface]:
    surface = started_mcp(daemon, tokens["read"], "read")
    yield surface
    surface.stop()
