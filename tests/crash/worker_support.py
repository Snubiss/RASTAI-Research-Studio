"""What the worker's kill -9 tests share (flows/platform.md 6.5; python layout 7.9): job files for the TextCNN train
fixture, the worker process as the node starts it, a stand-in node as a process of its own (so a test can kill it
with SIGKILL), /proc reading, and the run log read frame by frame.

Every process a test starts goes in the list of conftest.py's `started` fixture, which kills each one's whole
process group on every exit path, a failed assertion included (issue #244).
"""

import json
import os
import signal
import struct
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from rastai_formats import hash_bytes
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_worker.event_log import EVENT_LOG_HEADER, FRAME_HEADER_BYTES

# The train fixture's launch file (contracts 12.5): its paths are relative to the repository root.
TRAIN_JOB_FIXTURE = Path("jobs/textcnn_train/job.json")
START_PATH_FIELDS = ("design_path", "dataset_path")
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
# The worker's token for every job of these tests: 64 lowercase hex characters (contracts 4.5).
JOB_TOKEN = "ab" * 32
TOKEN_METADATA_KEY = "rastai-job-token"
WORKER_MODULE = "rastai_worker"
PAUSE_VARIABLE = "RASTAI_TEST_PAUSE_AT"
FRAME_HEADER_FORMAT = "<II"
# The budget guards against a hang, not a slow machine: a real TextCNN run hit 60 s at load average ~90 (#234).
WAIT_LIMIT_SECONDS = 600.0
POLL_SECONDS = 0.02
# In /proc/<pid>/stat, after the command name's closing parenthesis: the state comes first, the process group third.
STATE_FIELD_AFTER_NAME = 0
GROUP_FIELD_AFTER_NAME = 2
STOPPED_STATE = "T"
EXITED_STATES = ("Z", "X", "x")
WORKER_HOST_SOCKET = "worker-host.sock"
REGISTRATIONS_FILE = "registrations.jsonl"
# The stand-in node: WorkerHostService.Register on worker-host.sock, as the node checks it after a restart: the
# BLAKE3 of the token must equal the job's job_token_hash (contracts 4.5), else RRS-J2002. Each call is appended to
# registrations.jsonl. A new one unlinks the socket a killed one left, so the worker sees a replaced socket and
# registers again.
FAKE_NODE_SOURCE = """
import json
import os
import sys
import threading
from concurrent import futures

import grpc
from rastai_formats import hash_bytes
from rastai_proto.common.v1.diagnostic_pb2 import SEVERITY_ERROR, Diagnostic
from rastai_proto.worker.v1 import worker_host_service_pb2, worker_host_service_pb2_grpc

socket_path, registrations_path, job_token_hash = sys.argv[1], sys.argv[2], sys.argv[3]


class StandInNode(worker_host_service_pb2_grpc.WorkerHostServiceServicer):
    def Register(self, request, context):
        token = dict(context.invocation_metadata()).get("rastai-job-token", "")
        hash_matches = hash_bytes(token.encode("ascii")) == job_token_hash
        line = {"job_id": request.job_id, "job_socket_path": request.job_socket_path, "hash_matches": hash_matches}
        with open(registrations_path, "a", encoding="utf-8") as registrations:
            registrations.write(json.dumps(line) + "\\n")
        if not hash_matches:
            refusal = Diagnostic(code="RRS-J2002", severity=SEVERITY_ERROR, message="The job token was rejected.")
            return worker_host_service_pb2.RegisterResponse(diagnostic=refusal)
        return worker_host_service_pb2.RegisterResponse()


if os.path.exists(socket_path):
    os.unlink(socket_path)
server = grpc.server(futures.ThreadPoolExecutor(max_workers=2))
worker_host_service_pb2_grpc.add_WorkerHostServiceServicer_to_server(StandInNode(), server)
server.add_insecure_port("unix:" + socket_path)
server.start()
print("serving", flush=True)
threading.Event().wait()
"""
FAKE_NODE_FILE = "stand_in_node.py"


@dataclass
class StartedProcess:
    """A process a test started in a session of its own: its pid is its process group's id."""

    process: subprocess.Popen
    description: str

    @property
    def pid(self) -> int:
        return self.process.pid


@dataclass
class JobFolders:
    """Where one test's jobs live: the run folder (shared by the run's jobs) and a short socket folder (a Unix socket
    path must fit in 108 bytes)."""

    run_folder: Path
    socket_folder: Path

    @property
    def event_log_path(self) -> Path:
        return self.run_folder / "events.log"

    @property
    def stderr_path(self) -> Path:
        return self.run_folder / "stderr.log"

    @property
    def registrations_path(self) -> Path:
        return self.socket_folder / REGISTRATIONS_FILE


def write_job_file(folders: JobFolders, repo_root: Path, fixtures_folder: Path, job: dict) -> Path:
    """The train fixture's job.json for one job of the run: `job["job_id"]`, `job["max_steps"]` and, for a job that
    continues the run, `job["resume_from"]` (a marker: the worker takes the newest checkpoint that loads)."""
    fields = json.loads((fixtures_folder / TRAIN_JOB_FIXTURE).read_text(encoding="utf-8"))
    arguments = json.loads(fields["job_spec"]["arguments_json"])
    for name in START_PATH_FIELDS:
        arguments["start_arguments"][name] = str(repo_root / arguments["start_arguments"][name])
    arguments["max_steps"] = job["max_steps"]
    fields["job_id"] = job["job_id"]
    fields["job_spec"]["job_id"] = job["job_id"]
    fields["job_spec"]["arguments_json"] = json.dumps(arguments)
    fields["job_socket_path"] = str(folders.socket_folder / f"{job['job_id']}.sock")
    fields["worker_host_socket_path"] = str(folders.socket_folder / WORKER_HOST_SOCKET)
    fields["run_folder"] = str(folders.run_folder)
    fields["resume_from"] = job.get("resume_from", "")
    fields["job_token_hash"] = job_token_hash()
    folders.run_folder.mkdir(parents=True, exist_ok=True)
    job_file = folders.run_folder / "job.json"
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return job_file


def start_worker(started: list, folders: JobFolders, job_file: Path, pause_at: str = "", module: str = WORKER_MODULE):
    """`python -m rastai_worker run --job-file <job.json>` as the node starts it: a session of its own, stdin from
    /dev/null, stderr appended to the run's stderr.log, the token in RASTAI_JOB_TOKEN, and the pause point if any.
    `module` names a stand-in for `rastai_worker` (a launcher on PYTHONPATH) for a test that patches the worker."""
    environment = dict(os.environ, RASTAI_JOB_TOKEN=JOB_TOKEN)
    environment.pop(PAUSE_VARIABLE, None)
    if pause_at:
        environment[PAUSE_VARIABLE] = pause_at
    if module != WORKER_MODULE:
        environment["PYTHONPATH"] = str(folders.socket_folder)
    with open(folders.stderr_path, "a", encoding="utf-8") as stderr_file:
        process = subprocess.Popen(
            [sys.executable, "-m", module, "run", "--job-file", str(job_file)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=stderr_file,
            env=environment,
            start_new_session=True,
        )
    worker = StartedProcess(process, f"worker {job_file}")
    started.append(worker)
    return worker


def start_stand_in_node(started: list, folders: JobFolders) -> StartedProcess:
    """A stand-in node process serving worker-host.sock; returns once it serves."""
    script = folders.socket_folder / FAKE_NODE_FILE
    script.write_text(FAKE_NODE_SOURCE, encoding="utf-8")
    socket_path = folders.socket_folder / WORKER_HOST_SOCKET
    process = subprocess.Popen(
        [sys.executable, str(script), str(socket_path), str(folders.registrations_path), job_token_hash()],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    node = StartedProcess(process, "stand-in node")
    started.append(node)
    assert process.stdout is not None
    assert process.stdout.readline().strip() == b"serving"
    return node


def job_token_hash() -> str:
    """What the node keeps of the token in job.json: its BLAKE3, `blake3:<64 hex>` (contracts 4.5)."""
    return hash_bytes(JOB_TOKEN.encode("ascii"))


def registrations(folders: JobFolders) -> list[dict]:
    if not folders.registrations_path.exists():
        return []
    lines = folders.registrations_path.read_text(encoding="utf-8").splitlines()
    return [json.loads(line) for line in lines if line.strip()]


def kill_9(started_process: StartedProcess) -> None:
    """SIGKILL to the process alone, as an outside kill sends it, then reaps it."""
    os.kill(started_process.pid, signal.SIGKILL)
    started_process.process.wait(timeout=WAIT_LIMIT_SECONDS)


def kill_group(started_process: StartedProcess) -> None:
    """SIGKILL to the whole process group, while any of it is left. The clean-up of every exit path."""
    if live_group_members(started_process.pid):
        os.killpg(started_process.pid, signal.SIGKILL)
    if started_process.process.poll() is None:
        started_process.process.wait(timeout=WAIT_LIMIT_SECONDS)


def stat_field(pid: int, field_after_name: int) -> str | None:
    """One field of /proc/<pid>/stat, counted after the command name (which may hold spaces and parentheses)."""
    try:
        text = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except OSError:
        return None
    fields = text[text.rindex(")") + 1 :].split()
    return fields[field_after_name] if field_after_name < len(fields) else None


def is_stopped(pid: int) -> bool:
    return stat_field(pid, STATE_FIELD_AFTER_NAME) == STOPPED_STATE


def has_exited(pid: int) -> bool:
    state = stat_field(pid, STATE_FIELD_AFTER_NAME)
    return state is None or state in EXITED_STATES


def live_group_members(process_group_id: int) -> list[int]:
    """The processes of the group that haven't exited."""
    members = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if stat_field(pid, GROUP_FIELD_AFTER_NAME) == str(process_group_id) and not has_exited(pid):
            members.append(pid)
    return members


def wait_until(condition, what: str, folders: JobFolders) -> None:
    """Polls `condition` until it holds; fails with `what` and the run's stderr.log after WAIT_LIMIT_SECONDS."""
    deadline = time.monotonic() + WAIT_LIMIT_SECONDS
    while not condition():
        if time.monotonic() > deadline:
            stderr = folders.stderr_path.read_text(encoding="utf-8") if folders.stderr_path.exists() else ""
            raise AssertionError(f"waited {WAIT_LIMIT_SECONDS} s for {what}; stderr.log:\n{stderr}")
        time.sleep(POLL_SECONDS)


def wait_until_stopped(worker: StartedProcess, folders: JobFolders) -> None:
    """Waits until the worker has stopped itself at its pause point. A worker that ended instead fails the test."""

    def stopped() -> bool:
        assert worker.process.poll() is None, f"the worker ended with {worker.process.returncode}"
        return is_stopped(worker.pid)

    wait_until(stopped, "the worker to stop at its pause point", folders)


def wait_for_exit(worker: StartedProcess) -> int:
    return worker.process.wait(timeout=WAIT_LIMIT_SECONDS)


@dataclass
class LogRead:
    """A run log's whole records, and the bytes after them that don't make a whole frame (a torn tail)."""

    records: list
    torn_bytes: int


def read_log(log_path: Path) -> LogRead:
    data = log_path.read_bytes()
    assert data.startswith(EVENT_LOG_HEADER)
    records = []
    offset = len(EVENT_LOG_HEADER)
    while offset + FRAME_HEADER_BYTES <= len(data):
        body_length, _checksum = struct.unpack_from(FRAME_HEADER_FORMAT, data, offset)
        body_start = offset + FRAME_HEADER_BYTES
        if body_start + body_length > len(data):
            break
        records.append(EventRecord.FromString(data[body_start : body_start + body_length]))
        offset = body_start + body_length
    return LogRead(records=records, torn_bytes=len(data) - offset)


def sequences(records: list) -> list[int]:
    return [record.sequence for record in records]
