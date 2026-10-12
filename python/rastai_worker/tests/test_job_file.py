"""job.json and the command line (job_file.py, arguments.py; python layout 7.3): a good file reads into a
WorkerLaunch, and every bad file or command line is a bad launch, RRS-J2001, which the process exits 2 on.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_J2001
from rastai_proto.node.v1.jobs_pb2 import JOB_KIND_TRAIN
from rastai_worker.arguments import MODE_PLUGIN, MODE_RUN, parse_arguments
from rastai_worker.errors import WorkerError
from rastai_worker.exit_codes import EXIT_BAD_LAUNCH
from rastai_worker.job_file import read_job_file

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
# What the node writes (rastai-node jobs/job_file.rs), with the job_spec in protobuf JSON (rastai-core proto_json.rs).
NODE_JOB_FILE = {
    "job_id": JOB_ID,
    "run_id": RUN_ID,
    "kind": "train",
    "state": "starting",
    "started_unix_ms": 1_791_700_000_123,
    "job_spec": {
        "job_id": JOB_ID,
        "run_id": RUN_ID,
        "kind": "JOB_KIND_TRAIN",
        "environment_id": "cpu",
        "arguments_json": '{"recipe": "classification"}',
        "resources": {"gpu_indices": [0]},
        "a_field_from_a_newer_node": 1,
    },
    "job_socket_path": "/run/rastai/jobs/job_a.sock",
    "worker_host_socket_path": "/run/rastai/worker-host.sock",
    "run_folder": "/data/runs/run_a",
    "environment_id": "cpu",
    "environment_lock_hash": "",
    "hardware_profile": {"cpu_count": 8},
    "granted_leases": [],
    "resolved_models": {},
    "plugins": [],
    "resume_from": "",
    "pid": 4242,
    "process_group_id": 4242,
    "process_start_ticks": 987654,
    "boot_id": "boot",
    "job_token_hash": "blake3:" + "00" * 32,
    "final_state": "",
}


def write_job_file(folder: Path, fields: object) -> Path:
    job_file = folder / "job.json"
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return job_file


def changed(**changes: object) -> dict:
    fields = dict(NODE_JOB_FILE)
    fields.update(changes)
    return fields


def without(name: str) -> dict:
    fields = dict(NODE_JOB_FILE)
    del fields[name]
    return fields


def test_the_nodes_file_reads_into_a_launch(tmp_path: Path) -> None:
    launch = read_job_file(write_job_file(tmp_path, NODE_JOB_FILE))
    assert launch.job_id == JOB_ID
    assert launch.run_id == RUN_ID
    assert launch.job_spec.kind == JOB_KIND_TRAIN
    assert launch.job_spec.arguments_json == '{"recipe": "classification"}'
    assert list(launch.job_spec.resources.gpu_indices) == [0]
    assert launch.job_socket_path == "/run/rastai/jobs/job_a.sock"
    assert launch.worker_host_socket_path == "/run/rastai/worker-host.sock"
    assert launch.event_log_path == Path("/data/runs/run_a/events.log")
    assert launch.environment_id == "cpu"
    assert launch.hardware_profile == {"cpu_count": 8}
    assert launch.cache_folder == ""


def test_fields_the_node_fills_later_may_be_missing(tmp_path: Path) -> None:
    fields = dict(NODE_JOB_FILE)
    for name in ("cache_folder", "environment_lock_hash", "granted_leases", "resolved_models", "plugins"):
        fields.pop(name, None)
    launch = read_job_file(write_job_file(tmp_path, fields))
    assert launch.granted_leases == []
    assert launch.resolved_models == {}
    assert launch.plugins == []


@pytest.mark.parametrize(
    ("fields", "reason"),
    [
        ([1, 2], "isn't a JSON object"),
        (without("job_socket_path"), "has no job_socket_path"),
        (without("run_folder"), "has no run_folder"),
        (changed(job_id=""), "has no job_id"),
        (changed(resume_from=7), "resume_from isn't text"),
        (changed(plugins={}), "plugins isn't a list"),
        (changed(job_spec="train"), "job_spec isn't an object"),
        (changed(job_spec={"job_id": JOB_ID, "arguments_json": 5}), "isn't a JobSpec"),
        (changed(job_spec={"job_id": "job_other", "run_id": RUN_ID}), "is for job job_other"),
    ],
)
def test_a_bad_file_is_a_bad_launch(tmp_path: Path, fields: object, reason: str) -> None:
    with pytest.raises(WorkerError) as raised:
        read_job_file(write_job_file(tmp_path, fields))
    assert raised.value.code == RRS_J2001
    assert reason in raised.value.message


def test_a_missing_or_unparsable_file_is_a_bad_launch(tmp_path: Path) -> None:
    with pytest.raises(WorkerError) as raised:
        read_job_file(tmp_path / "job.json")
    assert raised.value.code == RRS_J2001
    assert "can't be read" in raised.value.message
    (tmp_path / "job.json").write_text("{not json", encoding="utf-8")
    with pytest.raises(WorkerError) as raised:
        read_job_file(tmp_path / "job.json")
    assert "isn't JSON" in raised.value.message


def test_the_two_command_lines_parse() -> None:
    run = parse_arguments(["run", "--job-file", "/data/runs/run_a/job.json"])
    assert run.mode == MODE_RUN
    assert run.job_file == Path("/data/runs/run_a/job.json")
    plugin = parse_arguments(["plugin", "--plugin", "word_count", "--job-file", "/data/job.json"])
    assert plugin.mode == MODE_PLUGIN
    assert plugin.plugin_id == "word_count"


@pytest.mark.parametrize(
    "argv",
    [
        [],
        ["serve"],
        ["run"],
        ["run", "--job-file"],
        ["run", "--plugin", "word_count", "--job-file", "job.json"],
        ["run", "--job-file", "a.json", "--job-file", "b.json"],
        ["plugin", "--job-file", "job.json"],
    ],
)
def test_a_wrong_command_line_is_a_bad_launch(argv: list[str]) -> None:
    with pytest.raises(WorkerError) as raised:
        parse_arguments(argv)
    assert raised.value.code == RRS_J2001


@pytest.mark.posix_only
def test_the_process_exits_2_on_an_unreadable_job_file(tmp_path: Path) -> None:
    """The acceptance case, as the node runs it: `python -m rastai_worker run --job-file` with stderr in a file. The
    last line of stderr.log is the reason the node shows (RRS-J2001), and nothing is written to the run folder."""
    job_file = tmp_path / "job.json"
    job_file.write_text("{damaged", encoding="utf-8")
    stderr_path = tmp_path / "stderr.log"
    environment = dict(os.environ, RASTAI_JOB_TOKEN="ab" * 32)
    with open(stderr_path, "w", encoding="utf-8") as stderr_file:
        finished = subprocess.run(
            [sys.executable, "-m", "rastai_worker", "run", "--job-file", str(job_file)],
            stdin=subprocess.DEVNULL,
            stderr=stderr_file,
            env=environment,
            timeout=60,
            check=False,
        )
    assert finished.returncode == EXIT_BAD_LAUNCH
    last_line = stderr_path.read_text(encoding="utf-8").strip().splitlines()[-1]
    assert last_line.startswith(f"{RRS_J2001}: ")
    assert "isn't JSON" in last_line
    assert sorted(path.name for path in tmp_path.iterdir()) == ["job.json", "stderr.log"]
