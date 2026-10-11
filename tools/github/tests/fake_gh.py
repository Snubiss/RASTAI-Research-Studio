"""A fake `gh` for the sync tests: a JSON file stands in for the GitHub repository.

It answers exactly the gh commands sync_backlog.py runs and refuses every other one, so a test fails if the tool
ever tries something it shouldn't (closing an issue, deleting a label). Every command is recorded in the state's
"calls" list, so a test can tell whether a run wrote anything.

The tests run it through a small executable that `make_fake_gh` in test_sync_backlog.py writes, which calls
main(state_path, arguments).
"""

import json
import sys
from pathlib import Path

# Commands that only read. Everything else the fake accepts is a write.
READ_COMMANDS = {("repo", "view"), ("label", "list"), ("issue", "list"), ("api", "GET")}


def empty_state(repo: str) -> dict:
    return {"repo": repo, "labels": [], "milestones": [], "issues": [], "calls": []}


def main(state_path: str, arguments: list[str]) -> int:
    path = Path(state_path)
    state = json.loads(path.read_text(encoding="utf-8"))
    state["calls"].append(arguments)
    try:
        output = run_command(state, arguments)
    except FakeGhError as error:
        print(f"fake gh: {error}", file=sys.stderr)
        path.write_text(json.dumps(state), encoding="utf-8")
        return 1
    path.write_text(json.dumps(state), encoding="utf-8")
    if output:
        print(output)
    return 0


class FakeGhError(Exception):
    pass


def run_command(state: dict, arguments: list[str]) -> str:
    words, options = split_arguments(arguments)
    command = tuple(words[:2])
    if command == ("repo", "view"):
        return state["repo"]
    if words[0] == "api":
        return run_api(state, words[1], options)
    check_repo(state, options)
    if command == ("label", "list"):
        return list_labels(state)
    if command == ("label", "create"):
        return create_label(state, words[2], options)
    if command == ("label", "edit"):
        return edit_label(state, words[2], options)
    if command == ("issue", "list"):
        return list_issues(state)
    if command == ("issue", "create"):
        return create_issue(state, options)
    if command == ("issue", "edit"):
        return edit_issue(state, int(words[2]), options)
    raise FakeGhError(f"unsupported command: {' '.join(arguments)}")


def split_arguments(arguments: list[str]) -> tuple[list[str], dict[str, list[str]]]:
    """Positional words, and each option's values in order. --paginate and --slurp take no value."""
    flags_without_value = {"--paginate", "--slurp"}
    words, options = [], {}
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument in flags_without_value:
            options.setdefault(argument, []).append("")
            index += 1
        elif argument.startswith("-"):
            options.setdefault(argument, []).append(arguments[index + 1])
            index += 2
        else:
            words.append(argument)
            index += 1
    return words, options


def check_repo(state: dict, options: dict) -> None:
    if options.get("--repo") != [state["repo"]]:
        raise FakeGhError(f"expected --repo {state['repo']}, got {options.get('--repo')}")


def one_option(options: dict, name: str) -> str:
    return options[name][-1]


def list_labels(state: dict) -> str:
    return json.dumps([{"name": label["name"], "color": label["color"]} for label in state["labels"]])


def create_label(state: dict, name: str, options: dict) -> str:
    if any(label["name"] == name for label in state["labels"]):
        raise FakeGhError(f"label {name} already exists")
    state["labels"].append({"name": name, "color": one_option(options, "--color"), "description": ""})
    return ""


def edit_label(state: dict, name: str, options: dict) -> str:
    for label in state["labels"]:
        if label["name"] == name:
            label["color"] = one_option(options, "--color")
            return ""
    raise FakeGhError(f"no label {name}")


def run_api(state: dict, first_word: str, options: dict) -> str:
    method = one_option(options, "-X") if "-X" in options else "GET"
    path = first_word
    prefix = f"repos/{state['repo']}/milestones"
    if method == "GET" and path.startswith(prefix + "?") and "--slurp" in options:
        return json.dumps([state["milestones"]])
    fields = dict(value.split("=", 1) for value in options.get("-f", []))
    if method == "POST" and path == prefix:
        number = max([milestone["number"] for milestone in state["milestones"]], default=0) + 1
        milestone = {"number": number, "title": fields["title"], "description": fields["description"], "state": "open"}
        state["milestones"].append(milestone)
        return json.dumps(milestone)
    if method == "PATCH" and path.startswith(prefix + "/"):
        milestone = find_milestone_by_number(state, int(path.rsplit("/", 1)[1]))
        milestone.update(fields)
        return json.dumps(milestone)
    raise FakeGhError(f"unsupported api call: {method} {path}")


def find_milestone_by_number(state: dict, number: int) -> dict:
    for milestone in state["milestones"]:
        if milestone["number"] == number:
            return milestone
    raise FakeGhError(f"no milestone {number}")


def check_milestone_title(state: dict, title: str) -> None:
    if not any(milestone["title"] == title for milestone in state["milestones"]):
        raise FakeGhError(f"no milestone titled {title!r}")


def check_labels_exist(state: dict, names: list[str]) -> None:
    known = {label["name"] for label in state["labels"]}
    for name in names:
        if name not in known:
            raise FakeGhError(f"no label {name}")


def list_issues(state: dict) -> str:
    issues = []
    for issue in state["issues"]:
        milestone = {"title": issue["milestone"]} if issue["milestone"] else None
        labels = [{"name": name} for name in issue["labels"]]
        issues.append({**issue, "milestone": milestone, "labels": labels})
    return json.dumps(issues)


def read_body(options: dict) -> str:
    if one_option(options, "--body-file") != "-":
        raise FakeGhError("the fake reads bodies from stdin only")
    return sys.stdin.read()


def create_issue(state: dict, options: dict) -> str:
    milestone = one_option(options, "--milestone")
    labels = options.get("--label", [])
    check_milestone_title(state, milestone)
    check_labels_exist(state, labels)
    number = max([issue["number"] for issue in state["issues"]], default=0) + 1
    url = f"https://github.com/{state['repo']}/issues/{number}"
    issue = {"number": number, "title": one_option(options, "--title"), "body": read_body(options)}
    issue.update({"milestone": milestone, "labels": labels, "state": "OPEN", "url": url})
    state["issues"].append(issue)
    return url


def edit_issue(state: dict, number: int, options: dict) -> str:
    issue = next((issue for issue in state["issues"] if issue["number"] == number), None)
    if issue is None:
        raise FakeGhError(f"no issue #{number}")
    if "--title" in options:
        issue["title"] = one_option(options, "--title")
    if "--body-file" in options:
        issue["body"] = read_body(options)
    if "--milestone" in options:
        check_milestone_title(state, one_option(options, "--milestone"))
        issue["milestone"] = one_option(options, "--milestone")
    check_labels_exist(state, options.get("--add-label", []))
    issue["labels"] += [label for label in options.get("--add-label", []) if label not in issue["labels"]]
    issue["labels"] = [label for label in issue["labels"] if label not in options.get("--remove-label", [])]
    return issue["url"]


def write_calls(state: dict) -> list[list[str]]:
    """The recorded commands that would change the repository."""
    writes = []
    for call in state["calls"]:
        words, options = split_arguments(call)
        method = one_option(options, "-X") if "-X" in options else "GET"
        key = (words[0], method) if words[0] == "api" else tuple(words[:2])
        if key not in READ_COMMANDS:
            writes.append(call)
    return writes
