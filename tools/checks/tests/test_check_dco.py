"""check-dco on a throwaway git repository with signed, unsigned and wrongly signed commits."""

import os
import subprocess
import sys
from pathlib import Path

AUTHOR_NAME = "Ada Example"
AUTHOR_EMAIL = "ada@example.com"
OTHER_SIGN_OFF = "Signed-off-by: Bob Example <bob@example.com>"
# Variables that tell git which repository to use. Git sets them for a hook and for `git rebase -x` in a linked
# worktree; inherited by the scratch repository's `git init`, they made git re-initialize the outer repository as
# bare (core.bare=true in the main clone's shared config) instead of making a new one.
REPOSITORY_LOCATION_VARIABLES = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
]
# The machine's own git settings (commit signing, hooks, a default branch name) must not change the test.
MACHINE_CONFIG_OFF = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
# The test that runs this file's other tests under a foreign GIT_DIR leaves itself out of that run.
FOREIGN_GIT_DIR_TEST = "test_tests_leave_a_repository_named_by_git_dir_alone"


def without_repository_location(environment: dict[str, str]) -> dict[str, str]:
    """A copy of the environment in which git finds the repository from the working folder (or -C) alone."""
    kept = {}
    for name, value in environment.items():
        if name not in REPOSITORY_LOCATION_VARIABLES:
            kept[name] = value
    return kept


ISOLATED_GIT_ENVIRONMENT = without_repository_location(dict(os.environ)) | MACHINE_CONFIG_OFF


def git(repository: Path, *arguments: str) -> str:
    identity = ["-c", f"user.name={AUTHOR_NAME}", "-c", f"user.email={AUTHOR_EMAIL}"]
    command = ["git", "-C", str(repository), *identity, *arguments]
    result = subprocess.run(command, capture_output=True, text=True, check=True, env=ISOLATED_GIT_ENVIRONMENT)
    return result.stdout.strip()


def commit(repository: Path, file_name: str, *message_arguments: str) -> str:
    (repository / file_name).write_text(file_name)
    git(repository, "add", file_name)
    git(repository, "commit", "--quiet", *message_arguments)
    return git(repository, "rev-parse", "HEAD")


def new_repository(tmp_path: Path) -> Path:
    repository = tmp_path / "repository"
    repository.mkdir()
    git(repository, "init", "--quiet", "--initial-branch=main")
    commit(repository, "base.txt", "-m", "Base commit before the range")
    return repository


def run_dco(run_check, repository: Path):
    return run_check("check-dco", "--repo", str(repository), "main..HEAD")


def test_commits_signed_off_by_their_author_pass(tmp_path, run_check):
    repository = new_repository(tmp_path)
    git(repository, "switch", "--quiet", "-c", "feature")
    commit(repository, "one.txt", "-s", "-m", "Signed with -s")
    commit(repository, "two.txt", "-m", f"Signed by hand\n\nSigned-off-by: {AUTHOR_NAME} <ADA@example.com>")
    result = run_dco(run_check, repository)
    assert result.returncode == 0, result.stdout


def test_unsigned_and_wrongly_signed_commits_fail(tmp_path, run_check):
    repository = new_repository(tmp_path)
    git(repository, "switch", "--quiet", "-c", "feature")
    unsigned = commit(repository, "one.txt", "-m", "No sign-off")
    wrongly_signed = commit(repository, "two.txt", "-m", f"Someone else's sign-off\n\n{OTHER_SIGN_OFF}")
    commit(repository, "three.txt", "-s", "-m", "Signed")
    result = run_dco(run_check, repository)
    assert result.returncode == 1
    assert f"{unsigned[:12]}: no Signed-off-by line" in result.stdout
    assert f"{wrongly_signed[:12]}: signed off by Bob Example <bob@example.com>, but the author is" in result.stdout
    assert "2 problem(s)" in result.stdout


def test_merge_commits_are_skipped(tmp_path, run_check):
    repository = new_repository(tmp_path)
    git(repository, "switch", "--quiet", "-c", "feature")
    commit(repository, "one.txt", "-s", "-m", "Signed")
    git(repository, "switch", "--quiet", "main")
    commit(repository, "main-only.txt", "-s", "-m", "Signed on main")
    git(repository, "switch", "--quiet", "feature")
    git(repository, "merge", "--quiet", "--no-edit", "main")
    result = run_check("check-dco", "--repo", str(repository), "main~1..feature")
    assert result.returncode == 0, result.stdout


def test_tests_leave_a_repository_named_by_git_dir_alone(tmp_path):
    # As under a hook or `git rebase -x` in a linked worktree: GIT_DIR names that worktree's folder inside another
    # repository (a scratch one here, never the real checkout) while this file's tests make, commit to and read
    # their own scratch repositories.
    outer_repository = new_repository(tmp_path)
    git(outer_repository, "worktree", "add", "--quiet", "-b", "linked", str(tmp_path / "linked"))
    refs_before = git(outer_repository, "for-each-ref")
    environment = without_repository_location(dict(os.environ))
    environment["GIT_DIR"] = str(outer_repository / ".git" / "worktrees" / "linked")
    test_file = Path(__file__)
    command = [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", "-k", f"not {FOREIGN_GIT_DIR_TEST}"]
    # Its own scratch folder, so the inner run never tidies away this run's folders.
    command += ["--basetemp", str(tmp_path / "inner"), str(test_file)]
    result = subprocess.run(command, cwd=test_file.parent.parent, env=environment, capture_output=True, text=True)
    assert git(outer_repository, "config", "--get", "core.bare") == "false"
    assert git(outer_repository, "for-each-ref") == refs_before
    assert result.returncode == 0, result.stdout + result.stderr
