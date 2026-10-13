"""kill -9 of a worker, and the next job of its run (flows/platform.md 6.2 and 6.5 rows 11 and 12; python layout 7.9).

- **Mid-append** (row 11, the worker's half): a worker killed halfway through a frame leaves a torn tail. The next
  job of the run opens the log as its writer, cuts the torn bytes off, reports it (`RRS-S2001`, info) and continues
  the sequence from the last whole record. M1 has no `train.resume` command yet (M5), so the test starts that job
  as the node would: the same run folder, `resume_from` set.
- **Mid-checkpoint** (row 12): a worker killed while its checkpoint is still `step-NNNNNNNN.partial/` leaves no
  visible checkpoint, and the next job resumes from the newest complete one.

Each run starts with a job that trains to its first metrics and is preempted (SIGTERM), so the run has a checkpoint
to continue from. The kills land at named points: `RASTAI_TEST_PAUSE_AT` (rastai_worker/pause_points.py) for the
frame, and for the checkpoint, which rastai_train writes, a launcher that stops the worker once the `.partial`
folder's files are synced, before the rename.
"""

import os
import signal
from pathlib import Path

import pytest
from rastai_plugin.catalog.generated import RRS_S2001
from rastai_proto.common.v1.states_pb2 import RUN_STATE_COMPLETED
from rastai_worker.exit_codes import EXIT_OK, EXIT_PREEMPTED

from worker_support import (
    JobFolders,
    kill_9,
    read_log,
    sequences,
    start_stand_in_node,
    start_worker,
    wait_for_exit,
    wait_until_stopped,
    write_job_file,
)

pytestmark = pytest.mark.posix_only

# One run, three jobs.
FIRST_JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2A"
SECOND_JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2B"
THIRD_JOB_ID = "job_01JA2B3C4D5E6F7G8H9J0K1M2C"
MAX_STEPS = 60
# Any non-empty text: the worker continues from the run's newest complete checkpoint, whatever this says.
RESUME_MARKER = "latest"
AFTER_METRICS_FRAME = "worker.event_log.after_metrics_frame"
MID_METRICS_FRAME = "worker.event_log.mid_metrics_frame"
CHECKPOINTS_FOLDER = "checkpoints"
PARTIAL_SUFFIX = ".partial"
# `python -m rastai_worker`, with rastai_train's sync of a `.partial` checkpoint folder followed by a stop: the
# moment before the rename, when a kill must leave no visible checkpoint.
STOPPING_LAUNCHER_MODULE = "rastai_test_worker_stopping_mid_checkpoint"
STOPPING_LAUNCHER_SOURCE = """
import os
import signal
import sys

from rastai_train import checkpoints
from rastai_worker.__main__ import main

sync_folder_first = checkpoints.sync_folder


def sync_then_stop(folder):
    sync_folder_first(folder)
    if str(folder).endswith(".partial"):
        os.kill(os.getpid(), signal.SIGSTOP)


checkpoints.sync_folder = sync_then_stop
sys.exit(main(sys.argv[1:]))
"""


@pytest.fixture
def job_files(folders: JobFolders, repo_root: Path, fixtures_folder: Path):
    """Writes job.json for one job of the run."""

    def write(job_id: str, resume_from: str = "") -> Path:
        job = {"job_id": job_id, "max_steps": MAX_STEPS, "resume_from": resume_from}
        return write_job_file(folders, repo_root, fixtures_folder, job)

    return write


def preempt_after_first_metrics(started: list, folders: JobFolders, job_files) -> None:
    """The run's first job: it trains to its first metrics, then SIGTERM preempts it with a checkpoint (exit 75)."""
    first = start_worker(started, folders, job_files(FIRST_JOB_ID), pause_at=AFTER_METRICS_FRAME)
    wait_until_stopped(first, folders)
    os.killpg(first.pid, signal.SIGTERM)
    os.killpg(first.pid, signal.SIGCONT)
    assert wait_for_exit(first) == EXIT_PREEMPTED, folders.stderr_path.read_text(encoding="utf-8")


def complete_checkpoints(folders: JobFolders) -> list[str]:
    folder = folders.run_folder / CHECKPOINTS_FOLDER
    return sorted(entry.name for entry in folder.iterdir() if not entry.name.endswith(PARTIAL_SUFFIX))


def partial_checkpoints(folders: JobFolders) -> list[str]:
    folder = folders.run_folder / CHECKPOINTS_FOLDER
    return sorted(entry.name for entry in folder.iterdir() if entry.name.endswith(PARTIAL_SUFFIX))


def final_run_state(records: list) -> int:
    return records[-1].payload.run_state_changed.run_state


def metric_steps(records: list) -> list[int]:
    return [record.payload.run_metrics.step for record in records if record.event_type == "run.metrics"]


def test_the_next_job_cuts_a_torn_tail_off_and_continues_the_sequence(
    folders: JobFolders, job_files, started: list
) -> None:
    start_stand_in_node(started, folders)
    preempt_after_first_metrics(started, folders, job_files)
    second = start_worker(started, folders, job_files(SECOND_JOB_ID, RESUME_MARKER), pause_at=MID_METRICS_FRAME)
    wait_until_stopped(second, folders)
    torn = read_log(folders.event_log_path)
    assert torn.torn_bytes > 0
    kill_9(second)

    third = start_worker(started, folders, job_files(THIRD_JOB_ID, RESUME_MARKER))
    assert wait_for_exit(third) == EXIT_OK, folders.stderr_path.read_text(encoding="utf-8")
    after = read_log(folders.event_log_path)
    assert after.torn_bytes == 0
    assert sequences(after.records) == list(range(1, len(after.records) + 1))
    # Every whole record stays as it was; the third job's first record follows the last of them.
    assert after.records[: len(torn.records)] == torn.records
    continued = after.records[len(torn.records)]
    assert (continued.job_id, continued.sequence) == (THIRD_JOB_ID, torn.records[-1].sequence + 1)
    assert continued.event_type == "diagnostic.reported"
    assert continued.payload.diagnostic_reported.diagnostic.code == RRS_S2001
    assert final_run_state(after.records) == RUN_STATE_COMPLETED


def test_a_checkpoint_cut_off_before_its_rename_is_never_resumed_from(
    folders: JobFolders, job_files, started: list
) -> None:
    (folders.socket_folder / f"{STOPPING_LAUNCHER_MODULE}.py").write_text(STOPPING_LAUNCHER_SOURCE, encoding="utf-8")
    start_stand_in_node(started, folders)
    preempt_after_first_metrics(started, folders, job_files)
    kept = complete_checkpoints(folders)
    assert len(kept) == 1
    # The second job trains to its last step and stops inside its final checkpoint, before the rename.
    second_job = job_files(SECOND_JOB_ID, RESUME_MARKER)
    second = start_worker(started, folders, second_job, module=STOPPING_LAUNCHER_MODULE)
    wait_until_stopped(second, folders)
    assert partial_checkpoints(folders) != []
    kill_9(second)
    assert complete_checkpoints(folders) == kept

    third = start_worker(started, folders, job_files(THIRD_JOB_ID, RESUME_MARKER))
    assert wait_for_exit(third) == EXIT_OK, folders.stderr_path.read_text(encoding="utf-8")
    records = read_log(folders.event_log_path).records
    # Had it resumed from the cut-off checkpoint (the last step), it would have had nothing left to train.
    third_steps = metric_steps([record for record in records if record.job_id == THIRD_JOB_ID])
    resumed_from_step = int(kept[0].split("-")[1])
    assert third_steps != [] and min(third_steps) > resumed_from_step
    assert max(third_steps) == MAX_STEPS
    assert final_run_state(records) == RUN_STATE_COMPLETED
    assert partial_checkpoints(folders) == []
