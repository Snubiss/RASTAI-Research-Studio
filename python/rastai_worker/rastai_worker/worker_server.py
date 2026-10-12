"""WorkerService on the job's socket, `jobs/<job_id>.sock` (contracts 3.2 and 4.5; python layout 7.6).

The node is the only client. This file is the only one in rastai_worker that imports grpcio (contracts 2.5).

The handlers run on grpcio's thread pool and do three things only, so they never touch the model, the log or the
GPU (python layout 1.4 rule 8):
- **Hello** answers from the WorkerInfo built at start.
- **Control** flips a JobControl flag (control.py); a value that isn't a Control value comes back as the
  response's diagnostic (RRS-J0004).
- **WatchLive** streams the live events the job puts on its LiveStream, until the node goes away or the job exits.

Every call must carry the job's token in the `rastai-job-token` metadata key (contracts 4.5): a call without it, or
with another token, is refused as UNAUTHENTICATED before it does anything. The job's socket sits in the node's
0700 runtime folder, so the token is the second lock, not the only one.
"""

import hmac
import importlib.metadata
import os
from concurrent import futures

import grpc
from rastai_plugin.catalog.generated import RRS_J2001
from rastai_plugin.host import JOB_TOKEN_METADATA_KEY
from rastai_proto.common.v1.diagnostic_pb2 import Diagnostic, Severity
from rastai_proto.node.v1.jobs_pb2 import JobSignal
from rastai_proto.worker.v1 import worker_service_pb2, worker_service_pb2_grpc

from rastai_plugin import JobControl
from rastai_worker.control import apply_control_action
from rastai_worker.diagnostics import diagnostic_from_error
from rastai_worker.errors import WorkerError
from rastai_worker.job_file import WorkerLaunch
from rastai_worker.live_stream import LiveStream

# The worker protocol this worker speaks (contracts 9.1); the node checks it in Hello (RRS-J2003).
WORKER_PROTOCOL_VERSION = 1
# The distribution whose version is the rastai version (app, node and worker ship together, §20).
RASTAI_WORKER_DISTRIBUTION = "rastai-worker"
UNIX_TARGET_PREFIX = "unix:"
# Hello and Control are instant; each open WatchLive holds one thread. Eight leave room for a node that reconnects
# while its old stream is still ending.
SERVER_THREADS = 8
# How long a WatchLive waits for the next event before it checks whether the node is still there.
WATCH_LIVE_WAIT_SECONDS = 0.5
# How long calls in flight get to finish when the job exits.
SERVER_STOP_GRACE_SECONDS = 1.0
# JOB_SIGNAL_STOP is the Control value STOP.
JOB_SIGNAL_PREFIX = "JOB_SIGNAL_"


class WorkerServer(worker_service_pb2_grpc.WorkerServiceServicer):
    """The node's view of the job: Hello, Control and WatchLive."""

    def __init__(
        self, worker_info: worker_service_pb2.WorkerInfo, control: JobControl, live: LiveStream, job_token: str
    ) -> None:
        self.worker_info = worker_info
        self.control = control
        self.live = live
        self.job_token = job_token

    def Hello(self, request, context):  # noqa: N802 (the generated servicer names the method)
        self.check_token(context)
        return self.worker_info

    def Control(self, request, context):  # noqa: N802
        self.check_token(context)
        try:
            apply_control_action(self.control, control_action_name(request.action))
        except WorkerError as error:
            diagnostic = diagnostic_from_error(error, self.worker_info.job_id)
            return worker_service_pb2.ControlResponse(diagnostic=diagnostic_message(diagnostic))
        return worker_service_pb2.ControlResponse()

    def WatchLive(self, request, context):  # noqa: N802
        self.check_token(context)
        after_index = 0
        while context.is_active() and not self.live.is_closed:
            for index, _event_type, event in self.live.read(after_index, WATCH_LIVE_WAIT_SECONDS):
                after_index = index
                yield event

    def check_token(self, context: grpc.ServicerContext) -> None:
        """Refuses a call that doesn't carry this job's token. compare_digest takes the same time wherever the
        tokens differ, so the answer's timing says nothing about the token."""
        sent_token = dict(context.invocation_metadata()).get(JOB_TOKEN_METADATA_KEY, "")
        if not hmac.compare_digest(sent_token.encode("utf-8"), self.job_token.encode("utf-8")):
            context.abort(grpc.StatusCode.UNAUTHENTICATED, "the call doesn't carry this job's token")


def start_worker_server(launch: WorkerLaunch, control: JobControl, live: LiveStream, job_token: str) -> grpc.Server:
    """Serves WorkerService on the job's socket and returns the running server. A socket that can't be bound is
    RRS-J2001: the node could never reach this job."""
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=SERVER_THREADS))
    service = WorkerServer(worker_info_for(launch), control, live, job_token)
    worker_service_pb2_grpc.add_WorkerServiceServicer_to_server(service, server)
    try:
        bound_port = server.add_insecure_port(UNIX_TARGET_PREFIX + launch.job_socket_path)
    except RuntimeError as error:
        raise socket_error(launch, str(error)) from error
    # Older grpcio versions report a failed bind as port 0 instead of raising.
    if bound_port == 0:
        raise socket_error(launch, "grpcio couldn't bind it")
    server.start()
    return server


def stop_worker_server(server: grpc.Server, live: LiveStream) -> None:
    """Ends the open WatchLive streams, then stops serving. The node removes the socket file once the job ends."""
    live.close()
    server.stop(SERVER_STOP_GRACE_SECONDS).wait()


def worker_info_for(launch: WorkerLaunch) -> worker_service_pb2.WorkerInfo:
    return worker_service_pb2.WorkerInfo(
        job_id=launch.job_id,
        pid=os.getpid(),
        kind=launch.job_spec.kind,
        protocol_version=WORKER_PROTOCOL_VERSION,
        rastai_version=installed_rastai_version(),
        environment_id=launch.environment_id,
    )


def installed_rastai_version() -> str:
    try:
        return importlib.metadata.version(RASTAI_WORKER_DISTRIBUTION)
    except importlib.metadata.PackageNotFoundError:
        return ""


def control_action_name(action: int) -> str:
    """The Control value's name without its prefix ("STOP"); a number this worker doesn't know reads as one, so it
    is refused like any other value that isn't a Control value."""
    if action not in JobSignal.values():
        return f"number {action}"
    return JobSignal.Name(action).removeprefix(JOB_SIGNAL_PREFIX)


def diagnostic_message(diagnostic: dict) -> Diagnostic:
    return Diagnostic(
        code=diagnostic["code"],
        severity=Severity.Value(diagnostic["severity"]),
        message=diagnostic["message"],
        fix_text=diagnostic.get("fix_text", ""),
    )


def socket_error(launch: WorkerLaunch, reason: str) -> WorkerError:
    return WorkerError(RRS_J2001, f"the job socket {launch.job_socket_path} can't be served: {reason}")
