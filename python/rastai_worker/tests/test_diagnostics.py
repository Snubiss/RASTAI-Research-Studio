"""The worker's edge (diagnostics.py; contracts 9.1 and 10; python layout 7.5 and 7.9): how the entry ends becomes
the final records and the exit code; package errors keep their code, a GPU out-of-memory error is RRS-H3007 and an
unexpected exception is RRS-J1005 with the traceback in stderr.log only; an event log another process holds exits 5
with RRS-J3006 in stderr.log and the log's bytes unchanged.
"""

import json
import os
import struct
import subprocess
import sys
from pathlib import Path

import pytest
import torch
from rastai_plugin.catalog.generated import RRS_H3007, RRS_J1005, RRS_J3006, RRS_P0103
from rastai_proto.common.v1.diagnostic_pb2 import SEVERITY_ERROR
from rastai_proto.common.v1.states_pb2 import (
    RUN_STATE_CANCELLED,
    RUN_STATE_COMPLETED,
    RUN_STATE_FAILED,
    RUN_STATE_PREEMPTED,
)
from rastai_proto.events.v1.event_record_pb2 import EventRecord
from rastai_proto.events.v1.payloads_pb2 import EventPayload
from rastai_worker.diagnostics import diagnostic_from_error, run_entry_with_diagnostics
from rastai_worker.event_log import EVENT_LOG_HEADER, FRAME_HEADER_BYTES, EventLogWriter
from rastai_worker.event_payloads import payload_from_dict
from rastai_worker.exit_codes import (
    EXIT_CANCELLED,
    EXIT_EVENT_LOG_LOCKED,
    EXIT_FAILED,
    EXIT_OK,
    EXIT_PREEMPTED,
)
from rastai_worker.job_events import WorkerJobEvents
from rastai_worker.job_file import WorkerLaunch, read_job_file
from rastai_worker.live_stream import LiveStream

from rastai_plugin import JobControl, PluginError

pytestmark = pytest.mark.posix_only

JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2N"
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
# A detail only the traceback may carry: it must never reach the run's log.
SECRET_DETAIL = "detail-only-for-stderr"
FRAME_HEADER_FORMAT = "<II"


def write_launch(folder: Path, kind: str = "JOB_KIND_TRAIN", entry: str = "") -> WorkerLaunch:
    run_folder = folder / "run"
    run_folder.mkdir(exist_ok=True)
    fields = {
        "job_id": JOB_ID,
        "run_id": RUN_ID,
        "job_spec": {"job_id": JOB_ID, "run_id": RUN_ID, "kind": kind, "entry": entry},
        "job_socket_path": str(folder / "job.sock"),
        "worker_host_socket_path": str(folder / "worker-host.sock"),
        "run_folder": str(run_folder),
        "environment_id": "cpu",
    }
    job_file = run_folder / "job.json"
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return read_job_file(job_file)


def read_records(log_path: Path) -> list[EventRecord]:
    data = log_path.read_bytes()
    assert data.startswith(EVENT_LOG_HEADER)
    records = []
    offset = len(EVENT_LOG_HEADER)
    while offset < len(data):
        body_length, _checksum = struct.unpack_from(FRAME_HEADER_FORMAT, data, offset)
        body_start = offset + FRAME_HEADER_BYTES
        records.append(EventRecord.FromString(data[body_start : body_start + body_length]))
        offset = body_start + body_length
    return records


def run_entry(tmp_path: Path, entry, control: JobControl | None = None) -> tuple[int, list[EventRecord]]:
    """Runs `entry` under the edge with a real event log, and returns the exit code and the log's records."""
    launch = write_launch(tmp_path)
    control = control or JobControl()
    log = EventLogWriter(launch.event_log_path, RUN_ID, JOB_ID)
    events = WorkerJobEvents(log, LiveStream(), control, JOB_ID)
    try:
        exit_code = run_entry_with_diagnostics(entry, launch, None, events, control)
    finally:
        log.close()
    return exit_code, read_records(launch.event_log_path)


def final_state(records: list[EventRecord]) -> tuple[int, str]:
    change = records[-1].payload.run_state_changed
    assert records[-1].event_type == "run.state_changed"
    return change.run_state, change.reason


def raising_entry(error: BaseException):
    def entry(job, host, events, control) -> None:
        events.emit_durable("run.metrics", {"step": 1, "values": {"train.loss": 2.5}})
        raise error

    return entry


def test_an_unexpected_exception_is_j1005_with_the_traceback_in_stderr_only(tmp_path: Path, capsys) -> None:
    exit_code, records = run_entry(tmp_path, raising_entry(KeyError(SECRET_DETAIL)))
    assert exit_code == EXIT_FAILED
    assert [record.event_type for record in records] == ["run.metrics", "diagnostic.reported", "run.state_changed"]
    reported = records[1].payload.diagnostic_reported
    assert reported.diagnostic.code == RRS_J1005
    assert reported.diagnostic.severity == SEVERITY_ERROR
    assert reported.diagnostic.message == f"The worker of job {JOB_ID} stopped on an unexpected KeyError."
    assert reported.subject_id == JOB_ID
    assert final_state(records) == (RUN_STATE_FAILED, "")
    log_bytes = (tmp_path / "run" / "events.log").read_bytes()
    assert b"Traceback" not in log_bytes
    assert SECRET_DETAIL.encode() not in log_bytes
    stderr = capsys.readouterr().err
    assert "Traceback (most recent call last)" in stderr
    assert SECRET_DETAIL in stderr


def test_a_package_error_keeps_its_code(tmp_path: Path) -> None:
    error = PluginError(RRS_P0103, "register() of word_count raised ValueError")
    exit_code, records = run_entry(tmp_path, raising_entry(error))
    assert exit_code == EXIT_FAILED
    diagnostic = records[1].payload.diagnostic_reported.diagnostic
    assert (diagnostic.code, diagnostic.message) == (RRS_P0103, "register() of word_count raised ValueError")
    assert diagnostic.fix_text != ""


def test_codes_outside_the_catalog_become_j1005_except_a_plugins_own() -> None:
    plugin_error = PluginError("PLG-word_count-0001", "the word list is empty")
    assert diagnostic_from_error(plugin_error, JOB_ID)["code"] == "PLG-word_count-0001"
    made_up = PluginError("RRS-Z9999", "a code nobody registered")
    assert diagnostic_from_error(made_up, JOB_ID)["code"] == RRS_J1005


def test_a_gpu_out_of_memory_error_is_h3007() -> None:
    error = torch.OutOfMemoryError("CUDA out of memory. Tried to allocate 2.00 GiB\nmore lines")
    diagnostic = diagnostic_from_error(error, JOB_ID)
    assert diagnostic["code"] == RRS_H3007
    assert (
        diagnostic["message"]
        == "Out of GPU memory during the job's step: CUDA out of memory. Tried to allocate 2.00 GiB"
    )


def returning_entry(job, host, events, control) -> None:
    events.emit_durable("run.metrics", {"step": 1, "values": {"train.loss": 2.5}})


@pytest.mark.parametrize(
    ("flag_name", "expected_exit", "expected_state"),
    [
        ("", EXIT_OK, (RUN_STATE_COMPLETED, "")),
        ("stop_requested", EXIT_OK, (RUN_STATE_COMPLETED, "stopped")),
        ("cancel_requested", EXIT_CANCELLED, (RUN_STATE_CANCELLED, "")),
        ("preempt_requested", EXIT_PREEMPTED, (RUN_STATE_PREEMPTED, "")),
    ],
)
def test_a_returning_entry_ends_as_its_flags_say(
    tmp_path: Path, flag_name: str, expected_exit: int, expected_state: tuple
) -> None:
    control = JobControl()

    def entry(job, host, events, entry_control) -> None:
        returning_entry(job, host, events, entry_control)
        if flag_name:
            setattr(entry_control, flag_name, True)

    exit_code, records = run_entry(tmp_path, entry, control)
    assert exit_code == expected_exit
    assert final_state(records) == expected_state
    assert [record.event_type for record in records] == ["run.metrics", "run.state_changed"]


def test_a_job_preempted_before_it_began_doesnt_run(tmp_path: Path) -> None:
    calls = []
    exit_code, records = run_entry(
        tmp_path, lambda *arguments: calls.append(arguments), JobControl(preempt_requested=True)
    )
    assert calls == []
    assert exit_code == EXIT_PREEMPTED
    assert final_state(records) == (RUN_STATE_PREEMPTED, "")


def test_a_log_another_process_holds_exits_5_and_stays_unchanged(tmp_path: Path) -> None:
    """The single-writer backstop as the node sees it: a second `python -m rastai_worker run` for a run whose log is
    locked exits 5 before Register, with RRS-J3006 in its stderr.log, and the running copy's log and reserve keep
    every byte."""
    entries_folder = tmp_path / "entries"
    entries_folder.mkdir()
    (entries_folder / "rastai_test_entries.py").write_text("def run_job(job, host, events, control):\n    pass\n")
    launch = write_launch(tmp_path, "JOB_KIND_EVAL", "rastai_test_entries:run_job")
    running_copy = EventLogWriter(launch.event_log_path, RUN_ID, "job_running_copy")
    try:
        running_copy.append("run.metrics", running_metrics(), [], fsync_now=True)
        log_before = launch.event_log_path.read_bytes()
        reserve_before = running_copy.reserve_path.read_bytes()
        stderr_path = launch.run_folder / "stderr.log"
        exit_code = run_worker_process(launch.job_file, stderr_path, entries_folder)
        assert exit_code == EXIT_EVENT_LOG_LOCKED
        assert launch.event_log_path.read_bytes() == log_before
        assert running_copy.reserve_path.read_bytes() == reserve_before
        stderr_lines = stderr_path.read_text(encoding="utf-8").strip().splitlines()
        assert stderr_lines[-1].startswith(f"{RRS_J3006}: ")
        assert str(launch.event_log_path) in stderr_lines[-1]
    finally:
        running_copy.close()


def running_metrics() -> EventPayload:
    return payload_from_dict("run.metrics", {"step": 7, "values": {"train.loss": 1.5}})


def run_worker_process(job_file: Path, stderr_path: Path, entries_folder: Path) -> int:
    environment = dict(os.environ, RASTAI_JOB_TOKEN="ab" * 32, PYTHONPATH=str(entries_folder))
    with open(stderr_path, "a", encoding="utf-8") as stderr_file:
        finished = subprocess.run(
            [sys.executable, "-m", "rastai_worker", "run", "--job-file", str(job_file)],
            stdin=subprocess.DEVNULL,
            stderr=stderr_file,
            env=environment,
            timeout=60,
            check=False,
        )
    return finished.returncode
