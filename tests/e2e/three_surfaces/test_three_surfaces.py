"""Three surfaces end to end (plan M1-49, M1 exit criterion 3; flows/platform.md 3.1 and 4.3): `system.status`,
`train.start` and `job.get` run the same way over Connect JSON on HTTP, the `rastai` verbs and `rastai mcp`. Same
command and arguments, same outcome, same diagnostics; a token without the scope is refused the same way; an
idempotency key is honoured the same way.

The session runs one daemon. `three_surfaces_run` drives everything once, in a fixed order, so the MCP session is
the same every time and compares with its golden transcript (fixtures/mcp/e2e/); each test then checks one promise
against what it recorded.

What differs between the surfaces by design, and is not compared:
- The principal. The CLI is always the owner (daemon.sock, peer credentials; it never sends a token, layout 9.7),
  so HTTP uses the owner token beside it; MCP always runs as a scoped token (`train`: `read` and `training`).
  For the same reason the scoped-token refusals run on HTTP and MCP only: the CLI has no token to refuse, which is
  how plan M1-49's Test line words its chat-only criterion.
- A job command's answer: the job id on all three, and the result (`run_id`, the plan) only over HTTP; the CLI and
  MCP leave the run to `job.get`, which shows it the same on all three.
- An MCP tools/call carries no idempotency key (none is defined for MCP), so keys are checked on HTTP and the CLI.
- The rendering of a diagnostic: MCP shows the code, message and fix as text, HTTP and the CLI as fields.
- Arguments that don't match the command's schema: the CLI refuses them before sending, worded in flags
  (layout 9.3 step 4), so only the code and the exit code are compared with it."""

import json
import sqlite3
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import pytest

from daemon_process import RunningDaemon
from surfaces import CliSurface, HttpSurface, McpSurface, Outcome, common_diagnostics
from transcripts import as_jsonl, read_golden, recorded_lines

DESIGN = "templates/designs/textcnn.rrsmodel"
DATASET = "fixtures/datasets/textcnn_tiny.jsonl"
# A path under the repo to no file, made absolute by paths_arguments: train.start takes it (the node checks the
# path), and the job fails reading it.
MISSING_DATASET = "fixtures/datasets/no_such_dataset.jsonl"
UNKNOWN_RECIPE = "no_such_recipe"
# A well-formed job id no daemon ever made.
UNKNOWN_JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q"
IDEMPOTENCY_KEY = "three-surfaces-train-start"
GOLDEN_FOLDER = "fixtures/mcp/e2e"
TRAIN_GOLDEN = "three_surfaces.jsonl"
CHAT_ONLY_GOLDEN = "chat_only_token.jsonl"
# A TextCNN run takes about 20 s on a quiet machine and over 200 s on a loaded one; three run at once here.
JOB_END_LIMIT_SECONDS = 1200
JOB_POLL_SECONDS = 1.0
ENDED_JOB_STATES = ("succeeded", "failed", "cancelled")
# The volatile node field system.status shows; every surface reads it at a different moment.
HEARTBEAT_FIELD = "last_heartbeat_unix_ms"
CODE_UNKNOWN_JOB = "RRS-J0005"
CODE_RECIPE_NOT_INSTALLED = "RRS-T0002"
CODE_INVALID_ARGUMENTS = "RRS-C0002"
CODE_IDEMPOTENCY_CONFLICT = "RRS-C0004"
CODE_SCOPE_NOT_GRANTED = "RRS-C1005"
CODE_DATASET_MISSING = "RRS-D3001"
# The CLI's exit codes (layout 9.5): a refusal of auth or scope, a usage error, any other failure.
EXIT_OK = 0
EXIT_FAILED = 1
EXIT_USAGE = 2
EXIT_NOT_PERMITTED = 4
USAGE_CODES = ("RRS-C0001", CODE_INVALID_ARGUMENTS)
NOT_PERMITTED_PREFIX = "RRS-C1"
OWNER_PRINCIPAL = "owner"
TOKEN_PRINCIPAL = "token:{token_id}"
RASTAI_DB = "data/rastai.db"
# The tokens refused train.start: the chat-only preset of the acceptance test, and a read token.
REFUSED_TOKENS = ("chat-only", "read")

# The daemon reads /proc and signals process groups, and the node's sockets are Unix ones. pytest reads
# `pytestmark` only in a test module (never in conftest.py), so each test module carries it.
pytestmark = pytest.mark.skipif(sys.platform != "linux", reason="the daemon's job tests need Linux")


@dataclass
class ThreeSurfacesRun:
    """What each surface answered, by surface name ("http", "cli", "mcp")."""

    status: dict
    started: dict
    # The job each surface started, and the job started to fail, by the same names plus "failing".
    job_ids: dict
    failing_start: Outcome
    unknown_job: dict
    unknown_recipe: dict
    missing_argument: dict
    # job.get of each job above, by job name, then by surface.
    job_reads: dict
    http_replay: Outcome
    cli_replay: Outcome
    http_conflict: Outcome
    cli_conflict: Outcome
    train_jobs_listed: list


def paths_arguments(repo_root: Path, recipe: str, dataset: str) -> dict:
    return {
        "recipe": recipe,
        "design_path": str(repo_root / DESIGN),
        "dataset_path": str(repo_root / dataset),
    }


def on_every_surface(surfaces: list, command_id: str, arguments: dict) -> dict:
    return {surface.name: surface.execute(command_id, arguments) for surface in surfaces}


def wait_until_ended(http: HttpSurface, job_ids: list[str]) -> None:
    """A bounded poll; a job still running at the limit fails the session (and the cleanup kills its worker)."""
    deadline = time.monotonic() + JOB_END_LIMIT_SECONDS
    waiting = list(job_ids)
    while waiting:
        job_id = waiting[0]
        outcome = http.execute("job.get", {"job_id": job_id})
        assert outcome.ok, outcome
        if outcome.result["job"]["state"] in ENDED_JOB_STATES:
            waiting.pop(0)
            continue
        assert time.monotonic() < deadline, f"{job_id} didn't end within {JOB_END_LIMIT_SECONDS} s: {outcome}"
        time.sleep(JOB_POLL_SECONDS)


@dataclass
class JobStarts:
    """The jobs train.start started, and what a retry with an idempotency key answered."""

    started: dict
    job_ids: dict
    failing_start: Outcome
    http_replay: Outcome
    cli_replay: Outcome
    http_conflict: Outcome
    cli_conflict: Outcome


def start_jobs(owner_http: HttpSurface, cli: CliSurface, train_mcp: McpSurface, repo_root: Path) -> JobStarts:
    """One train.start per surface, one that fails later, then a retry soon after the first call: the same key and
    arguments, then the same key with other arguments."""
    train_arguments = paths_arguments(repo_root, "classification", DATASET)
    missing_dataset_arguments = paths_arguments(repo_root, "classification", MISSING_DATASET)
    started = {
        "http": owner_http.execute("train.start", train_arguments, IDEMPOTENCY_KEY),
        "cli": cli.execute("train.start", train_arguments),
        "mcp": train_mcp.execute("train.start", train_arguments),
    }
    failing_start = owner_http.execute("train.start", missing_dataset_arguments)
    job_ids = {name: outcome.job_id for name, outcome in started.items()}
    job_ids["failing"] = failing_start.job_id
    http_replay = owner_http.execute("train.start", train_arguments, IDEMPOTENCY_KEY)
    cli_replay = cli.execute("train.start", train_arguments, IDEMPOTENCY_KEY)
    http_conflict = owner_http.execute("train.start", missing_dataset_arguments, IDEMPOTENCY_KEY)
    cli_conflict = cli.execute("train.start", missing_dataset_arguments, IDEMPOTENCY_KEY)
    return JobStarts(
        started=started,
        job_ids=job_ids,
        failing_start=failing_start,
        http_replay=http_replay,
        cli_replay=cli_replay,
        http_conflict=http_conflict,
        cli_conflict=cli_conflict,
    )


def train_jobs_listed(owner_http: HttpSurface) -> list:
    """job.list lists every kind; only train jobs are this test's."""
    listed = owner_http.execute("job.list", {})
    assert listed.ok, listed
    return [job for job in listed.result["jobs"] if job["kind"] == "train"]


@pytest.fixture(scope="session")
def three_surfaces_run(
    repo_root: Path, owner_http: HttpSurface, cli: CliSurface, train_mcp: McpSurface
) -> ThreeSurfacesRun:
    """Everything runs once, in this order, so the MCP session matches its golden transcript."""
    surfaces = [owner_http, cli, train_mcp]
    status = on_every_surface(surfaces, "system.status", {})
    starts = start_jobs(owner_http, cli, train_mcp, repo_root)
    unknown_job = on_every_surface(surfaces, "job.get", {"job_id": UNKNOWN_JOB_ID})
    unknown_recipe = on_every_surface(surfaces, "train.start", paths_arguments(repo_root, UNKNOWN_RECIPE, DATASET))
    without_dataset = paths_arguments(repo_root, "classification", DATASET)
    del without_dataset["dataset_path"]
    missing_argument = on_every_surface(surfaces, "train.start", without_dataset)
    job_ids = starts.job_ids
    wait_until_ended(owner_http, [job_id for job_id in job_ids.values() if job_id])
    job_reads = {name: on_every_surface(surfaces, "job.get", {"job_id": job_id}) for name, job_id in job_ids.items()}
    return ThreeSurfacesRun(
        status=status,
        started=starts.started,
        job_ids=job_ids,
        failing_start=starts.failing_start,
        unknown_job=unknown_job,
        unknown_recipe=unknown_recipe,
        missing_argument=missing_argument,
        job_reads=job_reads,
        http_replay=starts.http_replay,
        cli_replay=starts.cli_replay,
        http_conflict=starts.http_conflict,
        cli_conflict=starts.cli_conflict,
        train_jobs_listed=train_jobs_listed(owner_http),
    )


@dataclass
class TokenRefusals:
    """A refused token's answers, by command id and then surface name, and its MCP tools/list."""

    by_command: dict
    listed_tools: list


@pytest.fixture(scope="session")
def refusals(
    repo_root: Path,
    daemon: RunningDaemon,
    tokens: dict,
    chat_only_mcp: McpSurface,
    read_mcp: McpSurface,
    three_surfaces_run: ThreeSurfacesRun,
) -> dict:
    """train.start, system.status and job.get with each refused token, over HTTP and MCP. After the main run, so
    the job count it recorded shows that no refusal started a job."""
    mcp_by_token = {"chat-only": chat_only_mcp, "read": read_mcp}
    train_arguments = paths_arguments(repo_root, "classification", DATASET)
    commands = [
        ("system.status", {}),
        ("train.start", train_arguments),
        ("job.get", {"job_id": three_surfaces_run.job_ids["mcp"]}),
    ]
    refused = {}
    for token_name in REFUSED_TOKENS:
        http = HttpSurface(daemon.port, tokens[token_name]["token"])
        mcp = mcp_by_token[token_name]
        listed_tools = mcp.list_tools()
        by_command = {
            command_id: on_every_surface([http, mcp], command_id, arguments) for command_id, arguments in commands
        }
        refused[token_name] = TokenRefusals(by_command=by_command, listed_tools=listed_tools)
    return refused


def expected_exit_code(outcome: Outcome) -> int:
    """Layout 9.5: the first error decides."""
    if outcome.ok:
        return EXIT_OK
    first_code = outcome.diagnostics[0]["code"]
    if first_code.startswith(NOT_PERMITTED_PREFIX):
        return EXIT_NOT_PERMITTED
    if first_code in USAGE_CODES:
        return EXIT_USAGE
    return EXIT_FAILED


def without_heartbeats(status: dict) -> dict:
    nodes = [{key: value for key, value in node.items() if key != HEARTBEAT_FIELD} for node in status["nodes"]]
    return {**status, "nodes": nodes}


def assert_same_refusal(outcomes: dict) -> None:
    """Every surface refused, with the same diagnostics; HTTP and the CLI also agree on the fix command and path,
    and the CLI's exit code is the one its first diagnostic gives."""
    for outcome in outcomes.values():
        assert not outcome.ok, outcome
        assert outcome.job_id == "", outcome
        assert outcome.result is None, outcome
    first = next(iter(outcomes.values()))
    for outcome in outcomes.values():
        assert common_diagnostics(outcome) == common_diagnostics(first), (first, outcome)
    if "http" in outcomes and "cli" in outcomes:
        assert outcomes["cli"].diagnostics == outcomes["http"].diagnostics
        assert outcomes["cli"].exit_code == expected_exit_code(outcomes["cli"])


def test_system_status_is_the_same_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    outcomes = three_surfaces_run.status
    for outcome in outcomes.values():
        assert outcome.ok and outcome.diagnostics == [], outcome
    assert outcomes["cli"].exit_code == EXIT_OK
    expected = without_heartbeats(outcomes["http"].result)
    assert [node["transport"] for node in expected["nodes"]] == ["local"]
    for outcome in outcomes.values():
        assert without_heartbeats(outcome.result) == expected, outcome.surface


def test_train_start_starts_one_job_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    for outcome in three_surfaces_run.started.values():
        assert outcome.ok and outcome.diagnostics == [], outcome
        assert outcome.job_id.startswith("job_"), outcome
    assert three_surfaces_run.started["cli"].exit_code == EXIT_OK
    job_ids = [outcome.job_id for outcome in three_surfaces_run.started.values()]
    assert len(set(job_ids)) == len(job_ids), "each surface's train.start is a job of its own"
    http_result = three_surfaces_run.started["http"].result
    assert http_result["run_id"].startswith("run_") and http_result["plan"]["recipe"] == "classification"


def test_job_get_reads_each_job_the_same_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    node_id = three_surfaces_run.status["http"].result["nodes"][0]["node_id"]
    for job_name, outcomes in three_surfaces_run.job_reads.items():
        for outcome in outcomes.values():
            assert outcome.ok and outcome.diagnostics == [], (job_name, outcome)
        expected = outcomes["http"].result
        for outcome in outcomes.values():
            assert outcome.result == expected, (job_name, outcome.surface)
        job = expected["job"]
        assert job["job_id"] == three_surfaces_run.job_ids[job_name]
        assert (job["kind"], job["node_id"]) == ("train", node_id)
        assert job["run_id"].startswith("run_")
    started_run_id = three_surfaces_run.started["http"].result["run_id"]
    assert three_surfaces_run.job_reads["http"]["http"].result["job"]["run_id"] == started_run_id


def test_every_surfaces_job_trains_to_the_end(three_surfaces_run: ThreeSurfacesRun):
    for job_name in ("http", "cli", "mcp"):
        job = three_surfaces_run.job_reads[job_name]["http"].result["job"]
        assert (job["state"], job.get("error_code", "")) == ("succeeded", ""), (job_name, job)


def test_a_failed_job_shows_the_same_code_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    assert three_surfaces_run.failing_start.ok, three_surfaces_run.failing_start
    job = three_surfaces_run.job_reads["failing"]["http"].result["job"]
    assert (job["state"], job["error_code"]) == ("failed", CODE_DATASET_MISSING), job


def test_an_unknown_job_is_refused_the_same_way_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    outcomes = three_surfaces_run.unknown_job
    assert_same_refusal(outcomes)
    assert [item["code"] for item in outcomes["http"].diagnostics] == [CODE_UNKNOWN_JOB]
    assert outcomes["http"].diagnostics[0]["fix_command_id"] == "job.list"


def test_bad_train_arguments_are_refused_the_same_way_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    outcomes = three_surfaces_run.unknown_recipe
    assert_same_refusal(outcomes)
    assert [item["code"] for item in outcomes["http"].diagnostics] == [CODE_RECIPE_NOT_INSTALLED]
    assert UNKNOWN_RECIPE in outcomes["http"].diagnostics[0]["message"], outcomes["http"]


def test_a_missing_argument_is_refused_as_invalid_arguments_on_every_surface(three_surfaces_run: ThreeSurfacesRun):
    """The CLI checks a verb's flags against the command's schema before it sends anything, and words its refusal
    in flags (layout 9.3 step 4), so only the code and the exit code are compared with it; HTTP and MCP carry the
    daemon's own refusal, with each problem's path."""
    outcomes = three_surfaces_run.missing_argument
    for outcome in outcomes.values():
        assert not outcome.ok and outcome.job_id == "", outcome
        assert outcome.diagnostics[0]["code"] == CODE_INVALID_ARGUMENTS, outcome
    assert outcomes["cli"].exit_code == EXIT_USAGE
    assert "--dataset-path" in outcomes["cli"].diagnostics[0]["message"], outcomes["cli"]
    assert common_diagnostics(outcomes["mcp"]) == common_diagnostics(outcomes["http"])
    assert [item["path"] for item in outcomes["http"].diagnostics] == ["#/dataset_path", "#/dataset_path"]


def test_the_same_command_leaves_the_same_journal_row_on_every_surface(
    three_surfaces_run: ThreeSurfacesRun, daemon: RunningDaemon, tokens: dict, repo_root: Path
):
    """Each surface's train.start reached the daemon as the same command with the same arguments; only the
    principal differs, as the top says."""
    expected_principals = {
        "http": OWNER_PRINCIPAL,
        "cli": OWNER_PRINCIPAL,
        "mcp": TOKEN_PRINCIPAL.format(token_id=tokens["train"]["token_id"]),
    }
    train_arguments = paths_arguments(repo_root, "classification", DATASET)
    for surface_name, principal in expected_principals.items():
        job_id = three_surfaces_run.job_ids[surface_name]
        rows = journal_rows_of_job(daemon.home, job_id)
        assert len(rows) == 1, (surface_name, rows)
        command_id, row_principal, arguments_json, ok, code = rows[0]
        assert (command_id, row_principal, ok, code) == ("train.start", principal, 1, None), surface_name
        assert json.loads(arguments_json) == train_arguments, surface_name


def journal_rows_of_job(home: Path, job_id: str) -> list[tuple]:
    """command_journal is contracts 8.2's table. Opened read-only while the daemon runs; it writes in WAL mode."""
    connection = sqlite3.connect(f"file:{home / RASTAI_DB}?mode=ro", uri=True)
    try:
        query = "SELECT command_id, principal, arguments_json, ok, code FROM command_journal WHERE job_id = ?"
        return connection.execute(query, (job_id,)).fetchall()
    finally:
        connection.close()


def test_an_idempotency_key_is_honoured_the_same_over_http_and_the_cli(three_surfaces_run: ThreeSurfacesRun):
    """The CLI and HTTP with the owner token are one principal, so a key one of them used replays on the other."""
    first = three_surfaces_run.started["http"]
    replays = [three_surfaces_run.http_replay, three_surfaces_run.cli_replay]
    for replay in replays:
        assert replay.ok and replay.job_id == first.job_id, replay
    assert three_surfaces_run.http_replay.result == first.result
    assert three_surfaces_run.cli_replay.exit_code == EXIT_OK
    conflicts = {"http": three_surfaces_run.http_conflict, "cli": three_surfaces_run.cli_conflict}
    assert_same_refusal(conflicts)
    assert [item["code"] for item in conflicts["http"].diagnostics] == [CODE_IDEMPOTENCY_CONFLICT]
    listed_ids = sorted(job["job_id"] for job in three_surfaces_run.train_jobs_listed)
    assert listed_ids == sorted(three_surfaces_run.job_ids.values()), "a replay or a refusal started a job"


@pytest.mark.parametrize("token_name", REFUSED_TOKENS)
def test_a_token_without_the_scope_is_refused_train_start_the_same_way(
    refusals: dict, tokens: dict, token_name: str, owner_http: HttpSurface, three_surfaces_run: ThreeSurfacesRun
):
    outcomes = refusals[token_name].by_command["train.start"]
    assert_same_refusal(outcomes)
    token_id = tokens[token_name]["token_id"]
    expected_message = f"train.start needs the scope training, which token:{token_id} wasn't granted."
    assert common_diagnostics(outcomes["http"]) == [
        {
            "code": CODE_SCOPE_NOT_GRANTED,
            "message": expected_message,
            "fix_text": "Use a token that has the scope shown.",
        }
    ]
    train_job_ids_now = sorted(job["job_id"] for job in train_jobs_listed(owner_http))
    train_job_ids_before = sorted(job["job_id"] for job in three_surfaces_run.train_jobs_listed)
    assert train_job_ids_now == train_job_ids_before, "a refused train.start started a job"


def test_a_chat_only_token_reads_nothing_and_a_read_token_reads_the_same_as_the_owner(
    refusals: dict, three_surfaces_run: ThreeSurfacesRun
):
    chat_only = refusals["chat-only"]
    assert chat_only.listed_tools == []
    for command_id in ("system.status", "job.get"):
        outcomes = chat_only.by_command[command_id]
        assert_same_refusal(outcomes)
        assert outcomes["http"].diagnostics[0]["code"] == CODE_SCOPE_NOT_GRANTED
    read_job = refusals["read"].by_command["job.get"]
    for outcome in read_job.values():
        assert outcome.result == three_surfaces_run.job_reads["mcp"]["http"].result, outcome.surface


def assert_matches_golden(mcp: McpSurface, golden_name: str, token: dict, repo_root: Path) -> None:
    session, golden_exchanges = read_golden(repo_root / GOLDEN_FOLDER / golden_name)
    assert session["token_scopes"] == token["scopes"]
    recorded = recorded_lines(mcp.exchanges, repo_root)
    message = f"the MCP session differs from {golden_name}; this run recorded:\n{as_jsonl(recorded)}"
    assert recorded == golden_exchanges, message


def test_the_train_tokens_mcp_session_matches_its_golden_transcript(
    three_surfaces_run: ThreeSurfacesRun, train_mcp: McpSurface, tokens: dict, repo_root: Path
):
    assert_matches_golden(train_mcp, TRAIN_GOLDEN, tokens["train"], repo_root)


def test_the_chat_only_tokens_mcp_session_matches_its_golden_transcript(
    refusals: dict, chat_only_mcp: McpSurface, tokens: dict, repo_root: Path
):
    assert_matches_golden(chat_only_mcp, CHAT_ONLY_GOLDEN, tokens["chat-only"], repo_root)
