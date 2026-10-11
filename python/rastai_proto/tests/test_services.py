"""The gRPC services of rastai.node.v1 and rastai.worker.v1 (layout: rust-node-gateway-proto.md sections 3.4 and 3.6).

Each service has exactly the RPCs of the M1 skeleton, every generated servicer registers on a grpcio server, and
WorkerService, the one service Python serves (rastai_worker), answers a real call over a Unix socket. The Rust test
crates/rastai-proto/tests/services.rs builds the tonic side of the same services.
"""

from collections.abc import Iterator
from concurrent import futures
from pathlib import Path

import grpc
import pytest
from rastai_proto.events.v1.payloads_pb2 import JobProgress
from rastai_proto.node.v1 import (
    node_hub_service_pb2,
    node_hub_service_pb2_grpc,
    node_service_pb2,
    node_service_pb2_grpc,
)
from rastai_proto.node.v1.jobs_pb2 import JOB_KIND_TRAIN
from rastai_proto.node.v1.live_pb2 import LiveEvent, WatchLiveRequest
from rastai_proto.worker.v1 import (
    worker_host_service_pb2,
    worker_host_service_pb2_grpc,
    worker_service_pb2,
    worker_service_pb2_grpc,
)
from rastai_proto.worker.v1.worker_service_pb2 import HelloRequest, WorkerInfo

# Service full name -> its RPCs as (name, request type, response type, server streaming). Contracts 4.4 and 4.5
# list every RPC each service will have; the M1 skeleton has only these, and the rest arrive with the milestone
# that uses them (node layout 3.6), each in its own contract PR that updates this table.
SKELETON_RPCS = {
    "rastai.node.v1.NodeService": [
        ("Handshake", "rastai.node.v1.HandshakeRequest", "rastai.node.v1.HandshakeResponse", False),
        ("GetHardware", "rastai.node.v1.GetHardwareRequest", "rastai.node.v1.GetHardwareResponse", False),
        ("StartJob", "rastai.node.v1.JobSpec", "rastai.node.v1.JobInfo", False),
        ("SignalJob", "rastai.node.v1.SignalJobRequest", "rastai.node.v1.SignalJobResponse", False),
        ("ListJobs", "rastai.node.v1.ListJobsRequest", "rastai.node.v1.ListJobsResponse", False),
        ("WatchRunEvents", "rastai.node.v1.WatchRunEventsRequest", "rastai.events.v1.EventRecord", True),
        ("WatchLive", "rastai.node.v1.WatchLiveRequest", "rastai.node.v1.LiveEvent", True),
    ],
    "rastai.node.v1.NodeHubService": [
        ("Heartbeat", "rastai.node.v1.HeartbeatRequest", "rastai.node.v1.HeartbeatResponse", False),
        ("ExecuteCommand", "rastai.commands.v1.ExecuteRequest", "rastai.commands.v1.ExecuteResponse", False),
    ],
    "rastai.worker.v1.WorkerService": [
        ("Hello", "rastai.worker.v1.HelloRequest", "rastai.worker.v1.WorkerInfo", False),
        ("Control", "rastai.worker.v1.ControlRequest", "rastai.worker.v1.ControlResponse", False),
        ("WatchLive", "rastai.node.v1.WatchLiveRequest", "rastai.node.v1.LiveEvent", True),
    ],
    "rastai.worker.v1.WorkerHostService": [
        ("Register", "rastai.worker.v1.RegisterRequest", "rastai.worker.v1.RegisterResponse", False),
    ],
}

# Service full name -> (the message module that declares it, its generated servicer base class, the generated
# function that registers a servicer on a grpcio server).
GENERATED_SERVICES = {
    "rastai.node.v1.NodeService": (
        node_service_pb2,
        node_service_pb2_grpc.NodeServiceServicer,
        node_service_pb2_grpc.add_NodeServiceServicer_to_server,
    ),
    "rastai.node.v1.NodeHubService": (
        node_hub_service_pb2,
        node_hub_service_pb2_grpc.NodeHubServiceServicer,
        node_hub_service_pb2_grpc.add_NodeHubServiceServicer_to_server,
    ),
    "rastai.worker.v1.WorkerService": (
        worker_service_pb2,
        worker_service_pb2_grpc.WorkerServiceServicer,
        worker_service_pb2_grpc.add_WorkerServiceServicer_to_server,
    ),
    "rastai.worker.v1.WorkerHostService": (
        worker_host_service_pb2,
        worker_host_service_pb2_grpc.WorkerHostServiceServicer,
        worker_host_service_pb2_grpc.add_WorkerHostServiceServicer_to_server,
    ),
}

# One handler thread is enough for a test that makes one call at a time.
SERVER_THREADS = 1
# Long enough for a slow CI machine to start a server and answer one call.
CALL_TIMEOUT_SECONDS = 10
JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2Q"
# The version the test worker reports in Hello; any value works, so this is not contracts' WORKER_PROTOCOL_VERSION.
HELLO_PROTOCOL_VERSION = 1


def service_rpcs(service_name: str) -> list[tuple[str, str, str, bool]]:
    """The RPCs of one service as the generated descriptor declares them, in the .proto file's order."""
    message_module = GENERATED_SERVICES[service_name][0]
    short_name = service_name.rsplit(".", 1)[1]
    service = message_module.DESCRIPTOR.services_by_name[short_name]
    rpcs = []
    for method in service.methods:
        rpcs.append((method.name, method.input_type.full_name, method.output_type.full_name, method.server_streaming))
    return rpcs


@pytest.mark.parametrize("service_name", sorted(SKELETON_RPCS))
def test_each_service_has_the_skeleton_rpcs(service_name: str) -> None:
    assert service_rpcs(service_name) == SKELETON_RPCS[service_name]


@pytest.mark.parametrize("service_name", sorted(GENERATED_SERVICES))
def test_every_servicer_registers_on_a_grpc_server(service_name: str) -> None:
    _, servicer_class, add_servicer_to_server = GENERATED_SERVICES[service_name]
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=SERVER_THREADS))
    try:
        add_servicer_to_server(servicer_class(), server)
    finally:
        server.stop(grace=None)


class StubWorker(worker_service_pb2_grpc.WorkerServiceServicer):
    """A worker that answers Hello and streams one progress event; Control stays the generated UNIMPLEMENTED."""

    def Hello(self, request: HelloRequest, context: grpc.ServicerContext) -> WorkerInfo:  # noqa: N802 (gRPC name)
        return WorkerInfo(job_id=JOB_ID, kind=JOB_KIND_TRAIN, protocol_version=HELLO_PROTOCOL_VERSION)

    def WatchLive(  # noqa: N802 (gRPC name)
        self, request: WatchLiveRequest, context: grpc.ServicerContext
    ) -> Iterator[LiveEvent]:
        yield LiveEvent(job_id=JOB_ID, job_progress=JobProgress(fraction=0.5, message="epoch 1 of 2"))


def serve_worker_on_unix_socket(socket_path: Path, worker: StubWorker) -> grpc.Server:
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=SERVER_THREADS))
    worker_service_pb2_grpc.add_WorkerServiceServicer_to_server(worker, server)
    server.add_insecure_port(f"unix:{socket_path}")
    server.start()
    return server


# The node reaches a worker the same way: a channel to jobs/<job_id>.sock (contracts 3.2).
@pytest.mark.posix_only
def test_worker_service_answers_over_a_unix_socket(tmp_path: Path) -> None:
    socket_path = tmp_path / "job.sock"
    server = serve_worker_on_unix_socket(socket_path, StubWorker())
    try:
        with grpc.insecure_channel(f"unix:{socket_path}") as channel:
            worker = worker_service_pb2_grpc.WorkerServiceStub(channel)
            info = worker.Hello(HelloRequest(), timeout=CALL_TIMEOUT_SECONDS)
            live_events = list(worker.WatchLive(WatchLiveRequest(), timeout=CALL_TIMEOUT_SECONDS))
    finally:
        server.stop(grace=None)

    assert info.job_id == JOB_ID
    assert info.protocol_version == HELLO_PROTOCOL_VERSION
    assert [event.WhichOneof("payload") for event in live_events] == ["job_progress"]
    assert live_events[0].job_progress.message == "epoch 1 of 2"
