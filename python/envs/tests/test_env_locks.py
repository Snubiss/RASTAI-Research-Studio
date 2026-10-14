"""The locked environments against each other (M2-15; layout python.md section 11).

These read the files only: no network, so `just test-python` runs them in `just check`. `just env-locks` (CI) is the
online half: it resolves every lock and fails when a lock would change. Each test here fails on the file it names.
"""

import json
import re
import tomllib
from pathlib import Path

ENVIRONMENTS_FILE = "python/envs/environments.json"
ENVIRONMENTS_FOLDER = "python/envs"
LOCK_FILE_NAME = "uv.lock"
PROJECT_FILE_NAME = "pyproject.toml"
JUSTFILE = "justfile"
# The env-locks recipe's loop over every project folder (justfile, `env-locks`). Matched without its indent, so
# reindenting the justfile doesn't break the test.
ENV_LOCKS_LOOP_START = "for project in python/envs/*/pyproject.toml;"
TORCH = "torch"
TORCHVISION = "torchvision"
ROCM = "rocm"
# The torchvision release published with each torch release (torchvision 0.29.1 came out with torch 2.14.1, on
# 2026-09-30). A new torch needs its row here in the same PR as its lock.
TORCHVISION_FOR_TORCH = {"2.14.1": "0.29.1"}
# A build tag after the version, as uv writes it: torch 2.14.1+cpu, torchvision 0.29.1+rocm7.14.
LOCAL_TAG_SEPARATOR = "+"
# A gfx target as ROCm names it: gfx1100, gfx90a.
GFX_TARGET_PATTERN = r"gfx[0-9a-f]+"


def read_environments(repo_root: Path) -> dict:
    return json.loads((repo_root / ENVIRONMENTS_FILE).read_text(encoding="utf-8"))["environments"]


def read_lock(lock_file: Path) -> dict:
    return tomllib.loads(lock_file.read_text(encoding="utf-8"))


def versions_of(lock: dict, name: str) -> list[str]:
    """Every version of `name` the lock holds: one per platform split (torch has a darwin entry and a Linux one)."""
    return [package["version"] for package in lock["package"] if package["name"] == name]


def split_local_tag(version: str) -> tuple[str, str]:
    """2.14.1+cpu is ("2.14.1", "+cpu"); a version without a build tag gives ("2.14.1", "")."""
    base, separator, tag = version.partition(LOCAL_TAG_SEPARATOR)
    return base, separator + tag


def locked_environments(repo_root: Path) -> dict[str, Path]:
    """The environments whose lock is in the repository now: environment id -> its uv.lock."""
    environments = read_environments(repo_root)
    return {
        environment_id: repo_root / entry["lock"] for environment_id, entry in environments.items() if entry["lock"]
    }


def test_each_lock_folder_is_listed_and_each_listed_lock_has_a_folder(repo_root: Path) -> None:
    environments = read_environments(repo_root)
    folders = {project.parent.name for project in (repo_root / ENVIRONMENTS_FOLDER).glob(f"*/{PROJECT_FILE_NAME}")}
    listed = {environment_id for environment_id, entry in environments.items() if entry["lock"]}
    assert folders == listed
    for environment_id in listed:
        assert environments[environment_id]["lock"] == f"{ENVIRONMENTS_FOLDER}/{environment_id}/{LOCK_FILE_NAME}"


def test_each_lock_torchvision_matches_its_torch(repo_root: Path) -> None:
    """Every torch entry in a lock has a torchvision with the same build tag (the same index built both), and that
    torchvision is the release TORCHVISION_FOR_TORCH names for the torch."""
    for environment_id, lock_file in locked_environments(repo_root).items():
        lock = read_lock(lock_file)
        torchvision_versions = versions_of(lock, TORCHVISION)
        assert torchvision_versions, f"{environment_id}: its lock holds no torchvision"
        for torch_version in versions_of(lock, TORCH):
            torch_base, torch_tag = split_local_tag(torch_version)
            matching = [split_local_tag(version) for version in torchvision_versions]
            torchvision_release = TORCHVISION_FOR_TORCH.get(torch_base)
            assert torchvision_release is not None, (
                f"{environment_id}: no torchvision release is listed for {torch_base}"
            )
            assert (torchvision_release, torch_tag) in matching, (
                f"{environment_id}: torch {torch_version} needs torchvision {torchvision_release}{torch_tag} "
                f"in the lock, which holds {torchvision_versions}"
            )


def test_each_listed_environment_names_its_pytorch_build(repo_root: Path) -> None:
    """environments.json's pytorch_build is the torch the environment's lock holds, build tag included."""
    environments = read_environments(repo_root)
    for environment_id, lock_file in locked_environments(repo_root).items():
        torch_versions = versions_of(read_lock(lock_file), TORCH)
        assert environments[environment_id]["pytorch_build"] in torch_versions, environment_id


def test_supported_amd_gfx_targets_are_listed_only_for_rocm_and_are_well_formed(repo_root: Path) -> None:
    for environment_id, entry in read_environments(repo_root).items():
        targets = entry["supported_amd_gfx_targets"]
        if environment_id != ROCM:
            assert targets == [], environment_id
        assert len(set(targets)) == len(targets), environment_id
        for target in targets:
            assert re.fullmatch(GFX_TARGET_PATTERN, target), target


def test_env_locks_recipe_resolves_each_folder_with_directory_and_refresh(repo_root: Path) -> None:
    """`--directory`, not `--project`: from the repo root `--project` reports a stale lock on a clean tree (uv 0.13.0).
    `--refresh` stays, so `--locked` also sees a tampered hash. Fails on a recipe that drops either."""
    recipe_lines = (repo_root / JUSTFILE).read_text(encoding="utf-8").splitlines()
    loop_lines = [line for line in recipe_lines if line.strip().startswith(ENV_LOCKS_LOOP_START)]
    assert len(loop_lines) == 1, (
        f"justfile: expected one line starting '{ENV_LOCKS_LOOP_START}' in the env-locks recipe, "
        f"found {len(loop_lines)}; keep that loop's first words when the recipe is edited"
    )
    assert "uv lock --locked --refresh --directory" in loop_lines[0], loop_lines[0]
