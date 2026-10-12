"""check-schema-diff on throwaway git repositories: a base commit on main with command and event schema files and a
contracts excerpt, then a change in the working tree (or a commit) on a feature branch. One case per rule of
contracts 12.6, with the passing twin of each failing case."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

# repo_files.py sits one folder up, beside the check scripts that import it.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from repo_files import git_environment  # noqa: E402

CHECK = "check-schema-diff"
# This file sits in tools/checks/tests/.
CHECKS_FOLDER = Path(__file__).resolve().parent.parent
REPO_ROOT = CHECKS_FOLDER.parent.parent
CONTRACTS_FILE = "docs/architecture/contracts.md"
COMMAND_PATH = "schemas/commands/rastai.command.sample.get/1.json"
OTHER_COMMAND_PATH = "schemas/commands/rastai.command.sample.list/1.json"
EVENT_PATH = "schemas/events/sample.changed.json"
BASE_CONTRACTS = "# Contracts\n\n| `sample.get` | get one sample |\n| `sample.changed` | durable |\n"
COMMAND_CONTRACTS_LINE = "- `sample.get`: its arguments changed.\n"
EVENT_CONTRACTS_LINE = "- `sample.changed`: its payload changed.\n"
AUTHOR = ["-c", "user.name=Ada Example", "-c", "user.email=ada@example.com"]
# Two protobuf JSON fields as codegen writes them: the field's doc, then the sentence that names the message.
TELEMETRY_FIELD = {"type": "object", "description": "Its telemetry. Protobuf JSON of rastai.common.v1.NodeTelemetry."}
DIAGNOSTIC_FIELD = {"type": "object", "description": "Its telemetry. Protobuf JSON of rastai.common.v1.Diagnostic."}
# The machine's own git settings (commit signing, hooks, a default branch name) must not change the test.
MACHINE_CONFIG_OFF = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
# The M1 event types (contracts 7.3) that schemas/events/ must hold one file each for.
M1_EVENT_TYPES = [
    "job.state_changed",
    "job.progress",
    "job.log_line",
    "run.state_changed",
    "run.metrics",
    "run.tags_changed",
    "checkpoint.saved",
    "artifact.created",
    "pin.changed",
    "node.heartbeat",
    "node.state_changed",
    "document.changed",
    "command.finished",
    "plugin.state_changed",
    "diagnostic.reported",
]


def isolated_environment(base_ref: str = "main") -> dict[str, str]:
    # Without the repository variables a hook sets, so the scratch repository is found from -C alone.
    environment = git_environment()
    environment.update(MACHINE_CONFIG_OFF)
    environment["RASTAI_BASE_REF"] = base_ref
    return environment


def git(repository: Path, *arguments: str) -> str:
    command = ["git", "-C", str(repository), *AUTHOR, *arguments]
    result = subprocess.run(command, capture_output=True, text=True, check=True, env=isolated_environment())
    return result.stdout.strip()


# ---- Fixture files, shaped like the ones codegen writes ----


def command_file(
    stability: str = "stable",
    kind: str = "sync",
    argument_fields: dict | None = None,
    required_arguments: list[str] | None = None,
    command_id: str = "sample.get",
) -> str:
    if argument_fields is None:
        argument_fields = {"sample_id": {"type": "string"}, "limit": {"type": "integer"}}
    if required_arguments is None:
        required_arguments = ["sample_id"]
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": command_id,
        "x-rastai-kind": kind,
        "x-rastai-stability": stability,
        "x-rastai-since-version": "0.1.0",
        "$defs": {
            "SampleSummary": {
                "type": "object",
                "properties": {"state": {"type": "string"}, "node_id": {"type": "string"}},
                "required": ["state"],
            },
            "arguments": {
                "title": "SampleGetArgs",
                "type": "object",
                "properties": argument_fields,
                "required": required_arguments,
            },
            "result": {
                "title": "SampleGetResult",
                "type": "object",
                "properties": {"sample": {"$ref": "#/$defs/SampleSummary", "description": "The sample."}},
                "required": ["sample"],
            },
        },
    }
    return json.dumps(schema, indent=2, sort_keys=True) + "\n"


def event_file(stability: str = "stable", event_class: str = "durable", fields: list[dict] | None = None) -> str:
    if fields is None:
        fields = [payload_field("sample_id", 1, "string"), payload_field("step", 2, "uint64")]
    content = {
        "event_type": "sample.changed",
        "event_class": event_class,
        "emitter": "daemon",
        "payload_type": "rastai.events.v1.SampleChanged",
        "x-rastai-stability": stability,
        "x-rastai-since-version": "0.1.0",
        "fields": fields,
    }
    return json.dumps(content, indent=2, sort_keys=True) + "\n"


def payload_field(name: str, number: int, type_name: str, repeated: bool = False) -> dict:
    return {"name": name, "number": number, "type": type_name, "repeated": repeated, "optional": False}


# ---- Repositories ----


def write_files(repository: Path, files: dict[str, str]) -> None:
    for relative_path, text in files.items():
        path = repository / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def new_repository(tmp_path: Path, base_files: dict[str, str]) -> Path:
    """A repository whose main holds base_files and contracts.md (BASE_CONTRACTS unless base_files has its own),
    checked out on a new feature branch."""
    repository = tmp_path / "repository"
    repository.mkdir()
    git(repository, "init", "--quiet", "--initial-branch=main")
    files = {CONTRACTS_FILE: BASE_CONTRACTS, **base_files}
    write_files(repository, files)
    git(repository, "add", "--all")
    git(repository, "commit", "--quiet", "-m", "Base")
    git(repository, "switch", "--quiet", "-c", "feature")
    return repository


def add_contracts_line(repository: Path, line: str) -> None:
    path = repository / CONTRACTS_FILE
    path.write_text(path.read_text() + line)


def run_schema_diff(repository: Path, base_ref: str = "main") -> subprocess.CompletedProcess:
    command = [sys.executable, str(CHECKS_FOLDER / CHECK), "--root", str(repository)]
    return subprocess.run(command, capture_output=True, text=True, env=isolated_environment(base_ref))


def changed_command_repository(tmp_path: Path, base_text: str, head_text: str, contracts_line: str = "") -> Path:
    repository = new_repository(tmp_path, {COMMAND_PATH: base_text})
    write_files(repository, {COMMAND_PATH: head_text})
    if contracts_line:
        add_contracts_line(repository, contracts_line)
    return repository


# The three field changes ADR-0038 calls breaking, as (name, head arguments, head required list).
BREAKING_FIELD_CHANGES = [
    ("removed field", {"sample_id": {"type": "string"}}, ["sample_id"]),
    ("changed type", {"sample_id": {"type": "string"}, "limit": {"type": "string"}}, ["sample_id"]),
    ("newly required field", {"sample_id": {"type": "string"}, "limit": {"type": "integer"}}, ["sample_id", "limit"]),
]
BREAKING_MESSAGES = {
    "removed field": "the field `arguments.limit` was removed or renamed",
    "changed type": 'the field `arguments.limit` changed type from "integer" to "string"',
    "newly required field": "the field `arguments.limit` is newly required",
}


# ---- Unchanged, and the contracts rule ----


def test_unchanged_schemas_pass(tmp_path):
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file(), EVENT_PATH: event_file()})
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout
    assert "every command and event schema change since" in result.stdout


def test_a_schema_change_without_a_contracts_change_fails(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert (
        f"check-schema-diff: {COMMAND_PATH} changed, but no changed line of {CONTRACTS_FILE} names `sample.get`, "
        "`SampleGetArgs`, `SampleGetResult`: update contracts.md in the same change (contracts 12.6)"
    ) in result.stdout


def test_the_same_change_passes_once_a_contracts_line_names_the_command(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


def test_a_contracts_line_naming_the_argument_type_counts(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    line = "| `SampleGetArgs` | `sample_id` only |\n"
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head, line)
    assert run_schema_diff(repository).returncode == 0


def test_a_contracts_line_naming_only_another_command_fails(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    line = "- `sample.get_all` and `sample.list` changed.\n"
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head, line)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "no changed line of" in result.stdout


def test_a_removed_contracts_line_naming_the_command_counts(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head)
    contracts = repository / CONTRACTS_FILE
    contracts.write_text(contracts.read_text().replace("| `sample.get` | get one sample |\n", ""))
    assert run_schema_diff(repository).returncode == 0


def test_an_added_file_needs_a_contracts_change_too(tmp_path):
    repository = new_repository(tmp_path, {})
    write_files(repository, {COMMAND_PATH: command_file(stability="preview")})
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert f"{COMMAND_PATH} was added, but no changed line" in result.stdout
    add_contracts_line(repository, COMMAND_CONTRACTS_LINE)
    assert run_schema_diff(repository).returncode == 0


def test_a_removed_contracts_line_that_starts_with_two_dashes_counts(tmp_path):
    # In the diff it reads `--- ...`, like a file header, but it comes after the first hunk.
    contracts = BASE_CONTRACTS + "-- `sample.get` takes a sample id.\n"
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file(stability="preview"), CONTRACTS_FILE: contracts})
    write_files(repository, {COMMAND_PATH: head, CONTRACTS_FILE: BASE_CONTRACTS})
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


def test_only_the_file_headers_before_the_first_hunk_are_skipped(load_check):
    check = load_check(CHECK)
    diff = (
        "diff --git a/docs/architecture/contracts.md b/docs/architecture/contracts.md\n"
        "--- a/docs/architecture/contracts.md\n"
        "+++ b/docs/architecture/contracts.md\n"
        "@@ -3 +3 @@ intro\n"
        "--- `job.get` was here\n"
        "+++ `job.get` is here\n"
        "@@ -9,0 +10 @@ more\n"
        "+- `job.list` too\n"
    )
    assert check.changed_lines(diff) == ["-- `job.get` was here", "++ `job.get` is here", "- `job.list` too"]


def test_committed_changes_are_compared_like_working_tree_ones(tmp_path):
    head = command_file(stability="preview", argument_fields={"sample_id": {"type": "string"}})
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head)
    git(repository, "commit", "--quiet", "--all", "-m", "Change the arguments")
    assert run_schema_diff(repository).returncode == 1
    add_contracts_line(repository, COMMAND_CONTRACTS_LINE)
    git(repository, "commit", "--quiet", "--all", "-m", "Register it")
    assert run_schema_diff(repository).returncode == 0


# ---- Breaking changes: stable and deprecated fail, preview passes ----


@pytest.mark.parametrize("change_name, argument_fields, required_arguments", BREAKING_FIELD_CHANGES)
def test_a_breaking_field_change_to_a_stable_command_fails(tmp_path, change_name, argument_fields, required_arguments):
    head = command_file(argument_fields=argument_fields, required_arguments=required_arguments)
    repository = changed_command_repository(tmp_path, command_file(), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    expected = (
        f"check-schema-diff: {COMMAND_PATH}: command `sample.get` is stable, so it can't break: "
        f"{BREAKING_MESSAGES[change_name]}"
    )
    assert expected in result.stdout


@pytest.mark.parametrize("change_name, argument_fields, required_arguments", BREAKING_FIELD_CHANGES)
def test_the_same_change_to_a_preview_command_passes_with_the_contracts_change(
    tmp_path, change_name, argument_fields, required_arguments
):
    head = command_file(stability="preview", argument_fields=argument_fields, required_arguments=required_arguments)
    base = command_file(stability="preview")
    repository = changed_command_repository(tmp_path, base, head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


@pytest.mark.parametrize("change_name, argument_fields, required_arguments", BREAKING_FIELD_CHANGES)
def test_the_same_change_to_a_deprecated_command_fails(tmp_path, change_name, argument_fields, required_arguments):
    head = command_file(stability="deprecated", argument_fields=argument_fields, required_arguments=required_arguments)
    base = command_file(stability="deprecated")
    repository = changed_command_repository(tmp_path, base, head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "is deprecated, so it can't break: " + BREAKING_MESSAGES[change_name] in result.stdout


def test_a_stable_protobuf_json_field_that_holds_another_message_fails(tmp_path):
    base = command_file(argument_fields={"sample_id": {"type": "string"}, "telemetry": TELEMETRY_FIELD})
    head = command_file(argument_fields={"sample_id": {"type": "string"}, "telemetry": DIAGNOSTIC_FIELD})
    repository = changed_command_repository(tmp_path, base, head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert (
        'the field `arguments.telemetry` changed type from "object" of protobuf message '
        'rastai.common.v1.NodeTelemetry to "object" of protobuf message rastai.common.v1.Diagnostic'
    ) in result.stdout


def test_a_new_doc_on_a_stable_protobuf_json_field_passes(tmp_path):
    reworded = {"type": "object", "description": "The last telemetry. Protobuf JSON of rastai.common.v1.NodeTelemetry."}
    base = command_file(argument_fields={"sample_id": {"type": "string"}, "telemetry": TELEMETRY_FIELD})
    head = command_file(argument_fields={"sample_id": {"type": "string"}, "telemetry": reworded})
    repository = changed_command_repository(tmp_path, base, head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


def test_a_field_removed_from_a_shared_type_of_a_stable_result_fails(tmp_path):
    base = command_file()
    schema = json.loads(base)
    del schema["$defs"]["SampleSummary"]["properties"]["node_id"]
    head = json.dumps(schema, indent=2, sort_keys=True) + "\n"
    repository = changed_command_repository(tmp_path, base, head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "the field `result.sample.node_id` was removed or renamed" in result.stdout


def test_a_new_optional_field_on_a_stable_command_passes(tmp_path):
    argument_fields = {
        "sample_id": {"type": "string"},
        "limit": {"type": "integer"},
        "filter": {"type": "object", "properties": {"tag": {"type": "string"}}, "required": ["tag"]},
    }
    head = command_file(argument_fields=argument_fields)
    repository = changed_command_repository(tmp_path, command_file(), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


def test_a_stable_command_whose_kind_changes_fails(tmp_path):
    head = command_file(kind="job")
    repository = changed_command_repository(tmp_path, command_file(), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "command `sample.get` is stable, so it can't break: its kind changed from `sync` to `job`" in result.stdout


def test_a_preview_command_whose_kind_changes_passes_with_the_contracts_change(tmp_path):
    head = command_file(stability="preview", kind="job")
    repository = changed_command_repository(tmp_path, command_file(stability="preview"), head, COMMAND_CONTRACTS_LINE)
    assert run_schema_diff(repository).returncode == 0


def test_a_change_from_stable_to_preview_fails(tmp_path):
    head = command_file(stability="preview")
    repository = changed_command_repository(tmp_path, command_file(), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "its stability moved from `stable` to `preview`" in result.stdout


def test_deprecating_a_stable_command_passes_with_the_contracts_change(tmp_path):
    head = command_file(stability="deprecated")
    repository = changed_command_repository(tmp_path, command_file(), head, COMMAND_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


def test_a_removed_or_renamed_stable_command_fails(tmp_path):
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file()})
    (repository / COMMAND_PATH).unlink()
    write_files(repository, {OTHER_COMMAND_PATH: command_file(command_id="sample.list")})
    add_contracts_line(repository, "- `sample.get` is now `sample.list`.\n")
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert (
        f"{COMMAND_PATH}: command `sample.get` is stable, so it can't break: the file was removed "
        "(a removed or renamed command or event type)"
    ) in result.stdout


def test_a_removed_preview_command_passes_with_the_contracts_change(tmp_path):
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file(stability="preview")})
    (repository / COMMAND_PATH).unlink()
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert f"{COMMAND_PATH} was removed, but no changed line" in result.stdout
    add_contracts_line(repository, "- `sample.get` is gone.\n")
    assert run_schema_diff(repository).returncode == 0


# ---- Events ----


def changed_event_repository(tmp_path: Path, base_text: str, head_text: str) -> Path:
    repository = new_repository(tmp_path, {EVENT_PATH: base_text})
    write_files(repository, {EVENT_PATH: head_text})
    add_contracts_line(repository, EVENT_CONTRACTS_LINE)
    return repository


def test_a_stable_event_that_loses_a_payload_field_fails(tmp_path):
    head = event_file(fields=[payload_field("sample_id", 1, "string")])
    repository = changed_event_repository(tmp_path, event_file(), head)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert (
        f"{EVENT_PATH}: event type `sample.changed` is stable, so it can't break: "
        "the payload field `step` was removed or renamed"
    ) in result.stdout


def test_a_preview_event_that_loses_a_payload_field_passes_with_the_contracts_change(tmp_path):
    head = event_file(stability="preview", fields=[payload_field("sample_id", 1, "string")])
    repository = changed_event_repository(tmp_path, event_file(stability="preview"), head)
    assert run_schema_diff(repository).returncode == 0


def test_a_stable_event_whose_field_type_or_class_changes_fails(tmp_path):
    fields = [payload_field("sample_id", 1, "string"), payload_field("step", 2, "uint64", repeated=True)]
    head = event_file(event_class="ephemeral", fields=fields)
    repository = changed_event_repository(tmp_path, event_file(), head)
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert "its event_class changed from `durable` to `ephemeral`" in result.stdout
    assert (
        'the payload field `step` changed from number 2, type "uint64", repeated false, optional false '
        'to number 2, type "uint64", repeated true, optional false'
    ) in result.stdout


def test_a_new_payload_field_on_a_stable_event_passes_with_the_contracts_change(tmp_path):
    fields = [
        payload_field("sample_id", 1, "string"),
        payload_field("step", 2, "uint64"),
        payload_field("note", 3, "string"),
    ]
    repository = changed_event_repository(tmp_path, event_file(), event_file(fields=fields))
    assert run_schema_diff(repository).returncode == 0


def test_an_event_change_registered_by_its_payload_message_passes(tmp_path):
    head = event_file(stability="preview", fields=[payload_field("sample_id", 1, "string")])
    repository = new_repository(tmp_path, {EVENT_PATH: event_file(stability="preview")})
    write_files(repository, {EVENT_PATH: head})
    add_contracts_line(repository, "| `SampleChanged` | `sample_id` |\n")
    assert run_schema_diff(repository).returncode == 0


# ---- The merge base ----


def test_a_missing_merge_base_fails_with_a_clear_message(tmp_path):
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file()})
    result = run_schema_diff(repository, base_ref="origin/no-such-branch")
    assert result.returncode == 1
    assert "can't find the merge base of HEAD and origin/no-such-branch: fetch the base branch" in result.stdout


def test_a_base_without_schemas_events_reports_the_event_files_as_added(tmp_path):
    repository = new_repository(tmp_path, {COMMAND_PATH: command_file()})
    write_files(repository, {EVENT_PATH: event_file()})
    result = run_schema_diff(repository)
    assert result.returncode == 1
    assert f"{EVENT_PATH} was added, but no changed line of {CONTRACTS_FILE} names `sample.changed`" in result.stdout
    add_contracts_line(repository, EVENT_CONTRACTS_LINE)
    result = run_schema_diff(repository)
    assert result.returncode == 0, result.stdout


# ---- How names are matched ----


def test_a_name_counts_only_as_a_whole(load_check):
    check = load_check(CHECK)
    pattern = check.name_pattern("job.get")
    assert pattern.search("| `job.get` | one job |")
    assert pattern.search("see job.get.")
    assert not pattern.search("`job.get_all`")
    assert not pattern.search("`xjob.get`")
    assert not pattern.search("`job.get.more`")


def test_a_type_that_contains_itself_is_entered_once(load_check):
    check = load_check(CHECK)
    content = {
        "$defs": {
            "Node": {"type": "object", "properties": {"child": {"$ref": "#/$defs/Node"}}},
            "arguments": {"type": "object", "properties": {"root": {"$ref": "#/$defs/Node"}}, "required": ["root"]},
            "result": {"type": "object", "properties": {}},
        }
    }
    fields = check.command_fields(content)
    assert sorted(fields) == ["arguments", "arguments.root", "arguments.root.child", "result"]
    assert fields["arguments.root.child"].type_text == "a Node"


# ---- The checked-in schemas ----


def test_every_checked_in_command_file_carries_its_kind_and_stability(load_check):
    check = load_check(CHECK)
    command_files = sorted((REPO_ROOT / "schemas/commands").rglob("*.json"))
    assert command_files
    for path in command_files:
        content = json.loads(path.read_text())
        assert content["x-rastai-kind"] in ("sync", "job", "stream"), path
        assert content["x-rastai-stability"] in ("preview", "stable", "deprecated"), path
        assert content["x-rastai-since-version"], path
        relative = path.relative_to(REPO_ROOT).as_posix()
        assert check.parse_item(relative, path.read_text()) is not None, path


def test_the_checked_in_protobuf_json_fields_name_their_message(load_check):
    # Ties PROTO_MESSAGE_PATTERN to the sentence the real codegen writes.
    check = load_check(CHECK)
    path = REPO_ROOT / "schemas/commands/rastai.command.node.status/1.json"
    fields = check.command_fields(json.loads(path.read_text()))
    type_texts = [field.type_text for field in fields.values()]
    assert '"object" of protobuf message rastai.common.v1.NodeTelemetry' in type_texts


def test_schemas_events_holds_one_file_per_m1_event_type(load_check):
    check = load_check(CHECK)
    events_folder = REPO_ROOT / "schemas/events"
    file_names = sorted(path.name for path in events_folder.glob("*.json"))
    assert file_names == sorted(f"{event_type}.json" for event_type in M1_EVENT_TYPES)
    for event_type in M1_EVENT_TYPES:
        path = events_folder / f"{event_type}.json"
        content = json.loads(path.read_text())
        assert content["event_type"] == event_type
        assert content["event_class"] in ("durable", "ephemeral")
        assert content["x-rastai-stability"] == "preview"
        assert content["fields"], event_type
        item = check.parse_item(f"schemas/events/{event_type}.json", path.read_text())
        assert item.label == f"event type `{event_type}`"
