"""Shared pytest fixtures and marker rules for every package under python/ (layout: python.md section 1.2).

The markers themselves are registered in python/pyproject.toml. The tiny-model factory joins this file with
rastai_models, the first package that can build a model.
"""

import importlib
import os
import warnings
from pathlib import Path

import pytest

# This file sits in python/, one folder below the repository root.
PYTHON_FOLDER = Path(__file__).resolve().parent
REPO_ROOT = PYTHON_FOLDER.parent
# The shared test inputs of contracts 12.5. Tests read from it and never write into it.
FIXTURES_FOLDER = REPO_ROOT / "fixtures"
# A workspace package's import package: python/<name>/<name>/__init__.py (python/CLAUDE.md).
WORKSPACE_PACKAGE_INIT_PATTERN = "rastai*/rastai*/__init__.py"
# torch warns when it is imported without numpy, which no Rastai code uses (python/pyproject.toml filters it during
# the tests). Packages are imported here, before those filters apply, so the same warning is ignored here too.
NUMPY_MISSING_WARNING = "Failed to initialize NumPy"


def pytest_configure(config: pytest.Config) -> None:
    import_workspace_packages()


def import_workspace_packages() -> None:
    """Imports every workspace package before pytest collects a test.

    pytest's importlib mode names python/rastai_proto/tests/test_golden.py `rastai_proto.tests.test_golden`, after
    its path below python/. If `rastai_proto` isn't imported yet, pytest imports the folder python/rastai_proto/ for
    it, which has no __init__.py, as a namespace package, and that hides the real package (python/rastai_proto/
    rastai_proto/) from the test. With the real packages already in sys.modules, pytest uses them.

    A package that fails to import stops the whole run here, before any test is collected, with that package's
    import error. That is on purpose: a broken package (for example a native wheel that wasn't built) should be
    the first thing a run reports, not a namespace-package mix-up in some other package's tests.
    """
    with warnings.catch_warnings():
        warnings.filterwarnings("ignore", message=NUMPY_MISSING_WARNING, category=UserWarning)
        for init_file in sorted(PYTHON_FOLDER.glob(WORKSPACE_PACKAGE_INIT_PATTERN)):
            package_folder = init_file.parent
            if package_folder.name == package_folder.parent.name:
                importlib.import_module(package_folder.name)


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
