"""check-dco on a throwaway git repository with signed, unsigned and wrongly signed commits."""

import os
import subprocess
from pathlib import Path

AUTHOR_NAME = "Ada Example"
AUTHOR_EMAIL = "ada@example.com"
OTHER_SIGN_OFF = "Signed-off-by: Bob Example <bob@example.com>"
# The machine's own git settings (commit signing, hooks, a default branch name) must not change the test.
ISOLATED_GIT_ENVIRONMENT = os.environ | {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}


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
