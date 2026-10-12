"""WorkerService over a temporary Unix socket (worker_server.py; contracts 4.5; python layout 7.9): Hello, Control and
WatchLive in process, with the job token checked on every call; then the whole job process against a fake node:
serve, Register with the token, Hello, WatchLive, and STOP or SIGTERM ending it with exit 0 or 75.

HandleCommand joins this test with its RPC (M4).
"""

import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Iterator
from concurrent import futures
from dataclasses import asdict
from pathlib import Path

import grpc
import pytest
from rastai_plugin.catalog.generated import RRS_J0004, RRS_J2001, RRS_J2002
from rastai_proto.common.v1.diagnostic_pb2 import SEVERITY_ERROR, Diagnostic
from rastai_proto.common.v1.states_pb2 import RUN_STATE_COMPLETED, RUN_STATE_PREEMPTED
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.node.v1.jobs_pb2 import JOB_KIND_TRAIN, JobSignal
from rastai_proto.node.v1.live_pb2 import WatchLiveRequest
from rastai_proto.worker.v1 import (
    worker_host_service_pb2,
    worker_host_service_pb2_grpc,
    worker_service_pb2,
    worker_service_pb2_grpc,
)
from rastai_worker.errors import WorkerError
from rastai_worker.event_log import EVENT_LOG_HEADER, FRAME_HEADER_BYTES, EventLogWriter
from rastai_worker.exit_codes import EXIT_BAD_LAUNCH, EXIT_OK, EXIT_PREEMPTED
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.job_file import WorkerLaunch, read_job_file
from rastai_worker.live_stream import LiveStream
from rastai_worker.worker_server import WORKER_PROTOCOL_VERSION, start_worker_server, stop_worker_server

from rastai_plugin import JobControl

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
# 32 random bytes as 64 lowercase hex characters, the form of RASTAI_JOB_TOKEN (contracts 4.5).
JOB_TOKEN = "ab" * 32
OTHER_TOKEN = "cd" * 32
TOKEN_METADATA_KEY = "rastai-job-token"
# A short folder: a Unix socket path must fit in 108 bytes, and pytest's tmp_path can be long.
SOCKET_FOLDER_PREFIX = "rr-worker-"
UNIX_TARGET_PREFIX = "unix:"
CALL_TIMEOUT_SECONDS = 5.0
PROCESS_TIMEOUT_SECONDS = 60.0
POLL_SECONDS = 0.02
FRAME_HEADER_FORMAT = "<II"
# The job entry the end-to-end tests run in place of the train job's (jobs/train.py lands with M1-46): it logs one
# metric and streams progress until it is asked to end.
ENTRY_MODULE_NAME = "rastai_test_server_entries"
ENTRY_MODULE_SOURCE = """
import time


def run_job(job, host, events, control):
    events.emit_durable("run.metrics", {"step": 1, "values": {"train.loss": 2.5}})
    while not (control.stop_requested or control.cancel_requested or control.preempt_requested):
        events.emit_ephemeral("job.progress", {"fraction": 0.5, "message": "waiting for the test"})
        time.sleep(0.05)
"""
# `python -m rastai_worker` with JOB_ENTRIES pointing the train kind at the entry above, so the whole process runs
# as the node starts it while no job module is built yet.
LAUNCHER_MODULE_NAME = "rastai_test_worker_main"
LAUNCHER_MODULE_SOURCE = f"""
import sys

from rastai_worker import dispatch
from rastai_worker.__main__ import main

dispatch.JOB_ENTRIES["train"] = "{ENTRY_MODULE_NAME}:run_job"
sys.exit(main(sys.argv[1:]))
"""


@pytest.fixture
def socket_folder() -> Iterator[Path]:
    folder = Path(tempfile.mkdtemp(prefix=SOCKET_FOLDER_PREFIX))
    yield folder
    shutil.rmtree(folder, ignore_errors=True)


def write_launch(socket_folder: Path, run_folder: Path, kind: str, entry: str = "") -> WorkerLaunch:
    run_folder.mkdir(parents=True, exist_ok=True)
    fields = {
        "job_id": JOB_ID,
        "run_id": RUN_ID,
        "job_spec": {"job_id": JOB_ID, "run_id": RUN_ID, "kind": kind, "entry": entry, "environment_id": "cpu"},
        "job_socket_path": str(socket_folder / "job.sock"),
        "worker_host_socket_path": str(socket_folder / "worker-host.sock"),
        "run_folder": str(run_folder),
        "environment_id": "cpu",
    }
    job_file = run_folder / "job.json"
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return read_job_file(job_file)


def worker_stub(socket_path: str) -> tuple[grpc.Channel, worker_service_pb2_grpc.WorkerServiceStub]:
    channel = grpc.insecure_channel(UNIX_TARGET_PREFIX + socket_path)
    return channel, worker_service_pb2_grpc.WorkerServiceStub(channel)


def token_metadata(token: str = JOB_TOKEN) -> list[tuple[str, str]]:
    return [(TOKEN_METADATA_KEY, token)]


# ---- In process ----


class ServedJob:
    """A WorkerService started by start_worker_server, as the job process starts it, and a client for it."""

    def __init__(self, socket_folder: Path, run_folder: Path) -> None:
        self.launch = write_launch(socket_folder, run_folder, "JOB_KIND_TRAIN")
        self.control = JobControl()
        self.live = LiveStream()
        self.log = EventLogWriter(self.launch.event_log_path, RUN_ID, JOB_ID)
        self.events = WorkerJobEvents(self.log, self.live, self.control, JOB_ID)
        self.server = start_worker_server(self.launch, self.control, self.live, JOB_TOKEN)
        self.channel, self.stub = worker_stub(self.launch.job_socket_path)

    def close(self) -> None:
        self.channel.close()
        stop_worker_server(self.server, self.live)
        self.log.close()


@pytest.fixture
def served_job(socket_folder: Path, tmp_path: Path) -> Iterator[ServedJob]:
    job = ServedJob(socket_folder, tmp_path / "run")
    yield job
    job.close()


def test_hello_answers_with_the_worker_info(served_job: ServedJob) -> None:
    info = served_job.stub.Hello(
        worker_service_pb2.HelloRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS
    )
    assert info.job_id == JOB_ID
    assert info.pid == os.getpid()
    assert info.kind == JOB_KIND_TRAIN
    assert info.protocol_version == WORKER_PROTOCOL_VERSION == 1
    assert info.rastai_version == "0.1.0"
    assert info.environment_id == "cpu"


@pytest.mark.parametrize("metadata", [[], token_metadata(OTHER_TOKEN), token_metadata(JOB_TOKEN[:-1])])
def test_every_call_without_the_job_token_is_refused(served_job: ServedJob, metadata: list) -> None:
    calls = [
        lambda: served_job.stub.Hello(
            worker_service_pb2.HelloRequest(), metadata=metadata, timeout=CALL_TIMEOUT_SECONDS
        ),
        lambda: served_job.stub.Control(
            worker_service_pb2.ControlRequest(action=JobSignal.JOB_SIGNAL_STOP),
            metadata=metadata,
            timeout=CALL_TIMEOUT_SECONDS,
        ),
        lambda: next(served_job.stub.WatchLive(WatchLiveRequest(), metadata=metadata, timeout=CALL_TIMEOUT_SECONDS)),
    ]
    for call in calls:
        with pytest.raises(grpc.RpcError) as raised:
            call()
        assert raised.value.code() == grpc.StatusCode.UNAUTHENTICATED
    assert asdict(served_job.control) == asdict(JobControl())


def control(served_job: ServedJob, action: int) -> worker_service_pb2.ControlResponse:
    request = worker_service_pb2.ControlRequest(action=action)
    return served_job.stub.Control(request, metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)


def test_control_sets_flags_and_never_preempts(served_job: ServedJob) -> None:
    response = control(served_job, JobSignal.JOB_SIGNAL_STOP)
    assert not response.HasField("diagnostic")
    expected = asdict(JobControl(stop_requested=True))
    assert asdict(served_job.control) == expected
    control(served_job, JobSignal.JOB_SIGNAL_PAUSE)
    assert served_job.control.paused
    control(served_job, JobSignal.JOB_SIGNAL_RESUME)
    control(served_job, JobSignal.JOB_SIGNAL_CHECKPOINT_NOW)
    control(served_job, JobSignal.JOB_SIGNAL_CANCEL)
    assert asdict(served_job.control) == asdict(
        JobControl(stop_requested=True, checkpoint_requested=True, cancel_requested=True)
    )


@pytest.mark.parametrize("action", [JobSignal.JOB_SIGNAL_TERMINATE, JobSignal.JOB_SIGNAL_KILL, 0, 42])
def test_control_refuses_what_isnt_a_control_value(served_job: ServedJob, action: int) -> None:
    response = control(served_job, action)
    assert response.diagnostic.code == RRS_J0004
    assert response.diagnostic.severity == SEVERITY_ERROR
    # The catalog message names the job: "Signal {signal} wasn't delivered to job {job_id}: {reason}."
    assert f"wasn't delivered to job {JOB_ID}: " in response.diagnostic.message
    assert "{" not in response.diagnostic.message
    assert asdict(served_job.control) == asdict(JobControl())


def test_watch_live_streams_the_jobs_ephemeral_events(served_job: ServedJob) -> None:
    served_job.events.emit_ephemeral("job.progress", {"fraction": 0.25, "message": "tokenizing 1 of 4 files"})
    stream = served_job.stub.WatchLive(WatchLiveRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
    first = next(stream)
    assert first.job_id == JOB_ID
    assert first.emitted_unix_ms > 0
    assert first.job_progress.message == "tokenizing 1 of 4 files"
    served_job.events.emit_ephemeral("job.progress", {"fraction": 0.5, "message": "tokenizing 2 of 4 files"})
    assert next(stream).job_progress.fraction == 0.5
    # Ephemeral events never touch the run's log.
    assert served_job.launch.event_log_path.read_bytes() == EVENT_LOG_HEADER
    stream.cancel()


def test_watch_live_ends_when_the_job_exits(served_job: ServedJob) -> None:
    stream = served_job.stub.WatchLive(WatchLiveRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
    threading.Timer(0.2, served_job.live.close).start()
    assert list(stream) == []


def test_watch_live_sends_the_events_put_just_before_the_close(served_job: ServedJob) -> None:
    served_job.events.emit_ephemeral("job.progress", {"fraction": 1.0, "message": "done"})
    served_job.live.close()
    stream = served_job.stub.WatchLive(WatchLiveRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
    assert [event.job_progress.message for event in stream] == ["done"]


def test_a_socket_that_cant_be_bound_is_a_bad_launch(tmp_path: Path) -> None:
    launch = write_launch(tmp_path / "no-such-folder", tmp_path / "run", "JOB_KIND_TRAIN")
    with pytest.raises(WorkerError) as raised:
        start_worker_server(launch, JobControl(), LiveStream(), JOB_TOKEN)
    assert raised.value.code == RRS_J2001


# ---- The whole process against a fake node ----


class FakeNode(worker_host_service_pb2_grpc.WorkerHostServiceServicer):
    """The node's WorkerHostService: records each Register and its token, and refuses another job's token."""

    def __init__(self) -> None:
        self.registrations = []
        self.lock = threading.Lock()

    def Register(self, request, context):  # noqa: N802 (the generated servicer names the method)
        token = dict(context.invocation_metadata()).get(TOKEN_METADATA_KEY, "")
        with self.lock:
            self.registrations.append((request.job_id, request.job_socket_path, token))
        if token != JOB_TOKEN:
            refusal = Diagnostic(
                code=RRS_J2002, severity=SEVERITY_ERROR, message=f"The token of job {JOB_ID} was rejected."
            )
            return worker_host_service_pb2.RegisterResponse(diagnostic=refusal)
        return worker_host_service_pb2.RegisterResponse()


@pytest.fixture
def fake_node(socket_folder: Path) -> Iterator[FakeNode]:
    node = FakeNode()
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=2))
    worker_host_service_pb2_grpc.add_WorkerHostServiceServicer_to_server(node, server)
    server.add_insecure_port(UNIX_TARGET_PREFIX + str(socket_folder / "worker-host.sock"))
    server.start()
    yield node
    server.stop(0.5).wait()


class WorkerProcess:
    """`python -m rastai_worker run --job-file <job.json>` as the node spawns it, through the test's launcher: its
    own session, stdin from /dev/null, stderr appended to the run folder's stderr.log, the token in
    RASTAI_JOB_TOKEN."""

    def __init__(self, launch: WorkerLaunch, entries_folder: Path, job_token: str = JOB_TOKEN) -> None:
        self.launch = launch
        self.stderr_path = launch.run_folder / "stderr.log"
        environment = dict(os.environ, RASTAI_JOB_TOKEN=job_token, PYTHONPATH=str(entries_folder))
        with open(self.stderr_path, "a", encoding="utf-8") as stderr_file:
            self.process = subprocess.Popen(
                [sys.executable, "-m", LAUNCHER_MODULE_NAME, "run", "--job-file", str(launch.job_file)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=stderr_file,
                env=environment,
                start_new_session=True,
            )

    def wait(self) -> int:
        try:
            return self.process.wait(timeout=PROCESS_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise

    def stderr(self) -> str:
        return self.stderr_path.read_text(encoding="utf-8")

    def kill_if_running(self) -> None:
        """SIGKILL to the worker's group when a failed test left it running, so no worker outlives the run."""
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=PROCESS_TIMEOUT_SECONDS)


@pytest.fixture
def entries_folder(tmp_path: Path) -> Path:
    folder = tmp_path / "entries"
    folder.mkdir()
    (folder / f"{ENTRY_MODULE_NAME}.py").write_text(ENTRY_MODULE_SOURCE, encoding="utf-8")
    (folder / f"{LAUNCHER_MODULE_NAME}.py").write_text(LAUNCHER_MODULE_SOURCE, encoding="utf-8")
    return folder


@pytest.fixture
def started_workers() -> Iterator[list[WorkerProcess]]:
    """The workers a test started; any still running when the test ends is killed."""
    workers = []
    yield workers
    for worker in workers:
        worker.kill_if_running()


def start_worker(
    socket_folder: Path,
    tmp_path: Path,
    entries_folder: Path,
    started_workers: list[WorkerProcess],
    job_token: str = JOB_TOKEN,
) -> WorkerProcess:
    launch = write_launch(socket_folder, tmp_path / "run", "JOB_KIND_TRAIN")
    worker = WorkerProcess(launch, entries_folder, job_token)
    started_workers.append(worker)
    return worker


def wait_for_registration(node: FakeNode, worker: WorkerProcess) -> None:
    deadline = time.monotonic() + PROCESS_TIMEOUT_SECONDS
    while not node.registrations and worker.process.poll() is None and time.monotonic() < deadline:
        time.sleep(POLL_SECONDS)
    assert node.registrations, worker.stderr()


def read_records(log_path: Path) -> list[EventRecord]:
    data = log_path.read_bytes()
    records = []
    offset = len(EVENT_LOG_HEADER)
    while offset < len(data):
        body_length, _checksum = struct.unpack_from(FRAME_HEADER_FORMAT, data, offset)
        body_start = offset + FRAME_HEADER_BYTES
        records.append(EventRecord.FromString(data[body_start : body_start + body_length]))
        offset = body_start + body_length
    return records


def hello_watch_and_end(worker: WorkerProcess, end_the_job) -> None:
    """What the node does after Register: Hello, WatchLive, then the test's way of ending the job."""
    channel, stub = worker_stub(worker.launch.job_socket_path)
    try:
        info = stub.Hello(worker_service_pb2.HelloRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
        assert (info.job_id, info.pid, info.kind) == (JOB_ID, worker.process.pid, JOB_KIND_TRAIN)
        stream = stub.WatchLive(WatchLiveRequest(), metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
        assert next(stream).job_progress.message == "waiting for the test"
        stream.cancel()
        end_the_job(stub)
    finally:
        channel.close()


def test_the_job_process_serves_registers_and_stops(
    socket_folder: Path,
    tmp_path: Path,
    fake_node: FakeNode,
    entries_folder: Path,
    started_workers: list[WorkerProcess],
) -> None:
    worker = start_worker(socket_folder, tmp_path, entries_folder, started_workers)
    wait_for_registration(fake_node, worker)
    assert fake_node.registrations[0] == (JOB_ID, str(socket_folder / "job.sock"), JOB_TOKEN)

    def send_stop(stub) -> None:
        request = worker_service_pb2.ControlRequest(action=JobSignal.JOB_SIGNAL_STOP)
        response = stub.Control(request, metadata=token_metadata(), timeout=CALL_TIMEOUT_SECONDS)
        assert not response.HasField("diagnostic")

    hello_watch_and_end(worker, send_stop)
    assert worker.wait() == EXIT_OK, worker.stderr()
    records = read_records(worker.launch.event_log_path)
    assert [record.event_type for record in records] == ["run.metrics", "run.state_changed"]
    assert records[-1].job_id == JOB_ID
    final = records[-1].payload.run_state_changed
    assert (final.run_state, final.reason) == (RUN_STATE_COMPLETED, "stopped")
    # The reserve is deleted on exit, and nothing reached stderr.log.
    assert not (worker.launch.run_folder / "events.log.reserve").exists()
    assert worker.stderr() == ""


def test_sigterm_ends_the_job_process_preempted(
    socket_folder: Path,
    tmp_path: Path,
    fake_node: FakeNode,
    entries_folder: Path,
    started_workers: list[WorkerProcess],
) -> None:
    worker = start_worker(socket_folder, tmp_path, entries_folder, started_workers)
    wait_for_registration(fake_node, worker)
    hello_watch_and_end(worker, lambda _stub: os.killpg(worker.process.pid, signal.SIGTERM))
    assert worker.wait() == EXIT_PREEMPTED, worker.stderr()
    final = read_records(worker.launch.event_log_path)[-1].payload.run_state_changed
    assert final.run_state == RUN_STATE_PREEMPTED


def test_a_refused_registration_is_a_bad_launch(
    socket_folder: Path,
    tmp_path: Path,
    fake_node: FakeNode,
    entries_folder: Path,
    started_workers: list[WorkerProcess],
) -> None:
    worker = start_worker(socket_folder, tmp_path, entries_folder, started_workers, job_token=OTHER_TOKEN)
    assert worker.wait() == EXIT_BAD_LAUNCH
    assert len(fake_node.registrations) == 1
    assert worker.stderr().strip().splitlines()[-1].startswith(f"{RRS_J2002}: ")
    # The job never ran, so the log holds no record of it.
    assert read_records(worker.launch.event_log_path) == []
