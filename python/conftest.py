"""Shared pytest fixtures and marker rules for every package under python/ (layout: python.md section 1.2).

The markers themselves are registered in python/pyproject.toml. The tiny-model factory joins this file with
rastai_models, the first package that can build a model.
"""

import os
from pathlib import Path

import pytest

# This file sits one folder below the repository root.
REPO_ROOT = Path(__file__).resolve().parent.parent
# The shared test inputs of contracts 12.5. Tests read from it and never write into it.
FIXTURES_FOLDER = REPO_ROOT / "fixtures"


@pytest.fixture
def repo_root() -> Path:
    """The repository root, for tests that read a checked-in file such as a lock or a schema."""
    return REPO_ROOT


@pytest.fixture
def fixtures_folder() -> Path:
    """The shared fixtures folder (contracts 12.5)."""
    return FIXTURES_FOLDER


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Skips the tests this machine can't run, with a reason that says why."""
    skip_marker_on_items(items, "posix_only", not is_posix(), "needs Unix sockets, flock or setsid")
    if any(item.get_closest_marker("gpu") for item in items):
        skip_marker_on_items(items, "gpu", not gpu_is_available(), "needs a CUDA or ROCm device")


def skip_marker_on_items(items: list[pytest.Item], marker_name: str, should_skip: bool, reason: str) -> None:
    if not should_skip:
        return
    skip = pytest.mark.skip(reason=reason)
    for item in items:
        if item.get_closest_marker(marker_name):
            item.add_marker(skip)


def is_posix() -> bool:
    return os.name == "posix"


def gpu_is_available() -> bool:
    # Imported here, not at the top, so a run without any gpu test never pays for importing torch.
    import torch

    # ROCm builds of torch answer through torch.cuda too.
    return torch.cuda.is_available()
