"""Fixtures of the worker's kill -9 tests: the repository's folders, each test's job folders, and the list of the
processes it started, whose process groups are killed on every exit path (issue #244)."""

import shutil
import tempfile
from collections.abc import Iterator
from pathlib import Path

import pytest

from worker_support import JobFolders, kill_group, live_group_members

# This file sits in tests/crash/, two folders below the repository root.
REPO_ROOT = Path(__file__).resolve().parents[2]
# The shared test inputs of contracts 12.5. Tests read from it and never write into it.
FIXTURES_FOLDER = REPO_ROOT / "fixtures"
# A short folder: a Unix socket path must fit in 108 bytes, and pytest's tmp_path can be long.
SOCKET_FOLDER_PREFIX = "rcw-"


@pytest.fixture
def repo_root() -> Path:
    return REPO_ROOT


@pytest.fixture
def fixtures_folder() -> Path:
    return FIXTURES_FOLDER


@pytest.fixture
def folders(tmp_path: Path) -> Iterator[JobFolders]:
    socket_folder = Path(tempfile.mkdtemp(prefix=SOCKET_FOLDER_PREFIX))
    yield JobFolders(run_folder=tmp_path / "run", socket_folder=socket_folder)
    shutil.rmtree(socket_folder, ignore_errors=True)


@pytest.fixture
def started() -> Iterator[list]:
    """The processes the test started. Each one's group is killed when the test ends, however it ends, and none may
    be left after that."""
    processes = []
    yield processes
    for started_process in processes:
        kill_group(started_process)
    for started_process in processes:
        assert live_group_members(started_process.pid) == [], f"{started_process.description} left processes"
