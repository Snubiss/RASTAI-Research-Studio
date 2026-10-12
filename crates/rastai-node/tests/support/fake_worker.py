"""A tiny stand-in for rastai_worker in the node's job tests (layout: rust-node-gateway-proto.md section 7.1).

It does what the real worker does at the edges the node sees, and nothing else: it reads job.json, serves
WorkerService (Hello and Control) on the job socket, registers with WorkerHostService with the job token in the
`rastai-job-token` metadata key (contracts 4.5), turns SIGTERM and SIGINT into exits, and exits with the code its
script names. Standard library plus rastai_proto and grpcio only, from the cpu environment.

The test writes a JSON script and starts this file through a stand-in environment whose `bin/python` runs
`python -I fake_worker.py --script <script.json> -m rastai_worker run --job-file <job.json>`. Script keys, all
optional:

- "register": false to never call Register (default true).
- "token": the token to register with instead of RASTAI_JOB_TOKEN (a wrong token).
- "protocol_version": what Hello reports (default 1).
- "exit_after_register": exit with this code right after registering.
- "control": {"STOP": {"exit": 0}, "CANCEL": "ignore", ...}; a Control value not named is answered and ignored.
- "on_sigterm", "on_sigint": {"exit": <code>} or "ignore" (defaults: exit 75 and exit 3, as the real worker).
- "register_again_on_sigusr1": true to call Register once more, with the same token, at each SIGUSR1 (a worker
  that reconnected to its node).

Everything it sees goes to `fake-worker-record.jsonl` in the run folder, one JSON object per line, so the test can
check what arrived and how: its process ids, its environment, Register's answer, each Control value and whether
the node sent the right token with it, and each signal.
"""

import json
import os
import signal
import sys
import threading
from concurrent import futures
from pathlib import Path

import grpc
from rastai_proto.node.v1.jobs_pb2 import JobSignal
from rastai_proto.worker.v1 import (
    worker_host_service_pb2,
    worker_host_service_pb2_grpc,
    worker_service_pb2,
    worker_service_pb2_grpc,
)

TOKEN_METADATA_KEY = "rastai-job-token"
TOKEN_VARIABLE = "RASTAI_JOB_TOKEN"
RECORD_FILE = "fake-worker-record.jsonl"
SIGNAL_NAME_PREFIX = "JOB_SIGNAL_"
UNIX_TARGET_PREFIX = "unix:"
DEFAULT_PROTOCOL_VERSION = 1
DEFAULT_SIGTERM_EXIT = 75
DEFAULT_SIGINT_EXIT = 3
# How long a reply may take to leave before the process exits after answering a Control call.
EXIT_GRACE_SECONDS = 1.0
REGISTER_TIMEOUT_SECONDS = 10.0
# grpcio sends a unix: target's path as the HTTP/2 :authority, which the node's server (tonic and h2) refuses as a
# protocol error; a plain host name is what it accepts. The real worker's host connection needs the same option.
CHANNEL_OPTIONS = [("grpc.default_authority", "localhost")]
SERVER_THREADS = 4
RASTAI_VERSION = "0.0.0-fake-worker"
IGNORE = "ignore"


class WorkerState:
    """What the threads share: the run folder for records, and the exit code once something asks for one."""

    def __init__(self, run_folder: Path):
        self.run_folder = run_folder
        self.exit_code = None
        self.exit_requested = threading.Event()
        self.register_again = threading.Event()
        self.lock = threading.Lock()

    def request_exit(self, code: int) -> None:
        with self.lock:
            if self.exit_code is None:
                self.exit_code = code
        self.exit_requested.set()


def record(state: WorkerState, entry: dict) -> None:
    with state.lock:
        with open(state.run_folder / RECORD_FILE, "a", encoding="utf-8") as file:
            file.write(json.dumps(entry) + "\n")


class FakeWorkerService(worker_service_pb2_grpc.WorkerServiceServicer):
    def __init__(self, state: WorkerState, launch: dict, script: dict, token: str):
        self.state = state
        self.launch = launch
        self.script = script
        self.token = token

    def Hello(self, request, context):  # noqa: N802 (gRPC name)
        token_ok = self.has_job_token(context)
        record(self.state, {"event": "hello", "token_ok": token_ok})
        if not token_ok:
            context.abort(grpc.StatusCode.UNAUTHENTICATED, "wrong job token")
        return worker_service_pb2.WorkerInfo(
            job_id=self.launch["job_id"],
            pid=os.getpid(),
            protocol_version=self.script.get("protocol_version", DEFAULT_PROTOCOL_VERSION),
            rastai_version=RASTAI_VERSION,
            environment_id=self.launch["environment_id"],
        )

    def Control(self, request, context):  # noqa: N802 (gRPC name)
        action = JobSignal.Name(request.action).removeprefix(SIGNAL_NAME_PREFIX)
        token_ok = self.has_job_token(context)
        record(self.state, {"event": "control", "action": action, "token_ok": token_ok})
        if not token_ok:
            context.abort(grpc.StatusCode.UNAUTHENTICATED, "wrong job token")
        answer = self.script.get("control", {}).get(action, IGNORE)
        if answer != IGNORE:
            self.state.request_exit(answer["exit"])
        return worker_service_pb2.ControlResponse()

    def has_job_token(self, context) -> bool:
        metadata = dict(context.invocation_metadata())
        return metadata.get(TOKEN_METADATA_KEY) == self.token


def parse_arguments(argv: list[str]) -> tuple[Path, Path]:
    script_path = Path(argv[argv.index("--script") + 1])
    job_file = Path(argv[argv.index("--job-file") + 1])
    return script_path, job_file


def install_signal_handlers(state: WorkerState, script: dict) -> None:
    handled = [
        (signal.SIGTERM, "SIGTERM", script.get("on_sigterm", {"exit": DEFAULT_SIGTERM_EXIT})),
        (signal.SIGINT, "SIGINT", script.get("on_sigint", {"exit": DEFAULT_SIGINT_EXIT})),
    ]
    for number, name, answer in handled:
        signal.signal(number, signal_handler(state, name, answer))
    if script.get("register_again_on_sigusr1", False):
        signal.signal(signal.SIGUSR1, register_again_handler(state))


def signal_handler(state: WorkerState, name: str, answer):
    def handle(_number, _frame):
        record(state, {"event": "signal", "name": name})
        if answer != IGNORE:
            state.request_exit(answer["exit"])

    return handle


def register_again_handler(state: WorkerState):
    # The main loop calls Register: a gRPC call made inside a signal handler could interrupt one already running.
    def handle(_number, _frame):
        record(state, {"event": "signal", "name": "SIGUSR1"})
        state.register_again.set()

    return handle


def record_start(state: WorkerState) -> None:
    pid = os.getpid()
    entry = {
        "event": "started",
        "pid": pid,
        "process_group_id": os.getpgid(0),
        "session_id": os.getsid(0),
        "environment": dict(os.environ),
    }
    record(state, entry)


def start_server(state: WorkerState, launch: dict, script: dict, token: str) -> grpc.Server:
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=SERVER_THREADS))
    service = FakeWorkerService(state, launch, script, token)
    worker_service_pb2_grpc.add_WorkerServiceServicer_to_server(service, server)
    server.add_insecure_port(UNIX_TARGET_PREFIX + launch["job_socket_path"])
    server.start()
    return server


def register(state: WorkerState, launch: dict, token: str) -> None:
    target = UNIX_TARGET_PREFIX + launch["worker_host_socket_path"]
    with grpc.insecure_channel(target, options=CHANNEL_OPTIONS) as channel:
        stub = worker_host_service_pb2_grpc.WorkerHostServiceStub(channel)
        request = worker_host_service_pb2.RegisterRequest(
            job_id=launch["job_id"], job_socket_path=launch["job_socket_path"]
        )
        response = stub.Register(request, metadata=[(TOKEN_METADATA_KEY, token)], timeout=REGISTER_TIMEOUT_SECONDS)
    record(state, {"event": "registered", "diagnostic_code": response.diagnostic.code})


def main(argv: list[str]) -> int:
    script_path, job_file = parse_arguments(argv)
    script = json.loads(script_path.read_text(encoding="utf-8"))
    launch = json.loads(job_file.read_text(encoding="utf-8"))
    state = WorkerState(Path(launch["run_folder"]))
    install_signal_handlers(state, script)
    record_start(state)
    own_token = os.environ.get(TOKEN_VARIABLE, "")
    server = start_server(state, launch, script, own_token)
    if script.get("register", True):
        register(state, launch, script.get("token", own_token))
        if "exit_after_register" in script:
            state.request_exit(script["exit_after_register"])
    # Wait in short steps, so the signal handlers (which Python runs on the main thread) get their turn.
    while not state.exit_requested.wait(timeout=0.1):
        if state.register_again.is_set():
            state.register_again.clear()
            register(state, launch, own_token)
    server.stop(grace=EXIT_GRACE_SECONDS).wait()
    return state.exit_code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
