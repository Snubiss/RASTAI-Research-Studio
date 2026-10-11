"""Smoke test for the cpu environment and the Python workspace (M1-04).

It runs inside `just check` (through `just test-python`) against .dev/envs/cpu, the environment `just env-cpu`
syncs from python/envs/cpu/uv.lock. It fails when the environment drifts from the lock's pins, when torch isn't
the CPU build, or when python/conftest.py stops providing its shared fixtures.
"""

import importlib.metadata
import sys
import tomllib
from pathlib import Path

import pytest

CPU_PROJECT_FILE = Path("python/envs/cpu/pyproject.toml")
# The import name of each locked package, where it differs from its distribution name.
IMPORT_NAMES = {"torch": "torch", "tokenizers": "tokenizers", "grpcio": "grpc", "protobuf": "google.protobuf"}


def read_pinned_versions(project_file: Path) -> dict[str, str]:
    """Reads the exact `name==version` pins of a uv project's dependencies and dev group."""
    project = tomllib.loads(project_file.read_text(encoding="utf-8"))
    pins = project["project"]["dependencies"] + project["dependency-groups"]["dev"]
    versions = {}
    for pin in pins:
        name, version = pin.split("==")
        versions[name] = version
    return versions


def installed_version(distribution_name: str) -> str:
    """The installed version without its build variant: torch 2.14.1+cpu is 2.14.1."""
    return importlib.metadata.version(distribution_name).split("+")[0]


def test_python_is_3_12() -> None:
    assert sys.version_info[:2] == (3, 12)


def test_installed_versions_match_the_pins(repo_root: Path) -> None:
    pinned = read_pinned_versions(repo_root / CPU_PROJECT_FILE)
    installed = {name: installed_version(name) for name in pinned}
    assert installed == pinned


@pytest.mark.parametrize("distribution_name", sorted(IMPORT_NAMES))
def test_locked_package_imports(distribution_name: str) -> None:
    importlib.import_module(IMPORT_NAMES[distribution_name])


def test_torch_is_the_cpu_build() -> None:
    import torch

    assert torch.version.cuda is None
    assert torch.version.hip is None
    identity = torch.eye(3)
    assert torch.equal(identity @ torch.arange(3.0), torch.arange(3.0))


def test_shared_fixtures_point_into_the_repo(repo_root: Path, fixtures_folder: Path) -> None:
    assert (repo_root / "python" / "conftest.py").is_file()
    assert fixtures_folder == repo_root / "fixtures"
