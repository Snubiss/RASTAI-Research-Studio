"""kill -9 of the node while its worker trains (flows/platform.md 2.3 and 6.5; python layout 7.9; the weeks 3–4
exit criterion): the worker runs in its own session, so it outlives the node. It keeps training and writing its log
while no node is there, and when a new node serves worker-host.sock again it registers again with the same token,
which the node checks against the job's `job_token_hash` before it calls `Hello`. The log has no gap: whatever the
worker wrote while no node watched is there for the daemon to read after its ack.

The node here is a stand-in process (worker_support.py), so the test can SIGKILL it; the real node's half of the
same story is crates/rastai-node/tests/reattach.rs, and the daemon's is the crash matrix's row 1.
"""

import os
import signal
from pathlib import Path

import grpc
import pytest
from rastai_proto.common.v1.states_pb2 import RUN_STATE_COMPLETED
from rastai_proto.node.v1.jobs_pb2 import JobSignal
from rastai_proto.worker.v1 import worker_service_pb2, worker_service_pb2_grpc
from rastai_worker.exit_codes import EXIT_OK

from worker_support import (
    JOB_TOKEN,
    TOKEN_METADATA_KEY,
    JobFolders,
    kill_9,
    read_log,
    registrations,
    sequences,
    start_stand_in_node,
    start_worker,
    wait_for_exit,
    wait_until,
    wait_until_stopped,
    write_job_file,
)

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
# Long enough that the job is still training when the test stops it.
LONG_RUN_STEPS = 100_000
AFTER_METRICS_FRAME = "worker.event_log.after_metrics_frame"
CALL_TIMEOUT_SECONDS = 30.0
UNIX_TARGET_PREFIX = "unix:"


def last_sequence(folders: JobFolders) -> int:
    return read_log(folders.event_log_path).records[-1].sequence


def hello_then_stop(job_socket_path: str) -> worker_service_pb2.WorkerInfo:
    """What the node does once the job registered again: Hello with the job token, then (the test's own ending) STOP."""
    metadata = [(TOKEN_METADATA_KEY, JOB_TOKEN)]
    with grpc.insecure_channel(UNIX_TARGET_PREFIX + job_socket_path) as channel:
        stub = worker_service_pb2_grpc.WorkerServiceStub(channel)
        info = stub.Hello(worker_service_pb2.HelloRequest(), metadata=metadata, timeout=CALL_TIMEOUT_SECONDS)
        stop = worker_service_pb2.ControlRequest(action=JobSignal.JOB_SIGNAL_STOP)
        stopped = stub.Control(stop, metadata=metadata, timeout=CALL_TIMEOUT_SECONDS)
        assert not stopped.HasField("diagnostic")
    return info


def test_the_worker_trains_through_a_node_kill_and_registers_again(
    folders: JobFolders, repo_root: Path, fixtures_folder: Path, started: list
) -> None:
    first_node = start_stand_in_node(started, folders)
    job = {"job_id": JOB_ID, "max_steps": LONG_RUN_STEPS}
    worker = start_worker(
        started, folders, write_job_file(folders, repo_root, fixtures_folder, job), pause_at=AFTER_METRICS_FRAME
    )
    wait_until_stopped(worker, folders)
    assert len(registrations(folders)) == 1
    kill_9(first_node)
    logged_at_kill = last_sequence(folders)

    # No node: the worker trains on and writes records nobody reads.
    os.kill(worker.pid, signal.SIGCONT)
    wait_until(lambda: last_sequence(folders) > logged_at_kill, "the worker to write while no node runs", folders)
    start_stand_in_node(started, folders)
    wait_until(lambda: len(registrations(folders)) == 2, "the worker to register with the new node", folders)
    first, again = registrations(folders)
    assert again == first
    assert again["hash_matches"]

    info = hello_then_stop(again["job_socket_path"])
    assert (info.job_id, info.pid) == (JOB_ID, worker.pid)
    assert wait_for_exit(worker) == EXIT_OK, folders.stderr_path.read_text(encoding="utf-8")
    records = read_log(folders.event_log_path).records
    assert sequences(records) == list(range(1, len(records) + 1))
    assert {record.job_id for record in records} == {JOB_ID}
    final = records[-1].payload.run_state_changed
    assert (final.run_state, final.reason) == (RUN_STATE_COMPLETED, "stopped")
