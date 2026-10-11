"""Tests for tools/github/sync_backlog.py (M1-07), run against a fake `gh` (fake_gh.py) that keeps the repository in a
JSON file.

The acceptance test of M1-07: `--dry-run` on docs/plan/backlog.json prints the expected plan (a snapshot), a second
run against a repository that already has every item plans no changes, and an item whose milestone changed plans one
milestone update.

If the backlog or the plan changes on purpose, the snapshot changes with it. Regenerate it from the repository root
with `PYTHONPATH=tools/github:tools/github/tests .dev/envs/cpu/bin/python tools/github/tests/test_sync_backlog.py`
and review its diff like any other change.
"""

import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import fake_gh
import pytest

import sync_backlog

# This file sits in tools/github/tests/.
TESTS_FOLDER = Path(__file__).resolve().parent
SYNC_SCRIPT = TESTS_FOLDER.parent / "sync_backlog.py"
REPO_ROOT = TESTS_FOLDER.parents[2]
PULL_REQUEST_TEMPLATE = REPO_ROOT / ".github/pull_request_template.md"
SNAPSHOT_PATH = TESTS_FOLDER / "snapshots/dry_run_partial_repo.txt"

# The fake repository has the real name, so the snapshot's blob URLs look like the real ones.
REPO = "Snubiss/RASTAI-Research-Studio"
# The hand-added label every backlog issue may carry; the sync must leave it alone.
HAND_ADDED_LABEL = "bug"
# The item plan 8.2 names as moved: it is a task of M2 that lives in M10.
MOVED_ITEM_ID = "M2-11"
MOVED_ITEM_OLD_MILESTONE = "M2 Unified runtime"
MOVED_ITEM_NEW_MILESTONE = "M10 Prototype exit"


# ---- Helpers ----


def real_inputs() -> sync_backlog.Inputs:
    """The real backlog and plan, with an empty issue map."""
    items = sync_backlog.read_backlog(sync_backlog.DEFAULT_BACKLOG_PATH)
    plan_text = sync_backlog.DEFAULT_PLAN_PATH.read_text(encoding="utf-8")
    label_specs = sync_backlog.read_label_specs(plan_text)
    return sync_backlog.Inputs(items, label_specs, sync_backlog.read_milestone_specs(plan_text), {})


def make_fake_gh(folder: Path, state: dict) -> tuple[Path, Path]:
    """Writes the fake repository and an executable `gh` that serves it. Returns (gh path, state path)."""
    folder.mkdir(parents=True, exist_ok=True)
    state_path = folder / "github.json"
    state_path.write_text(json.dumps(state, indent=1), encoding="utf-8")
    gh_path = folder / "gh"
    gh_path.write_text(
        f"#!{sys.executable}\n"
        "import sys\n"
        f"sys.path.insert(0, {str(TESTS_FOLDER)!r})\n"
        "import fake_gh\n"
        f"sys.exit(fake_gh.main({str(state_path)!r}, sys.argv[1:]))\n",
        encoding="utf-8",
    )
    gh_path.chmod(0o755)
    return gh_path, state_path


def read_state(state_path: Path) -> dict:
    return json.loads(state_path.read_text(encoding="utf-8"))


def run_sync(gh_path: Path, issue_map_path: Path, *arguments: str) -> subprocess.CompletedProcess:
    """Runs the tool as a person would, on the real backlog and plan, with the fake gh."""
    command = [sys.executable, str(SYNC_SCRIPT), "--gh", str(gh_path), "--issue-map", str(issue_map_path), *arguments]
    return subprocess.run(command, capture_output=True, text=True, check=False)


def change_lines(output: str) -> list[str]:
    """The plan lines of the output: everything but the repository line and the summary."""
    lines = output.splitlines()
    return [line for line in lines if not line.startswith(("Repository:", "Plan:", "No changes.", "Wrote "))]


@dataclass
class FirstSync:
    """A fake repository after one real sync from empty, done once for the module because it runs gh about 340
    times. The issue map started with one stale entry, M9-E01 -> #999."""

    state: dict
    issue_map: dict
    output: str


@pytest.fixture(scope="module")
def first_sync(tmp_path_factory: pytest.TempPathFactory) -> FirstSync:
    folder = tmp_path_factory.mktemp("first_sync")
    gh_path, state_path = make_fake_gh(folder, fake_gh.empty_state(REPO))
    issue_map_path = folder / "github-issues.json"
    issue_map_path.write_text(json.dumps({"M9-E01": {"number": 999, "url": "old"}}), encoding="utf-8")
    result = run_sync(gh_path, issue_map_path)
    assert result.returncode == 0, result.stderr
    issue_map = json.loads(issue_map_path.read_text(encoding="utf-8"))
    return FirstSync(read_state(state_path), issue_map, result.stdout)


def synced_repo(folder: Path, first_sync: FirstSync) -> tuple[Path, Path, Path]:
    """A copy of the synced repository and its issue map for one test. Returns (gh path, state path, map path)."""
    state = {**first_sync.state, "calls": []}
    gh_path, state_path = make_fake_gh(folder, state)
    issue_map_path = folder / "github-issues.json"
    issue_map_path.write_text(json.dumps(first_sync.issue_map), encoding="utf-8")
    return gh_path, state_path, issue_map_path


def issues_by_id(state: dict) -> dict[str, dict]:
    return {issue["title"].partition(":")[0]: issue for issue in state["issues"]}


def partial_repo_state(inputs: sync_backlog.Inputs) -> dict:
    """A repository that is part way there, so a dry run plans every kind of create and update:
    two labels missing and one with the wrong color; M11 missing and M3's description stale; four issues present
    (one up to date, one with an old body, one in its old milestone, one with wrong labels), every other one missing."""
    state = fake_gh.empty_state(REPO)
    for spec in inputs.label_specs:
        if spec.name not in ("area:archive", "needs:amd"):
            color = "ffffff" if spec.name == "size:XL" else spec.color
            state["labels"].append({"name": spec.name, "color": color, "description": ""})
    state["labels"].append({"name": HAND_ADDED_LABEL, "color": "d73a4a", "description": ""})
    for number, spec in enumerate(inputs.milestone_specs[:10], start=1):
        description = "An older description" if spec.title == "M3 Inference" else spec.description
        state["milestones"].append({"number": number, "title": spec.title, "description": description, "state": "open"})
    items = {item.id: item for item in inputs.items}
    numbers = {"M1-01": 1, "M1-03": 3, "M1-07": 7, MOVED_ITEM_ID: 67}
    for item_id, number in numbers.items():
        item = items[item_id]
        body = sync_backlog.issue_body(item, REPO, numbers)
        state["issues"].append(fake_issue(number, item.title, body, item.milestone, list(item.labels)))
    issues = {issue["title"].partition(":")[0]: issue for issue in state["issues"]}
    issues["M1-01"]["state"] = "CLOSED"
    issues["M1-07"]["body"] = "An older body"
    issues[MOVED_ITEM_ID]["milestone"] = MOVED_ITEM_OLD_MILESTONE
    issues["M1-03"]["labels"] = ["area:ci", "type:ci", "size:XL", HAND_ADDED_LABEL]
    return state


def fake_issue(number: int, title: str, body: str, milestone: str, labels: list[str]) -> dict:
    url = f"https://github.com/{REPO}/issues/{number}"
    return {
        "number": number,
        "title": title,
        "body": body,
        "milestone": milestone,
        "labels": labels,
        "state": "OPEN",
        "url": url,
    }


def dry_run_output_on_partial_repo(folder: Path) -> str:
    gh_path, state_path = make_fake_gh(folder, partial_repo_state(real_inputs()))
    result = run_sync(gh_path, folder / "github-issues.json", "--dry-run")
    assert result.returncode == 0, result.stderr
    return result.stdout


# ---- The acceptance test of M1-07 ----


def test_dry_run_prints_the_expected_plan(tmp_path: Path) -> None:
    output = dry_run_output_on_partial_repo(tmp_path)
    expected = SNAPSHOT_PATH.read_text(encoding="utf-8")
    assert output == expected, "the plan differs from the snapshot; see the module docstring to regenerate it"


def test_dry_run_writes_nothing(tmp_path: Path) -> None:
    gh_path, state_path = make_fake_gh(tmp_path, partial_repo_state(real_inputs()))
    issue_map_path = tmp_path / "github-issues.json"
    result = run_sync(gh_path, issue_map_path, "--dry-run")
    assert result.returncode == 0, result.stderr
    assert fake_gh.write_calls(read_state(state_path)) == []
    assert not issue_map_path.exists()


def test_second_run_plans_no_changes(tmp_path: Path, first_sync: FirstSync) -> None:
    gh_path, state_path, issue_map_path = synced_repo(tmp_path, first_sync)
    result = run_sync(gh_path, issue_map_path)
    assert result.returncode == 0, result.stderr
    assert change_lines(result.stdout) == []
    assert result.stdout.splitlines()[-1] == "No changes."
    assert fake_gh.write_calls(read_state(state_path)) == []


def test_moved_item_plans_one_milestone_update(tmp_path: Path, first_sync: FirstSync) -> None:
    gh_path, state_path, issue_map_path = synced_repo(tmp_path, first_sync)
    state = read_state(state_path)
    moved_issue = issues_by_id(state)[MOVED_ITEM_ID]
    moved_issue["milestone"] = MOVED_ITEM_OLD_MILESTONE
    state_path.write_text(json.dumps(state), encoding="utf-8")

    result = run_sync(gh_path, issue_map_path, "--dry-run")
    assert result.returncode == 0, result.stderr
    number = moved_issue["number"]
    milestone_change = f'milestone "{MOVED_ITEM_OLD_MILESTONE}" -> "{MOVED_ITEM_NEW_MILESTONE}"'
    expected_line = f"update issue {MOVED_ITEM_ID} (#{number}): {milestone_change}"
    assert change_lines(result.stdout) == [expected_line]
    assert result.stdout.splitlines()[-1] == "Plan: 1 issue to update."

    assert run_sync(gh_path, issue_map_path).returncode == 0
    state = read_state(state_path)
    assert fake_gh.write_calls(state) == [
        ["issue", "edit", str(number), "--repo", REPO, "--milestone", MOVED_ITEM_NEW_MILESTONE]
    ]
    assert issues_by_id(state)[MOVED_ITEM_ID]["milestone"] == MOVED_ITEM_NEW_MILESTONE


# ---- A real run ----


def test_first_run_creates_every_label_milestone_and_issue(first_sync: FirstSync) -> None:
    inputs = real_inputs()
    state = first_sync.state
    assert {label["name"] for label in state["labels"]} == {spec.name for spec in inputs.label_specs}
    assert [milestone["title"] for milestone in state["milestones"]] == [spec.title for spec in inputs.milestone_specs]
    assert [issue["title"] for issue in state["issues"]] == [item.title for item in inputs.items]
    assert issues_by_id(state)[MOVED_ITEM_ID]["milestone"] == MOVED_ITEM_NEW_MILESTONE


def test_dependencies_on_later_issues_get_their_numbers(first_sync: FirstSync) -> None:
    # M1-29 depends on M1-41, which is created after it; the follow-up pass links it by number.
    issues = issues_by_id(first_sync.state)
    assert f"#{issues['M1-41']['number']}" in issues["M1-29"]["body"].splitlines()[-1]
    for item in real_inputs().items:
        for other in item.depends_on:
            assert other not in issues[item.id]["body"].splitlines()[-1], f"{item.id} names {other} by id"


def test_real_run_writes_the_issue_map(first_sync: FirstSync) -> None:
    issues = issues_by_id(first_sync.state)
    # The existing entry keeps its place (first) and gets the real number; the rest follow in backlog order.
    assert list(first_sync.issue_map) == ["M9-E01"] + [item.id for item in real_inputs().items if item.id != "M9-E01"]
    assert first_sync.issue_map["M9-E01"] == {"number": issues["M9-E01"]["number"], "url": issues["M9-E01"]["url"]}
    assert first_sync.output.splitlines()[-1].startswith("Wrote ")


def test_never_closes_reopens_or_deletes(tmp_path: Path, first_sync: FirstSync) -> None:
    gh_path, state_path, issue_map_path = synced_repo(tmp_path, first_sync)
    state = read_state(state_path)
    issues = issues_by_id(state)
    issues["M1-01"]["state"] = "CLOSED"  # closed by verification
    issues["M1-01"]["body"] = "An older body"
    issues["M1-03"]["labels"] += ["size:XL", HAND_ADDED_LABEL]
    state["labels"].append({"name": HAND_ADDED_LABEL, "color": "d73a4a", "description": ""})
    state["issues"].append(fake_issue(500, "A bug report by hand", "Steps", "", [HAND_ADDED_LABEL]))
    state["milestones"].append({"number": 99, "title": "Someday", "description": "", "state": "open"})
    state_path.write_text(json.dumps(state), encoding="utf-8")
    assert run_sync(gh_path, issue_map_path).returncode == 0

    state = read_state(state_path)
    verbs = {call[1] for call in fake_gh.write_calls(state) if call[0] != "api"}
    assert verbs == {"edit"}
    assert HAND_ADDED_LABEL in {label["name"] for label in state["labels"]}
    assert "Someday" in {milestone["title"] for milestone in state["milestones"]}
    issues = issues_by_id(state)
    assert any(issue["number"] == 500 and issue["title"] == "A bug report by hand" for issue in state["issues"])
    assert issues["M1-01"]["state"] == "CLOSED"  # its body was updated, and it stays closed
    assert issues["M1-01"]["body"] != "An older body"
    assert HAND_ADDED_LABEL in issues["M1-03"]["labels"]  # a hand-added label stays; the stale size:XL goes
    assert "size:XL" not in issues["M1-03"]["labels"]


def test_two_issues_for_one_id_stop_the_sync(tmp_path: Path) -> None:
    state = partial_repo_state(real_inputs())
    state["issues"].append(fake_issue(600, "M1-07: a copy", "", "", []))
    gh_path, state_path = make_fake_gh(tmp_path, state)
    result = run_sync(gh_path, tmp_path / "github-issues.json")
    assert result.returncode == 1
    assert "issues #7 and #600 both start with M1-07" in result.stderr
    assert fake_gh.write_calls(read_state(state_path)) == []


def test_repository_comes_from_gh_unless_given(tmp_path: Path) -> None:
    gh_path, _ = make_fake_gh(tmp_path, fake_gh.empty_state(REPO))
    found = run_sync(gh_path, tmp_path / "map.json", "--dry-run")
    assert found.stdout.startswith(f"Repository: {REPO} ")
    given = run_sync(gh_path, tmp_path / "map.json", "--dry-run", "--repo", "someone/else")
    assert given.returncode == 1  # the fake serves only REPO, so the tool used the one it was given
    assert "expected --repo" in given.stderr


# ---- Reading the backlog and the plan ----


def test_plan_tables_give_every_label_and_milestone() -> None:
    inputs = real_inputs()
    names = [spec.name for spec in inputs.label_specs]
    assert len(names) == len(set(names))
    assert {"area:archive", "type:contract", "size:XL", "level:L3", "gate:90-day", "needs:amd"} <= set(names)
    colors = {spec.name.partition(":")[0]: spec.color for spec in inputs.label_specs}
    assert colors == {
        "area": "1d76db",
        "type": "5319e7",
        "size": "c5def5",
        "level": "0e8a16",
        "gate": "fbca04",
        "needs": "d93f0b",
    }
    titles = [spec.title for spec in inputs.milestone_specs]
    assert titles == [f"M{number} " + title.partition(" ")[2] for number, title in enumerate(titles, start=1)]
    assert len(titles) == 11
    assert {item.milestone for item in inputs.items} == set(titles)
    m1 = inputs.milestone_specs[0]
    assert m1.description.startswith("Goal: Monorepo, CI,")
    assert "\n\nExit gate: kill -9 the daemon mid-run" in m1.description


def test_backlog_that_names_an_unknown_label_is_refused(tmp_path: Path) -> None:
    entries = json.loads(sync_backlog.DEFAULT_BACKLOG_PATH.read_text(encoding="utf-8"))
    entries[0]["labels"].append("area:nowhere")
    entries[1]["milestone"] = "M12 Later"
    backlog_path = tmp_path / "backlog.json"
    backlog_path.write_text(json.dumps(entries), encoding="utf-8")
    with pytest.raises(sync_backlog.SyncError) as error:
        sync_backlog.read_inputs(backlog_path, sync_backlog.DEFAULT_PLAN_PATH, tmp_path / "map.json")
    assert "M1-01: label 'area:nowhere' isn't in plan 8.3" in str(error.value)
    assert "M1-01a: milestone 'M12 Later' isn't a row of plan 2.1" in str(error.value)


def test_relative_links_become_blob_urls_on_main() -> None:
    text = (
        "[Backlog](docs/plan/backlog.json), [Guide](CONTRIBUTING.md#commits), "
        "[Web](https://example.com/a), [Here](#goal), [Root](/README.md), `](docs/...`)"
    )
    blob = f"https://github.com/{REPO}/blob/main/"
    assert sync_backlog.rewrite_links(text, REPO) == (
        f"[Backlog]({blob}docs/plan/backlog.json), [Guide]({blob}CONTRIBUTING.md#commits), "
        f"[Web](https://example.com/a), [Here](#goal), [Root](/README.md), `]({blob}docs/...`)"
    )


def test_issue_body_ends_with_its_dependencies() -> None:
    item = sync_backlog.BacklogItem("M1-07", "M1-07: x", "M1 Foundation and skeleton", [], ["M1-04", "M1-05"], "Body\n")
    assert sync_backlog.issue_body(item, REPO, {"M1-04": 4}) == "Body\n\nDepends on: #4, M1-05\n"
    item.depends_on = []
    assert sync_backlog.issue_body(item, REPO, {}) == "Body\n"


# ---- The PR template (plan 8.5) ----


def test_pull_request_template_has_the_plan_lines() -> None:
    template = PULL_REQUEST_TEMPLATE.read_text(encoding="utf-8")
    assert "Refs #123" in template
    assert "Closes #" not in template
    plan_text = sync_backlog.DEFAULT_PLAN_PATH.read_text(encoding="utf-8")
    section = plan_text[plan_text.index("### 8.5 The PR template") : plan_text.index("### 8.6 DCO")]
    plan_lines = [line for line in section.splitlines() if line.startswith("- [ ] ")]
    assert len(plan_lines) == 4
    for line in plan_lines:
        assert line in template.splitlines()


def write_snapshot() -> None:
    """Regenerates the dry-run snapshot after an intended change to the backlog, the plan or the tool."""
    import tempfile

    with tempfile.TemporaryDirectory() as folder:
        SNAPSHOT_PATH.write_text(dry_run_output_on_partial_repo(Path(folder)), encoding="utf-8")
    print(f"Wrote {os.path.relpath(SNAPSHOT_PATH)}")


if __name__ == "__main__":
    write_snapshot()
