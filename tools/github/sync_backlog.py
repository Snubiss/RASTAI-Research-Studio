#!/usr/bin/env python3
"""sync_backlog: labels, milestones and issues on GitHub from docs/plan/backlog.json (plan 8.2, 8.3, 8.9; M1-07).

Standard library plus the `gh` CLI, which holds the sign-in. What it does:

- Labels: every label of plan 8.3 exists with its family's color. A label's description is hand-written on GitHub,
  so the tool sets none and leaves the existing ones alone.
- Milestones: one per row of plan 2.1, titled as in backlog.json, its description the row's goal and exit gate.
- Issues: one per backlog item, matched by the id at the start of the title ("M1-07: ..."). Title, body, milestone
  and the plan 8.3 labels follow the item; a moved item moves its issue (M2-11 is in M10). Repo-relative links in
  a body become blob URLs on main, and the item's `depends_on` becomes a "Depends on: #n" line.
- docs/plan/github-issues.json maps each id to its issue number and URL after a real run.

It never deletes or closes anything: issues close only by verification (plan 3.6). Running it twice changes nothing
the second time. `--dry-run` prints the plan and writes nothing, here or on GitHub.
"""

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

# This file sits in tools/github/.
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BACKLOG_PATH = REPO_ROOT / "docs/plan/backlog.json"
DEFAULT_PLAN_PATH = REPO_ROOT / "docs/plan/implementation-plan.md"
DEFAULT_ISSUE_MAP_PATH = REPO_ROOT / "docs/plan/github-issues.json"

# The plan sections the labels and milestones are read from, so the plan stays their one source.
LABELS_HEADING = "### 8.3 Labels"
MILESTONES_HEADING = "### 2.1 At a glance"
MILESTONE_TITLE_COLUMN = "Milestone"
MILESTONE_GOAL_COLUMN = "Goal"
MILESTONE_EXIT_GATE_COLUMN = "Exit gate"  # the header reads "Exit gate (the test that closes it)"

# Links in issue bodies point at files on this branch.
BLOB_BRANCH = "main"
# A Markdown link target: "](docs/plan/backlog.json)". The target runs to the first ")" or space.
MARKDOWN_LINK_TARGET = re.compile(r"\]\(([^)\s]+)\)")
# A target that already names a scheme (https:, mailto:) isn't repo-relative.
URL_SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
# A backtick-quoted value in a plan table cell, and a color in one.
QUOTED_VALUE = re.compile(r"`([^`]+)`")
HEX_COLOR = re.compile(r"#([0-9A-Fa-f]{6})\b")

# gh lists 30 items unless told otherwise; these are far above what the repository holds.
LABEL_LIST_LIMIT = 1000
ISSUE_LIST_LIMIT = 10000
MILESTONES_PER_PAGE = 100
ISSUE_FIELDS = "number,title,body,milestone,labels,url"
# How the plan's last line names each kind of change, one and many.
KIND_NOUNS = {
    "label": ("label", "labels"),
    "milestone": ("milestone", "milestones"),
    "issue": ("issue", "issues"),
    "issue map": ("issue map entry", "issue map entries"),
}


class SyncError(Exception):
    """A problem that stops the sync before or while it writes: a bad input file, a gh failure, an ambiguity."""


@dataclass
class LabelSpec:
    name: str
    color: str  # six hex digits, lower case, no "#" (as gh reports them)


@dataclass
class MilestoneSpec:
    title: str
    description: str


@dataclass
class BacklogItem:
    id: str
    title: str
    milestone: str
    labels: list[str]
    depends_on: list[str]
    body: str


@dataclass
class GitHubMilestone:
    number: int
    title: str
    description: str


@dataclass
class GitHubIssue:
    number: int
    title: str
    body: str
    milestone: str  # the milestone's title; "" when the issue has none
    labels: list[str]
    url: str


@dataclass
class GitHubState:
    """What the repository holds now: label name -> color, milestones by title, backlog issues by item id."""

    label_colors: dict[str, str] = field(default_factory=dict)
    milestones: dict[str, GitHubMilestone] = field(default_factory=dict)
    issues: dict[str, GitHubIssue] = field(default_factory=dict)


@dataclass
class Change:
    """One planned write. `kind` is label, milestone, issue or issue map; `action` is create or update."""

    kind: str
    action: str
    name: str  # label name, milestone title or backlog item id
    summary: str  # the plan line printed for it
    fields: dict = field(default_factory=dict)


@dataclass
class Inputs:
    """Everything the sync reads from the repository."""

    items: list[BacklogItem]
    label_specs: list[LabelSpec]
    milestone_specs: list[MilestoneSpec]
    issue_map: dict


# ---- The gh CLI ----


class GhCli:
    """Every call to `gh`. Each method is one gh command, so the commands the tool can run are all listed here."""

    def __init__(self, program: str, repo: str) -> None:
        self.program = program
        self.repo = repo

    def run(self, arguments: list[str], stdin_text: str | None = None) -> str:
        command = [self.program, *arguments]
        try:
            result = subprocess.run(command, input=stdin_text, capture_output=True, text=True, check=False)
        except FileNotFoundError as error:
            message = f"can't run {self.program}: install the GitHub CLI and sign in with `gh auth login`"
            raise SyncError(message) from error
        if result.returncode != 0:
            raise SyncError(f"`gh {' '.join(arguments[:2])}` failed: {result.stderr.strip()}")
        return result.stdout

    def find_repo(self) -> str:
        """The repository gh resolves from the current checkout's remote, as OWNER/NAME."""
        return self.run(["repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"]).strip()

    def read_label_colors(self) -> dict[str, str]:
        arguments = ["label", "list", "--repo", self.repo, "--limit", str(LABEL_LIST_LIMIT)]
        output = self.run([*arguments, "--json", "name,color"])
        return {label["name"]: label["color"].lower() for label in json.loads(output)}

    def read_milestones(self) -> list[GitHubMilestone]:
        # --slurp wraps the pages in one array, so this is a list of pages, each a list of milestones.
        path = f"repos/{self.repo}/milestones?state=all&per_page={MILESTONES_PER_PAGE}"
        pages = json.loads(self.run(["api", "--paginate", "--slurp", path]))
        milestones = []
        for page in pages:
            for milestone in page:
                description = milestone.get("description") or ""
                milestones.append(GitHubMilestone(milestone["number"], milestone["title"], description))
        return milestones

    def read_issues(self) -> list[GitHubIssue]:
        arguments = ["issue", "list", "--repo", self.repo, "--state", "all", "--limit", str(ISSUE_LIST_LIMIT)]
        output = self.run([*arguments, "--json", ISSUE_FIELDS])
        return [issue_from_gh(issue) for issue in json.loads(output)]

    def create_label(self, spec: LabelSpec) -> None:
        self.run(["label", "create", spec.name, "--repo", self.repo, "--color", spec.color])

    def update_label_color(self, name: str, color: str) -> None:
        self.run(["label", "edit", name, "--repo", self.repo, "--color", color])

    def create_milestone(self, spec: MilestoneSpec) -> None:
        path = f"repos/{self.repo}/milestones"
        self.run(["api", "-X", "POST", path, "-f", f"title={spec.title}", "-f", f"description={spec.description}"])

    def update_milestone_description(self, number: int, description: str) -> None:
        path = f"repos/{self.repo}/milestones/{number}"
        self.run(["api", "-X", "PATCH", path, "-f", f"description={description}"])

    def create_issue(self, title: str, body: str, milestone: str, labels: list[str]) -> None:
        arguments = ["issue", "create", "--repo", self.repo, "--title", title, "--body-file", "-"]
        arguments += ["--milestone", milestone]
        for label in labels:
            arguments += ["--label", label]
        self.run(arguments, stdin_text=body)

    def update_issue(self, number: int, fields: dict) -> None:
        arguments = ["issue", "edit", str(number), "--repo", self.repo]
        body = None
        if "title" in fields:
            arguments += ["--title", fields["title"]]
        if "body" in fields:
            arguments += ["--body-file", "-"]
            body = fields["body"]
        if "milestone" in fields:
            arguments += ["--milestone", fields["milestone"]]
        for label in fields.get("add_labels", []):
            arguments += ["--add-label", label]
        for label in fields.get("remove_labels", []):
            arguments += ["--remove-label", label]
        self.run(arguments, stdin_text=body)


def issue_from_gh(issue: dict) -> GitHubIssue:
    """An issue as `gh issue list --json` reports it: the milestone is an object or null, the labels objects."""
    milestone = issue.get("milestone") or {}
    milestone_title = milestone.get("title", "")
    labels = [label["name"] for label in issue.get("labels", [])]
    body = issue.get("body") or ""
    return GitHubIssue(issue["number"], issue["title"], body, milestone_title, labels, issue["url"])


# ---- Reading the repository's files ----


def read_inputs(backlog_path: Path, plan_path: Path, issue_map_path: Path) -> Inputs:
    items = read_backlog(backlog_path)
    plan_text = read_text(plan_path)
    inputs = Inputs(items, read_label_specs(plan_text), read_milestone_specs(plan_text), read_issue_map(issue_map_path))
    check_backlog(inputs)
    return inputs


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise SyncError(f"can't read {path}: {error.strerror}") from error


def read_backlog(path: Path) -> list[BacklogItem]:
    try:
        entries = json.loads(read_text(path))
        return [BacklogItem(**entry) for entry in entries]
    except (ValueError, TypeError) as error:
        raise SyncError(f"{path} isn't a list of backlog items (plan 8.9): {error}") from error


def read_issue_map(path: Path) -> dict:
    """github-issues.json: item id -> {number, url}. A missing file is an empty map."""
    if not path.exists():
        return {}
    try:
        return json.loads(read_text(path))
    except ValueError as error:
        raise SyncError(f"{path} isn't valid JSON: {error}") from error


def read_plan_table(plan_text: str, heading: str) -> list[list[str]]:
    """The first Markdown table after `heading`, as rows of cells; the header row first, the --- row left out."""
    start = plan_text.find(f"\n{heading}\n")
    if start < 0:
        raise SyncError(f"the plan has no section {heading!r}")
    rows = []
    for line in plan_text[start + len(heading) + 2 :].splitlines():
        if line.startswith("|"):
            rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
        elif rows:
            break  # the first line after the table ends it
    return [row for row in rows if not is_separator_row(row)]


def is_separator_row(row: list[str]) -> bool:
    """The |---|:---| line under a table's header holds only dashes and colons."""
    return all(cell.replace("-", "").replace(":", "") == "" for cell in row)


def read_label_specs(plan_text: str) -> list[LabelSpec]:
    """Plan 8.3: one row per family (`area:*`), its labels as backtick-quoted values, and the family's color."""
    specs = []
    for row in read_plan_table(plan_text, LABELS_HEADING)[1:]:
        family_cell, labels_cell, color_cell = row[0], row[1], row[2]
        prefix = family_cell.strip("`").removesuffix("*")
        colors = HEX_COLOR.findall(color_cell)
        if not prefix.endswith(":") or len(colors) != 1:
            raise SyncError(f"plan 8.3 row {family_cell!r} doesn't name a family and one color")
        specs += [LabelSpec(prefix + value, colors[0].lower()) for value in QUOTED_VALUE.findall(labels_cell)]
    if not specs:
        raise SyncError("plan 8.3 lists no labels")
    return specs


def read_milestone_specs(plan_text: str) -> list[MilestoneSpec]:
    """Plan 2.1: one row per milestone. The description is the row's goal and its exit gate (plan 8.2)."""
    header, *rows = read_plan_table(plan_text, MILESTONES_HEADING)
    title_column = column_index(header, MILESTONE_TITLE_COLUMN)
    goal_column = column_index(header, MILESTONE_GOAL_COLUMN)
    exit_gate_column = column_index(header, MILESTONE_EXIT_GATE_COLUMN)
    specs = []
    for row in rows:
        if len(row) != len(header):
            raise SyncError(f"plan 2.1 row {row[0]!r} has {len(row)} cells, the header {len(header)}")
        description = f"Goal: {row[goal_column]}\n\nExit gate: {row[exit_gate_column]}"
        specs.append(MilestoneSpec(row[title_column], description))
    return specs


def column_index(header: list[str], name: str) -> int:
    for index, cell in enumerate(header):
        if cell.startswith(name):
            return index
    raise SyncError(f"plan 2.1 has no {name!r} column")


def check_backlog(inputs: Inputs) -> None:
    """Refuses a backlog GitHub would reject halfway through: a repeated id, an unknown label or milestone."""
    label_names = {spec.name for spec in inputs.label_specs}
    milestone_titles = {spec.title for spec in inputs.milestone_specs}
    item_ids = [item.id for item in inputs.items]
    problems = []
    for item in inputs.items:
        if item_ids.count(item.id) > 1:
            problems.append(f"{item.id}: the id is used more than once")
        if not item.title.startswith(f"{item.id}: "):
            problems.append(f'{item.id}: the title must start with "{item.id}: " so its issue can be found')
        if item.milestone not in milestone_titles:
            problems.append(f"{item.id}: milestone {item.milestone!r} isn't a row of plan 2.1")
        problems += [
            f"{item.id}: label {label!r} isn't in plan 8.3" for label in item.labels if label not in label_names
        ]
        problems += [
            f"{item.id}: depends on unknown item {other}" for other in item.depends_on if other not in item_ids
        ]
    if problems:
        raise SyncError("the backlog doesn't match the plan:\n  " + "\n  ".join(sorted(set(problems))))


# ---- Reading GitHub ----


def read_github_state(gh: GhCli, items: list[BacklogItem]) -> GitHubState:
    state = GitHubState(gh.read_label_colors())
    state.milestones = {milestone.title: milestone for milestone in gh.read_milestones()}
    state.issues = match_issues(gh.read_issues(), {item.id for item in items})
    return state


def match_issues(issues: list[GitHubIssue], item_ids: set[str]) -> dict[str, GitHubIssue]:
    """Backlog issues by the id their title starts with. Two issues for one id stop the sync: guessing which one
    is the real one could rewrite the wrong issue."""
    matched: dict[str, GitHubIssue] = {}
    for issue in issues:
        item_id = issue.title.partition(":")[0]
        if item_id not in item_ids:
            continue  # an issue that isn't from the backlog, such as a bug report
        if item_id in matched:
            numbers = sorted([matched[item_id].number, issue.number])
            raise SyncError(f"issues #{numbers[0]} and #{numbers[1]} both start with {item_id}; keep one")
        matched[item_id] = issue
    return matched


# ---- Issue bodies ----


def rewrite_links(text: str, repo: str) -> str:
    """Turns repo-relative link targets, such as "](docs/plan/backlog.json)", into blob URLs on main. Absolute URLs
    and in-page anchors are left as they are."""
    blob_prefix = f"https://github.com/{repo}/blob/{BLOB_BRANCH}/"

    def rewrite(match: re.Match) -> str:
        target = match.group(1)
        if URL_SCHEME.match(target) or target.startswith(("#", "/")):
            return match.group(0)
        return f"]({blob_prefix}{target})"

    return MARKDOWN_LINK_TARGET.sub(rewrite, text)


def issue_body(item: BacklogItem, repo: str, issue_numbers: dict[str, int]) -> str:
    """The item's body with its links rewritten, and a "Depends on: #4, #5" line that GitHub turns into links. A
    dependency with no issue yet (a dry run that would create it) is named by its id."""
    body = rewrite_links(item.body, repo)
    if not item.depends_on:
        return body
    references = [f"#{issue_numbers[other]}" if other in issue_numbers else other for other in item.depends_on]
    return f"{body}\nDepends on: {', '.join(references)}\n"


# ---- Planning ----


def make_plan(inputs: Inputs, state: GitHubState, repo: str) -> list[Change]:
    changes = plan_labels(inputs.label_specs, state)
    changes += plan_milestones(inputs.milestone_specs, state)
    changes += plan_issues(inputs, state, repo)
    changes += plan_issue_map(inputs, state)
    return changes


def plan_labels(specs: list[LabelSpec], state: GitHubState) -> list[Change]:
    changes = []
    for spec in specs:
        color = state.label_colors.get(spec.name)
        if color is None:
            changes.append(Change("label", "create", spec.name, f"create label {spec.name} (#{spec.color})"))
        elif color != spec.color:
            summary = f"update label {spec.name}: color #{color} -> #{spec.color}"
            changes.append(Change("label", "update", spec.name, summary, {"color": spec.color}))
    return changes


def plan_milestones(specs: list[MilestoneSpec], state: GitHubState) -> list[Change]:
    changes = []
    for spec in specs:
        milestone = state.milestones.get(spec.title)
        if milestone is None:
            changes.append(Change("milestone", "create", spec.title, f'create milestone "{spec.title}"'))
        elif milestone.description != spec.description:
            summary = f'update milestone "{spec.title}": description'
            changes.append(Change("milestone", "update", spec.title, summary, {"number": milestone.number}))
    return changes


def plan_issues(inputs: Inputs, state: GitHubState, repo: str) -> list[Change]:
    issue_numbers = {item_id: issue.number for item_id, issue in state.issues.items()}
    plan_label_names = {spec.name for spec in inputs.label_specs}
    changes = []
    for item in inputs.items:
        body = issue_body(item, repo, issue_numbers)
        issue = state.issues.get(item.id)
        if issue is None:
            changes.append(Change("issue", "create", item.id, f"create issue {item.title}", {"body": body}))
            continue
        fields = issue_differences(item, issue, body, plan_label_names)
        if fields:
            summary = f"update issue {item.id} (#{issue.number}): {describe_issue_fields(fields, issue)}"
            changes.append(Change("issue", "update", item.id, summary, {"number": issue.number, **fields}))
    return changes


def issue_differences(item: BacklogItem, issue: GitHubIssue, body: str, plan_label_names: set[str]) -> dict:
    """The fields of `issue` that differ from the backlog. Only the plan 8.3 labels are the backlog's: a label
    someone added by hand (bug, help wanted) stays."""
    fields = {}
    if issue.title != item.title:
        fields["title"] = item.title
    if issue.body != body:
        fields["body"] = body
    if issue.milestone != item.milestone:
        fields["milestone"] = item.milestone
    add_labels = [label for label in item.labels if label not in issue.labels]
    remove_labels = [label for label in issue.labels if label in plan_label_names and label not in item.labels]
    if add_labels:
        fields["add_labels"] = add_labels
    if remove_labels:
        fields["remove_labels"] = remove_labels
    return fields


def describe_issue_fields(fields: dict, issue: GitHubIssue) -> str:
    parts = []
    if "title" in fields:
        parts.append("title")
    if "body" in fields:
        parts.append("body")
    if "milestone" in fields:
        parts.append(f'milestone "{issue.milestone or "none"}" -> "{fields["milestone"]}"')
    label_changes = [f"+{label}" for label in fields.get("add_labels", [])]
    label_changes += [f"-{label}" for label in fields.get("remove_labels", [])]
    if label_changes:
        parts.append("labels " + " ".join(label_changes))
    return "; ".join(parts)


def plan_issue_map(inputs: Inputs, state: GitHubState) -> list[Change]:
    """github-issues.json entries that don't match the issues found on GitHub. Issues created by this run are added
    to the map after they exist, since their numbers aren't known before."""
    changes = []
    for item in inputs.items:
        issue = state.issues.get(item.id)
        if issue is None:
            continue
        entry = {"number": issue.number, "url": issue.url}
        if inputs.issue_map.get(item.id) != entry:
            summary = f"update issue map {item.id} -> #{issue.number}"
            changes.append(Change("issue map", "update", item.id, summary, entry))
    return changes


def plan_summary(changes: list[Change]) -> str:
    if not changes:
        return "No changes."
    counts: dict[tuple[str, str], int] = {}
    for change in changes:
        key = (change.kind, change.action)
        counts[key] = counts.get(key, 0) + 1
    parts = []
    for (kind, action), count in counts.items():
        singular, plural = KIND_NOUNS[kind]
        parts.append(f"{count} {singular if count == 1 else plural} to {action}")
    return "Plan: " + ", ".join(parts) + "."


def print_plan(changes: list[Change]) -> None:
    for change in changes:
        print(change.summary)
    print(plan_summary(changes))


# ---- Applying ----


def apply_changes(gh: GhCli, changes: list[Change], inputs: Inputs) -> None:
    """Writes the plan in order: labels and milestones first, because new issues name them."""
    label_specs = {spec.name: spec for spec in inputs.label_specs}
    milestone_specs = {spec.title: spec for spec in inputs.milestone_specs}
    items = {item.id: item for item in inputs.items}
    for change in changes:
        if change.kind == "label":
            apply_label_change(gh, change, label_specs[change.name])
        elif change.kind == "milestone":
            apply_milestone_change(gh, change, milestone_specs[change.name])
        elif change.kind == "issue":
            apply_issue_change(gh, change, items[change.name])
        # issue map changes are written once at the end, from the final state (write_issue_map)


def apply_label_change(gh: GhCli, change: Change, spec: LabelSpec) -> None:
    if change.action == "create":
        gh.create_label(spec)
    else:
        gh.update_label_color(spec.name, spec.color)


def apply_milestone_change(gh: GhCli, change: Change, spec: MilestoneSpec) -> None:
    if change.action == "create":
        gh.create_milestone(spec)
    else:
        gh.update_milestone_description(change.fields["number"], spec.description)


def apply_issue_change(gh: GhCli, change: Change, item: BacklogItem) -> None:
    if change.action == "create":
        gh.create_issue(item.title, change.fields["body"], item.milestone, item.labels)
    else:
        fields = {key: value for key, value in change.fields.items() if key != "number"}
        gh.update_issue(change.fields["number"], fields)


def write_issue_map(path: Path, inputs: Inputs, state: GitHubState) -> bool:
    """Writes github-issues.json from the issues on GitHub. Existing entries keep their order and new ones go at
    the end, so the diff shows only what changed. Returns whether the file changed."""
    issue_map = dict(inputs.issue_map)
    for item in inputs.items:
        issue = state.issues.get(item.id)
        if issue is not None:
            issue_map[item.id] = {"number": issue.number, "url": issue.url}
    if issue_map == inputs.issue_map and path.exists():
        return False
    path.write_text(json.dumps(issue_map, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return True


# ---- Main ----


def parse_arguments(arguments: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Sync labels, milestones and issues to GitHub from backlog.json.")
    parser.add_argument("--dry-run", action="store_true", help="print the plan; write nothing")
    parser.add_argument("--repo", help="OWNER/NAME; default: the repository gh finds from this checkout")
    parser.add_argument("--gh", default="gh", help="the gh program to run (default: gh)")
    parser.add_argument("--backlog", type=Path, default=DEFAULT_BACKLOG_PATH)
    parser.add_argument("--plan", type=Path, default=DEFAULT_PLAN_PATH)
    parser.add_argument("--issue-map", type=Path, default=DEFAULT_ISSUE_MAP_PATH)
    return parser.parse_args(arguments)


def run_sync(options: argparse.Namespace) -> None:
    inputs = read_inputs(options.backlog, options.plan, options.issue_map)
    gh = GhCli(options.gh, options.repo or "")
    if not gh.repo:
        gh.repo = gh.find_repo()
    print(f"Repository: {gh.repo}{' (dry run: nothing is written)' if options.dry_run else ''}")
    state = read_github_state(gh, inputs.items)
    changes = make_plan(inputs, state, gh.repo)
    print_plan(changes)
    if options.dry_run or not changes:
        return
    apply_changes(gh, changes, inputs)
    # An issue created this run may depend on one created after it, so its "Depends on" line could only name the
    # id. Now that every issue has a number, plan once more; what is left is those body updates.
    state = read_github_state(gh, inputs.items)
    follow_up = [change for change in plan_issues(inputs, state, gh.repo) if change.action == "update"]
    if follow_up:
        print_plan(follow_up)
        apply_changes(gh, follow_up, inputs)
    if write_issue_map(options.issue_map, inputs, state):
        print(f"Wrote {options.issue_map}")


def main(arguments: list[str] | None = None) -> int:
    options = parse_arguments(arguments)
    try:
        run_sync(options)
    except SyncError as error:
        print(f"sync_backlog: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
