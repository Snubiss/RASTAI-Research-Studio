"""Shared fixtures for the repo-check tests: build a fixture tree, run a check on it, load a check's functions.

Each test writes its fixture tree into a temporary folder instead of checking one in, so a failing fixture (a
pickle import, a .tsx file, a color literal) never sits in the repository for the real checks to find.
"""

import subprocess
import sys
from collections.abc import Callable
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
from types import ModuleType

import pytest

# This file sits in tools/checks/tests/.
CHECKS_FOLDER = Path(__file__).resolve().parent.parent
REPO_ROOT = CHECKS_FOLDER.parent.parent


@pytest.fixture
def write_tree(tmp_path: Path) -> Callable[[dict[str, str]], Path]:
    """Returns a function that writes {relative path: text} into a fresh folder and returns that folder."""

    def write(files: dict[str, str]) -> Path:
        root = tmp_path / "tree"
        for relative_path, text in files.items():
            path = root / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        root.mkdir(exist_ok=True)
        return root

    return write


@pytest.fixture
def run_check() -> Callable[..., subprocess.CompletedProcess]:
    """Returns a function that runs one check script, as `just` does, with the given arguments."""

    def run(check_name: str, *arguments: str) -> subprocess.CompletedProcess:
        command = [sys.executable, str(CHECKS_FOLDER / check_name), *arguments]
        return subprocess.run(command, capture_output=True, text=True)

    return run


@pytest.fixture
def load_check() -> Callable[[str], ModuleType]:
    """Returns a function that loads a check script as a module, for tests of its functions. The scripts have no
    .py ending (their names are the command names), so a plain import can't find them; the loader is named
    directly instead."""

    def load(check_name: str) -> ModuleType:
        module_name = check_name.replace("-", "_")
        loader = SourceFileLoader(module_name, str(CHECKS_FOLDER / check_name))
        module = module_from_spec(spec_from_loader(module_name, loader))
        loader.exec_module(module)
        return module

    return load
