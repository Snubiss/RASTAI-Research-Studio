"""The host views a job or a plugin command handler has of the node (layout frontend-and-plugins.md 11.2).

`WorkerHost` is the one client of `WorkerHostService` on worker-host.sock (contracts 4.5). It converts dicts to and
from rastai_proto messages, so engines never import rastai_proto (python layout 1.4 rule 7), and it is the only
file in this package that imports grpcio (contracts 2.5). Each method maps onto one RPC and lands with it: worker
protocol v0 has only `Register` (M1-10), so `register` is the only call so far.

Every call carries the job token in the gRPC metadata key `rastai-job-token` (contracts 3.2 and 4.5). A call the node
answered comes back with OK and, when it refused, a Diagnostic; that Diagnostic is raised as a PluginError with its
own code. A call that never reached the node is RRS-J2001, so callers (rastai_worker's host connection, which
retries Register) never see a grpc type.
"""

import json
import os
from pathlib import Path

import grpc
from rastai_proto.common.v1.diagnostic_pb2 import Diagnostic
from rastai_proto.worker.v1.worker_host_service_pb2 import RegisterRequest
from rastai_proto.worker.v1.worker_host_service_pb2_grpc import WorkerHostServiceStub

from rastai_plugin.catalog.generated import RRS_J2001
from rastai_plugin.errors import PluginError

# Contracts 4.5: the job token travels in this metadata key on every WorkerHostService call, never in a field.
JOB_TOKEN_METADATA_KEY = "rastai-job-token"
# Contracts 15.9: the node passes the job token to the worker in this environment variable; it is never on disk.
JOB_TOKEN_VARIABLE = "RASTAI_JOB_TOKEN"
UNIX_TARGET_PREFIX = "unix:"
# grpcio sends a unix: target's path as the HTTP/2 :authority, which the node's server (tonic and h2) refuses as a
# protocol error; a plain host name is what it accepts.
CHANNEL_OPTIONS = [("grpc.default_authority", "localhost")]
# How long one call may take. The node answers Register at once; a node that doesn't answer in this time is treated
# like one that isn't there, and the caller decides whether to retry (the node gives up after 60 s, contracts 9.1).
HOST_CALL_TIMEOUT_SECONDS = 10.0
# The job.json fields the host connection reads (contracts 8.4).
JOB_FILE_FIELDS = ("job_id", "job_socket_path", "worker_host_socket_path")
UNKNOWN_JOB_ID = "unknown"


class PluginHost:
    """What a command handler may ask of the node. Every call is checked there against the plugin's grant.

    Its calls (`execute_command`, `put_blob`, `get_blob`, `gateway_generate`) arrive with their RPCs (layout
    frontend-and-plugins.md 11.6), so for now it is the type a handler is given and nothing more."""


class WorkerHost(PluginHost):
    """A job's client of WorkerHostService on worker-host.sock (contracts 4.5)."""

    def __init__(
        self, worker_host_socket_path: str, job_id: str, job_socket_path: str, job_token: str, job_file_path: Path
    ) -> None:
        self.worker_host_socket_path = worker_host_socket_path
        self.job_id = job_id
        self.job_socket_path = job_socket_path
        self.job_token = job_token
        # The job's job.json on the node (contracts 8.4). A job entry is given only its JobSpec (contracts 12.3), so a
        # job reads the rest of its launch from here: the run folder, resume_from and, later, its resolved models.
        self.job_file_path = job_file_path

    def register(self) -> None:
        """Announces the job's socket to the node: the first call, on start and again on reattach. The worker serves
        WorkerService before it calls this, because the node calls Hello as soon as Register arrives."""
        request = RegisterRequest(job_id=self.job_id, job_socket_path=self.job_socket_path)
        # A channel per call: the calls are few, and nothing is left open when a job ends or the node restarts.
        with open_host_channel(self.worker_host_socket_path) as channel:
            stub = WorkerHostServiceStub(channel)
            try:
                response = stub.Register(request, metadata=self.token_metadata(), timeout=HOST_CALL_TIMEOUT_SECONDS)
            except grpc.RpcError as error:
                raise unreachable_host_error(self.job_id, self.worker_host_socket_path, error) from error
        raise_refusal(response.diagnostic)

    def token_metadata(self) -> list[tuple[str, str]]:
        return [(JOB_TOKEN_METADATA_KEY, self.job_token)]


def connect_worker_host(job_file_path: Path) -> WorkerHost:
    """The WorkerHost for the job that job.json describes: the socket paths and the job id from the file, the token
    from RASTAI_JOB_TOKEN. It makes no call yet; the worker calls register() once it serves its own socket."""
    launch = read_job_file_fields(job_file_path)
    job_token = os.environ.get(JOB_TOKEN_VARIABLE, "")
    if not job_token:
        raise PluginError(RRS_J2001, f"the worker of job {launch['job_id']} has no {JOB_TOKEN_VARIABLE}")
    return WorkerHost(
        launch["worker_host_socket_path"], launch["job_id"], launch["job_socket_path"], job_token, job_file_path
    )


def read_job_file_fields(job_file_path: Path) -> dict[str, str]:
    """The JOB_FILE_FIELDS of job.json, each a non-empty string. rastai_worker reads the rest of the file."""
    try:
        launch = json.loads(job_file_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise job_file_error(job_file_path, f"it can't be read as JSON: {error}") from error
    if not isinstance(launch, dict):
        raise job_file_error(job_file_path, "it isn't a JSON object")
    fields = {}
    for name in JOB_FILE_FIELDS:
        value = launch.get(name)
        if not isinstance(value, str) or not value:
            raise job_file_error(job_file_path, f"it has no {name}")
        fields[name] = value
    return fields


def job_file_error(job_file_path: Path, reason: str) -> PluginError:
    return PluginError(RRS_J2001, f"the worker of job {UNKNOWN_JOB_ID} can't use {job_file_path}: {reason}")


def open_host_channel(socket_path: str) -> grpc.Channel:
    return grpc.insecure_channel(UNIX_TARGET_PREFIX + socket_path, options=CHANNEL_OPTIONS)


def raise_refusal(diagnostic: Diagnostic) -> None:
    """The node accepted the call when the response's diagnostic has no code; otherwise its diagnostic is the reason
    it refused (RRS-J2002 for a wrong token)."""
    if diagnostic.code:
        raise PluginError(diagnostic.code, diagnostic.message)


def unreachable_host_error(job_id: str, socket_path: str, error: grpc.RpcError) -> PluginError:
    return PluginError(
        RRS_J2001, f"the worker of job {job_id} can't reach the node at {socket_path}: {rpc_failure(error)}"
    )


def rpc_failure(error: grpc.RpcError) -> str:
    # A failed call raises an RpcError that is also a grpc.Call, which carries the status code and its details.
    if isinstance(error, grpc.Call):
        return f"{error.code().name}: {error.details()}"
    return str(error)
