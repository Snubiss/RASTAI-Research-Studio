"""The host client (rastai_plugin/host.py): WorkerHost.register against a fake WorkerHostService on a temporary Unix
socket, and connect_worker_host reading job.json and RASTAI_JOB_TOKEN (contracts 3.2 and 4.5).

The fake service plays the node: it records each Register with the token it carried and refuses a wrong token with
RRS-J2002, as the node does.
"""

import json
import shutil
import tempfile
import threading
from collections.abc import Iterator
from concurrent import futures
from pathlib import Path

import grpc
import pytest
from rastai_plugin.catalog.generated import RRS_J2001, RRS_J2002
from rastai_proto.common.v1.diagnostic_pb2 import SEVERITY_ERROR, Diagnostic
from rastai_proto.worker.v1 import worker_host_service_pb2, worker_host_service_pb2_grpc

from rastai_plugin import PluginError, WorkerHost, connect_worker_host

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
# 32 random bytes as 64 lowercase hex characters, the form of RASTAI_JOB_TOKEN (contracts 4.5).
JOB_TOKEN = "ab" * 32
TOKEN_METADATA_KEY = "rastai-job-token"
TOKEN_VARIABLE = "RASTAI_JOB_TOKEN"
SOCKET_FILE_NAME = "worker-host.sock"
JOB_FILE_NAME = "job.json"
SERVER_THREADS = 2
SERVER_STOP_GRACE_SECONDS = 0.5
# A short folder: a Unix socket path must fit in 108 bytes, and pytest's tmp_path can be long.
SOCKET_FOLDER_PREFIX = "rr-host-"


class FakeWorkerHostService(worker_host_service_pb2_grpc.WorkerHostServiceServicer):
    """The node's side of Register: checks the token in the metadata, records what arrived."""

    def __init__(self, expected_token: str) -> None:
        self.expected_token = expected_token
        self.calls = []
        self.lock = threading.Lock()

    def Register(self, request, context):  # noqa: N802 (the generated servicer names the method)
        token = dict(context.invocation_metadata()).get(TOKEN_METADATA_KEY, "")
        with self.lock:
            self.calls.append({"job_id": request.job_id, "job_socket_path": request.job_socket_path, "token": token})
        if token != self.expected_token:
            refusal = Diagnostic(
                code=RRS_J2002, severity=SEVERITY_ERROR, message=f"The token of job {request.job_id} was rejected."
            )
            return worker_host_service_pb2.RegisterResponse(diagnostic=refusal)
        return worker_host_service_pb2.RegisterResponse()


@pytest.fixture
def socket_folder() -> Iterator[Path]:
    folder = Path(tempfile.mkdtemp(prefix=SOCKET_FOLDER_PREFIX))
    yield folder
    shutil.rmtree(folder, ignore_errors=True)


@pytest.fixture
def fake_node(socket_folder: Path) -> Iterator[tuple[FakeWorkerHostService, str]]:
    """A grpcio server with the fake service on <socket_folder>/worker-host.sock; yields the service and the path."""
    socket_path = str(socket_folder / SOCKET_FILE_NAME)
    service = FakeWorkerHostService(JOB_TOKEN)
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=SERVER_THREADS))
    worker_host_service_pb2_grpc.add_WorkerHostServiceServicer_to_server(service, server)
    server.add_insecure_port("unix:" + socket_path)
    server.start()
    yield service, socket_path
    server.stop(SERVER_STOP_GRACE_SECONDS).wait()


def write_job_file(folder: Path, fields: dict) -> Path:
    job_file = folder / JOB_FILE_NAME
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return job_file


def job_file_fields(socket_folder: Path, worker_host_socket_path: str) -> dict:
    # The fields connect_worker_host reads, plus some of the others the node writes (contracts 8.4).
    return {
        "job_id": JOB_ID,
        "run_id": "run_01JA2B3C4D5E6F7G8H9J0K1M2N",
        "job_socket_path": str(socket_folder / f"{JOB_ID}.sock"),
        "worker_host_socket_path": worker_host_socket_path,
        "job_token_hash": "blake3:" + "0" * 64,
    }


def test_register_sends_the_job_its_socket_and_the_token(fake_node, socket_folder: Path) -> None:
    service, socket_path = fake_node
    job_socket_path = str(socket_folder / f"{JOB_ID}.sock")
    host = WorkerHost(socket_path, JOB_ID, job_socket_path, JOB_TOKEN, socket_folder / JOB_FILE_NAME)
    host.register()
    assert service.calls == [{"job_id": JOB_ID, "job_socket_path": job_socket_path, "token": JOB_TOKEN}]


def test_the_channel_names_localhost_as_its_authority(
    fake_node, socket_folder: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # grpcio would send the socket path as :authority, which the real node (tonic) refuses. The fake grpcio server
    # accepts any authority, so this checks that the option reaches the channel. The check against the real server
    # waits for M1-43, the first task in which the node serves WorkerHostService.
    channel_options = []
    real_insecure_channel = grpc.insecure_channel

    def recording_insecure_channel(target, options=None):
        channel_options.append(options)
        return real_insecure_channel(target, options=options)

    monkeypatch.setattr(grpc, "insecure_channel", recording_insecure_channel)
    service, socket_path = fake_node
    WorkerHost(
        socket_path, JOB_ID, str(socket_folder / f"{JOB_ID}.sock"), JOB_TOKEN, socket_folder / JOB_FILE_NAME
    ).register()
    assert channel_options == [[("grpc.default_authority", "localhost")]]
    assert len(service.calls) == 1


def test_register_again_on_reattach_is_a_second_call(fake_node, socket_folder: Path) -> None:
    service, socket_path = fake_node
    host = WorkerHost(
        socket_path, JOB_ID, str(socket_folder / f"{JOB_ID}.sock"), JOB_TOKEN, socket_folder / JOB_FILE_NAME
    )
    host.register()
    host.register()
    assert len(service.calls) == 2


def test_a_refused_register_raises_the_nodes_code(fake_node, socket_folder: Path) -> None:
    service, socket_path = fake_node
    host = WorkerHost(
        socket_path, JOB_ID, str(socket_folder / f"{JOB_ID}.sock"), "cd" * 32, socket_folder / JOB_FILE_NAME
    )
    with pytest.raises(PluginError) as caught:
        host.register()
    assert caught.value.code == RRS_J2002
    assert JOB_ID in caught.value.message


def test_register_without_a_node_is_j2001(socket_folder: Path) -> None:
    missing_socket = str(socket_folder / SOCKET_FILE_NAME)
    host = WorkerHost(
        missing_socket, JOB_ID, str(socket_folder / f"{JOB_ID}.sock"), JOB_TOKEN, socket_folder / JOB_FILE_NAME
    )
    with pytest.raises(PluginError) as caught:
        host.register()
    assert caught.value.code == RRS_J2001
    assert missing_socket in caught.value.message
    assert "UNAVAILABLE" in caught.value.message


def test_connect_worker_host_reads_job_json_and_the_token(
    fake_node, socket_folder: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    service, socket_path = fake_node
    job_file = write_job_file(socket_folder, job_file_fields(socket_folder, socket_path))
    monkeypatch.setenv(TOKEN_VARIABLE, JOB_TOKEN)
    host = connect_worker_host(job_file)
    # Building the client makes no call; the worker registers once it serves its own socket.
    assert service.calls == []
    # A job reads the rest of its launch from the same file.
    assert host.job_file_path == job_file
    host.register()
    assert service.calls == [
        {"job_id": JOB_ID, "job_socket_path": str(socket_folder / f"{JOB_ID}.sock"), "token": JOB_TOKEN}
    ]


def test_connect_worker_host_without_the_token_is_j2001(socket_folder: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    job_file = write_job_file(socket_folder, job_file_fields(socket_folder, str(socket_folder / SOCKET_FILE_NAME)))
    monkeypatch.delenv(TOKEN_VARIABLE, raising=False)
    with pytest.raises(PluginError) as caught:
        connect_worker_host(job_file)
    assert caught.value.code == RRS_J2001
    assert TOKEN_VARIABLE in caught.value.message


def test_connect_worker_host_with_a_bad_job_file_is_j2001(socket_folder: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(TOKEN_VARIABLE, JOB_TOKEN)
    fields = job_file_fields(socket_folder, str(socket_folder / SOCKET_FILE_NAME))
    del fields["worker_host_socket_path"]
    cases = [
        (socket_folder / "missing.json", "can't be read"),
        (write_job_file(socket_folder, fields), "worker_host_socket_path"),
    ]
    not_json = socket_folder / "not-json.json"
    not_json.write_text("{", encoding="utf-8")
    cases.append((not_json, "can't be read as JSON"))
    for job_file, expected_text in cases:
        with pytest.raises(PluginError) as caught:
            connect_worker_host(job_file)
        assert caught.value.code == RRS_J2001
        assert expected_text in caught.value.message
