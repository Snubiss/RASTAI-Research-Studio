"""The train job (rastai_worker/jobs/train.py; python layout 7.4; contracts 4.4 and 9.5), called in process as the
worker's edge calls it, with fixtures/jobs/textcnn_train/job.json: TextCNN trains and the records come in order;
the JobSpec's two JSON fields are read as TrainJobArgs and SettingsCheckpoint, and anything else is RRS-J2001 before
a record is written; a full disk at a checkpoint pauses the run until RESUME, which writes that checkpoint before the
next step and doesn't prepare the run again; a job with resume_from continues the run from its newest checkpoint that
loads, the same as a run that never stopped.

The whole job process, under a fake node, is in test_worker_server.py.
"""

import dataclasses
import json
import os
import shutil
from pathlib import Path

import pytest
import rastai_train.loop
import torch
from rastai_formats.generated.command_train_start import train_job_args_from_json
from rastai_plugin.catalog.generated import RRS_J2001, RRS_T0002, RRS_T2001, RRS_T2002
from rastai_train.errors import TrainError
from rastai_train.loop import STEPS_BETWEEN_METRIC_RECORDS
from rastai_train.recipes.classification import ClassificationRecipe
from rastai_worker.errors import WorkerError
from rastai_worker.event_payloads import job_spec_from_dict
from rastai_worker.job_file import read_job_file
from rastai_worker.jobs.train import (
    CUBLAS_WORKSPACE_SETTING,
    CUBLAS_WORKSPACE_VARIABLE,
    read_checkpoint_policy,
    run_job,
    training_device,
)

from rastai_plugin import JobControl, JobEvents, WorkerHost

FIXTURE_JOB_FILE = Path("jobs/textcnn_train/job.json")
# The fixture's two paths are relative to the repository root; the tests make them absolute.
START_PATH_FIELDS = ("design_path", "dataset_path")
JOB_FILE_NAME = "job.json"
RUN_FOLDER_NAME = "run"
# The host is never called by the train job: these only fill its constructor.
UNUSED_SOCKET = "unused.sock"
UNUSED_TOKEN = "ab" * 32
FIXTURE_MAX_STEPS = 40
METRIC_STEPS = [10, 20, 30, 40]
PREEMPT_AT_STEP = 20
CHECKPOINT_NOW_AT_STEP = 10
CHECKPOINT_FOLDER_PATTERN = "checkpoints/step-*"
DAMAGED_STEP_FOLDER = "checkpoints/step-00000030"
OPTIMIZER_FILE = "optimizer.safetensors"
RUN_STATE_CHANGED = "run.state_changed"
RUN_METRICS = "run.metrics"
CHECKPOINT_SAVED = "checkpoint.saved"
DIAGNOSTIC_REPORTED = "diagnostic.reported"
TRAIN_LOSS = "train.loss"


@pytest.fixture(autouse=True)
def keep_torch_global_state():
    """The job seeds the global generator and turns deterministic mode on; the other tests in this process must find
    both as they were."""
    deterministic = torch.are_deterministic_algorithms_enabled()
    generator_state = torch.get_rng_state()
    yield
    torch.use_deterministic_algorithms(deterministic)
    torch.set_rng_state(generator_state)


class RecordingEvents(JobEvents):
    """Keeps every durable event. `on_durable` lets a test act at a record, as a Control call or a signal would."""

    def __init__(self, on_durable=None) -> None:
        self.durable: list[tuple[str, dict]] = []
        self.on_durable = on_durable

    def emit_durable(self, event_type: str, payload: dict, blob_hashes: list[str] | None = None) -> None:
        self.durable.append((event_type, payload))
        if self.on_durable is not None:
            self.on_durable(event_type, payload)

    def emit_ephemeral(self, event_type: str, payload: dict) -> None:
        pass

    def types(self) -> list[str]:
        return [event_type for event_type, _payload in self.durable]

    def payloads(self, event_type: str) -> list[dict]:
        return [payload for kind, payload in self.durable if kind == event_type]

    def run_states(self) -> list[str]:
        return [payload["run_state"] for payload in self.payloads(RUN_STATE_CHANGED)]

    def train_losses(self) -> dict[int, float]:
        losses = {}
        for payload in self.payloads(RUN_METRICS):
            if TRAIN_LOSS in payload["values"]:
                losses[payload["step"]] = payload["values"][TRAIN_LOSS]
        return losses


def fixture_fields(repo_root: Path, fixtures_folder: Path) -> dict:
    fields = json.loads((fixtures_folder / FIXTURE_JOB_FILE).read_text(encoding="utf-8"))
    arguments = json.loads(fields["job_spec"]["arguments_json"])
    for name in START_PATH_FIELDS:
        arguments["start_arguments"][name] = str(repo_root / arguments["start_arguments"][name])
    fields["job_spec"]["arguments_json"] = json.dumps(arguments)
    return fields


def write_job(folder: Path, fields: dict, resume_from: str = "") -> WorkerHost:
    """job.json with the run folder and sockets under `folder`, and the host a worker would hand the entry."""
    run_folder = folder / RUN_FOLDER_NAME
    run_folder.mkdir(parents=True, exist_ok=True)
    fields = dict(fields, run_folder=str(run_folder), resume_from=resume_from)
    fields["job_socket_path"] = str(folder / UNUSED_SOCKET)
    fields["worker_host_socket_path"] = str(folder / UNUSED_SOCKET)
    job_file = run_folder / JOB_FILE_NAME
    job_file.write_text(json.dumps(fields), encoding="utf-8")
    return WorkerHost(
        str(folder / UNUSED_SOCKET), fields["job_id"], str(folder / UNUSED_SOCKET), UNUSED_TOKEN, job_file
    )


def with_arguments(fields: dict, change) -> dict:
    """The fields with `change(arguments)` applied to the decoded arguments_json."""
    arguments = json.loads(fields["job_spec"]["arguments_json"])
    change(arguments)
    job_spec = dict(fields["job_spec"], arguments_json=json.dumps(arguments))
    return dict(fields, job_spec=job_spec)


def with_job_spec_field(fields: dict, name: str, text: str) -> dict:
    return dict(fields, job_spec=dict(fields["job_spec"], **{name: text}))


def run_fixture_job(host: WorkerHost, events: RecordingEvents, control: JobControl | None = None) -> None:
    launch = read_job_file(host.job_file_path)
    run_job(launch.job_spec, host, events, control or JobControl())


@pytest.fixture
def fields(repo_root: Path, fixtures_folder: Path) -> dict:
    return fixture_fields(repo_root, fixtures_folder)


# ---- Training ----


def test_the_fixture_job_trains_textcnn_and_records_in_order(tmp_path: Path, fields: dict) -> None:
    host = write_job(tmp_path, fields)
    events = RecordingEvents()
    run_fixture_job(host, events)
    # preparing, then the loop's running; the final state is the edge's, never the job's.
    assert events.run_states() == ["RUN_STATE_PREPARING", "RUN_STATE_RUNNING"]
    assert events.types()[:2] == [RUN_STATE_CHANGED, RUN_STATE_CHANGED]
    assert sorted(events.train_losses()) == METRIC_STEPS
    # One checkpoint: the last step's, with its validation loss after it.
    saved = events.payloads(CHECKPOINT_SAVED)
    assert [payload["step"] for payload in saved] == [FIXTURE_MAX_STEPS]
    assert events.types()[-2:] == [CHECKPOINT_SAVED, RUN_METRICS]
    run_folder = tmp_path / RUN_FOLDER_NAME
    assert (run_folder / saved[0]["folder"] / "weights.safetensors").is_file()
    # No pickle anywhere: weights, optimizer and RNG are safetensors, the rest JSON.
    assert list(run_folder.rglob("*.pt")) == []
    assert list(run_folder.rglob("*.bin")) == []


def test_the_checkpoint_policy_is_the_job_specs_training_checkpoint_section(fields: dict) -> None:
    job_spec = read_fixture_job_spec(fields)
    settings = read_checkpoint_policy(job_spec).checkpoint_settings
    # The fixture gives keep_last and the budget; the interval takes its 11.2 default.
    assert (settings.keep_last, settings.interval_minutes, settings.preemption_budget_seconds) == (2, 15, 20)
    job_spec.checkpoint_policy_json = ""
    settings = read_checkpoint_policy(job_spec).checkpoint_settings
    assert (settings.keep_last, settings.interval_minutes, settings.preemption_budget_seconds) == (3, 15, 25)


def read_fixture_job_spec(fields: dict):
    return job_spec_from_dict(fields["job_spec"])


def set_value(path: list[str], value):
    def change(arguments: dict) -> None:
        target = arguments
        for name in path[:-1]:
            target = target[name]
        target[path[-1]] = value

    return change


def remove_value(name: str):
    def change(arguments: dict) -> None:
        del arguments[name]

    return change


BAD_ARGUMENT_CHANGES = {
    "no plan": remove_value("plan"),
    "no steps": set_value(["max_steps"], 0),
    "steps as a boolean": set_value(["max_steps"], True),
    "steps as text": set_value(["max_steps"], "40"),
    "a negative seed": set_value(["seed"], -1),
    "no micro-batch": set_value(["plan", "micro_batch"], 0),
    "lora": set_value(["plan", "method"], "lora"),
    "bf16": set_value(["plan", "dtype"], "bf16"),
    "two GPUs": set_value(["plan", "gpu_indices"], [0, 1]),
    "no design": set_value(["start_arguments", "design_path"], ""),
    "start arguments that aren't an object": set_value(["start_arguments"], "classification"),
    "an unknown key": set_value(["max_step"], 40),
    "an unknown plan key": set_value(["plan", "micro_batches"], 8),
    "an unknown start arguments key": set_value(["start_arguments", "dataset"], "rows.jsonl"),
}


@pytest.mark.parametrize("case", sorted(BAD_ARGUMENT_CHANGES))
def test_arguments_that_arent_a_train_job_args_are_j2001_before_any_record(
    case: str, tmp_path: Path, fields: dict
) -> None:
    host = write_job(tmp_path, with_arguments(fields, BAD_ARGUMENT_CHANGES[case]))
    events = RecordingEvents()
    with pytest.raises(WorkerError) as caught:
        run_fixture_job(host, events)
    assert caught.value.code == RRS_J2001
    assert "arguments_json" in caught.value.message
    assert events.durable == []


BAD_JSON_FIELDS = {
    "arguments that aren't JSON": ("arguments_json", "{not json"),
    "arguments that are a list": ("arguments_json", "[]"),
    "a policy that isn't JSON": ("checkpoint_policy_json", "keep_last=3"),
    "a policy keeping no checkpoint": ("checkpoint_policy_json", '{"keep_last": 0}'),
    "a policy with a text budget": ("checkpoint_policy_json", '{"preemption_budget_seconds": "25"}'),
    "a policy with a misspelt key": ("checkpoint_policy_json", '{"keep_lst": 1}'),
}


@pytest.mark.parametrize("case", sorted(BAD_JSON_FIELDS))
def test_job_spec_text_that_isnt_its_shape_is_j2001_before_any_record(case: str, tmp_path: Path, fields: dict) -> None:
    name, text = BAD_JSON_FIELDS[case]
    host = write_job(tmp_path, with_job_spec_field(fields, name, text))
    events = RecordingEvents()
    with pytest.raises(WorkerError) as caught:
        run_fixture_job(host, events)
    assert caught.value.code == RRS_J2001
    assert name in caught.value.message
    assert events.durable == []


def test_an_unknown_recipe_is_t0002(tmp_path: Path, fields: dict) -> None:
    host = write_job(tmp_path, with_arguments(fields, set_value(["start_arguments", "recipe"], "jepa")))
    with pytest.raises(WorkerError) as caught:
        run_fixture_job(host, RecordingEvents())
    assert caught.value.code == RRS_T0002
    assert "jepa" in caught.value.message and "classification" in caught.value.message


def test_a_job_file_of_another_job_is_j2001(tmp_path: Path, fields: dict) -> None:
    host = write_job(tmp_path, fields)
    launch = read_job_file(host.job_file_path)
    launch.job_spec.job_id = "job_01JA2B3C4D5E6F7G8H9J0K1M2Z"
    with pytest.raises(WorkerError) as caught:
        run_job(launch.job_spec, host, RecordingEvents(), JobControl())
    assert caught.value.code == RRS_J2001


def test_a_gpu_plan_sets_cublas_workspace_before_cuda(fields: dict, monkeypatch: pytest.MonkeyPatch) -> None:
    plan = train_job_args_from_json(json.loads(fields["job_spec"]["arguments_json"])).plan
    # setenv first, so undo removes the variable the job sets: a delenv of an absent variable records nothing to undo,
    # and the setting would leak into every later test and its subprocesses.
    monkeypatch.setenv(CUBLAS_WORKSPACE_VARIABLE, CUBLAS_WORKSPACE_SETTING)
    monkeypatch.delenv(CUBLAS_WORKSPACE_VARIABLE)
    assert training_device(plan) == "cpu"
    assert CUBLAS_WORKSPACE_VARIABLE not in os.environ
    # The node shows the job only its own GPUs, so its one GPU is cuda:0 whatever its physical index.
    gpu_plan = dataclasses.replace(plan, gpu_indices=[3])
    assert training_device(gpu_plan) == "cuda:0"
    assert os.environ[CUBLAS_WORKSPACE_VARIABLE] == CUBLAS_WORKSPACE_SETTING


# ---- A full disk at a checkpoint ----


def full_disk_once(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """Makes the loop's next checkpoint write fail as a full disk does (RRS-T2001), and the ones after it succeed.
    Returns the list of steps each failed write was for."""
    real_write = rastai_train.loop.write_checkpoint
    failed_steps = []

    def write_or_fail(run_folder, step, context, recipe_state, validation_loss):
        if not failed_steps:
            failed_steps.append(step)
            raise TrainError(RRS_T2001, f"No space left to write the checkpoint of step {step}.")
        return real_write(run_folder, step, context, recipe_state, validation_loss)

    monkeypatch.setattr(rastai_train.loop, "write_checkpoint", write_or_fail)
    return failed_steps


def count_prepare_calls(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """Counts the classification recipe's prepare calls; returns a list that grows by one per call."""
    real_prepare = ClassificationRecipe.prepare
    calls = []

    def counted_prepare(recipe, context) -> None:
        calls.append(1)
        real_prepare(recipe, context)

    monkeypatch.setattr(ClassificationRecipe, "prepare", counted_prepare)
    return calls


def resume_when_paused(control: JobControl):
    """Clears `paused` when the run writes `paused`: the user freed space and sent RESUME (control.py's RESUME clears
    the flag)."""

    def resume(event_type: str, payload: dict) -> None:
        if event_type == RUN_STATE_CHANGED and payload["run_state"] == "RUN_STATE_PAUSED":
            assert control.paused
            control.paused = False

    return resume


def test_a_full_disk_at_a_checkpoint_pauses_the_run_until_resume(
    tmp_path: Path, fields: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    failed_steps = full_disk_once(monkeypatch)
    prepare_calls = count_prepare_calls(monkeypatch)
    control = JobControl()
    events = RecordingEvents(resume_when_paused(control))
    run_fixture_job(write_job(tmp_path, fields), events, control)
    assert failed_steps == [FIXTURE_MAX_STEPS]
    # The run went on in memory: the recipe was prepared once, not again after RESUME.
    assert len(prepare_calls) == 1
    assert events.run_states() == ["RUN_STATE_PREPARING", "RUN_STATE_RUNNING", "RUN_STATE_PAUSED", "RUN_STATE_RUNNING"]
    diagnostic = events.payloads(DIAGNOSTIC_REPORTED)[0]["diagnostic"]
    assert diagnostic["code"] == RRS_T2001
    # The run went on from the step it stopped at: the same last step's checkpoint, written after RESUME.
    assert [payload["step"] for payload in events.payloads(CHECKPOINT_SAVED)] == [FIXTURE_MAX_STEPS]
    assert events.types().index(DIAGNOSTIC_REPORTED) < events.types().index(CHECKPOINT_SAVED)
    assert not list((tmp_path / RUN_FOLDER_NAME).rglob("*.partial"))


def step_records(events: RecordingEvents) -> list[tuple[str, int]]:
    """The run.metrics and checkpoint.saved records in order, each as its type and step."""
    records = []
    for event_type, payload in events.durable:
        if event_type in (RUN_METRICS, CHECKPOINT_SAVED):
            records.append((event_type, payload["step"]))
    return records


def checkpoint_now_at(control: JobControl, step: int):
    """Sets checkpoint_requested, as Control CHECKPOINT_NOW does, when the metrics of `step` are recorded, and
    resumes the run when it pauses."""
    resume = resume_when_paused(control)

    def checkpoint_now(event_type: str, payload: dict) -> None:
        if event_type == RUN_METRICS and payload["step"] == step and TRAIN_LOSS in payload["values"]:
            control.checkpoint_requested = True
        resume(event_type, payload)

    return checkpoint_now


def test_a_mid_run_checkpoint_a_full_disk_refused_is_written_after_resume(
    tmp_path: Path, fields: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    uninterrupted = RecordingEvents()
    run_fixture_job(write_job(tmp_path / "uninterrupted", fields), uninterrupted)
    failed_steps = full_disk_once(monkeypatch)
    prepare_calls = count_prepare_calls(monkeypatch)
    control = JobControl()
    events = RecordingEvents(checkpoint_now_at(control, CHECKPOINT_NOW_AT_STEP))
    run_fixture_job(write_job(tmp_path / "paused", fields), events, control)
    assert failed_steps == [CHECKPOINT_NOW_AT_STEP]
    assert events.run_states() == ["RUN_STATE_PREPARING", "RUN_STATE_RUNNING", "RUN_STATE_PAUSED", "RUN_STATE_RUNNING"]
    # CHECKPOINT_NOW's checkpoint isn't lost: RESUME writes it, of the step it was asked at, before the next step.
    saved_steps = [payload["step"] for payload in events.payloads(CHECKPOINT_SAVED)]
    assert saved_steps == [CHECKPOINT_NOW_AT_STEP, FIXTURE_MAX_STEPS]
    assert events.types().index(DIAGNOSTIC_REPORTED) < events.types().index(CHECKPOINT_SAVED)
    # Step 10's training metrics, its checkpoint and validation loss, and only then step 20's metrics.
    assert step_records(events)[:4] == [
        (RUN_METRICS, CHECKPOINT_NOW_AT_STEP),
        (CHECKPOINT_SAVED, CHECKPOINT_NOW_AT_STEP),
        (RUN_METRICS, CHECKPOINT_NOW_AT_STEP),
        (RUN_METRICS, CHECKPOINT_NOW_AT_STEP + STEPS_BETWEEN_METRIC_RECORDS),
    ]
    assert len(prepare_calls) == 1
    # Pausing changed nothing the run computes: its losses are the uninterrupted run's, bit for bit on the CPU.
    assert events.train_losses() == uninterrupted.train_losses()


def test_a_cancel_while_paused_ends_without_training_again(
    tmp_path: Path, fields: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    full_disk_once(monkeypatch)
    control = JobControl()

    def cancel_when_paused(event_type: str, payload: dict) -> None:
        if event_type == RUN_STATE_CHANGED and payload["run_state"] == "RUN_STATE_PAUSED":
            control.cancel_requested = True

    events = RecordingEvents(cancel_when_paused)
    run_fixture_job(write_job(tmp_path, fields), events, control)
    assert events.run_states()[-1] == "RUN_STATE_PAUSED"
    assert events.payloads(CHECKPOINT_SAVED) == []


def test_a_preemption_whose_checkpoint_hits_a_full_disk_fails_instead_of_pausing(
    tmp_path: Path, fields: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    full_disk_once(monkeypatch)
    control = JobControl()
    events = RecordingEvents(preempt_at(control, PREEMPT_AT_STEP))
    with pytest.raises(TrainError) as caught:
        run_fixture_job(write_job(tmp_path, fields), events, control)
    assert caught.value.code == RRS_T2001
    assert "RUN_STATE_PAUSED" not in events.run_states()


# ---- Resume ----


def preempt_at(control: JobControl, step: int):
    """Sets preempt_requested, as SIGTERM does, when the metrics of `step` are recorded."""

    def preempt(event_type: str, payload: dict) -> None:
        if event_type == RUN_METRICS and payload["step"] == step:
            control.preempt_requested = True

    return preempt


def preempt_then_resume(tmp_path: Path, fields: dict) -> tuple[RecordingEvents, RecordingEvents]:
    """A first job preempted at PREEMPT_AT_STEP, then a second job of the same run with resume_from set."""
    first_control = JobControl()
    first = RecordingEvents(preempt_at(first_control, PREEMPT_AT_STEP))
    run_fixture_job(write_job(tmp_path, fields), first, first_control)
    saved = first.payloads(CHECKPOINT_SAVED)
    second = RecordingEvents()
    run_fixture_job(write_job(tmp_path, fields, resume_from=saved[-1]["folder"]), second)
    return first, second


def test_a_job_with_resume_from_continues_the_run_as_if_it_never_stopped(tmp_path: Path, fields: dict) -> None:
    uninterrupted = RecordingEvents()
    run_fixture_job(write_job(tmp_path / "uninterrupted", fields), uninterrupted)
    first, second = preempt_then_resume(tmp_path / "preempted", fields)
    # The SIGTERM checkpoint is of the step it stopped at.
    assert [payload["step"] for payload in first.payloads(CHECKPOINT_SAVED)] == [PREEMPT_AT_STEP]
    # The second job picks up after it and ends at the run's last step.
    assert sorted(second.train_losses()) == [30, 40]
    assert [payload["step"] for payload in second.payloads(CHECKPOINT_SAVED)] == [FIXTURE_MAX_STEPS]
    # Bit for bit on the CPU: the resumed losses are the uninterrupted run's (the resume contract, M1-38).
    resumed_losses = first.train_losses()
    resumed_losses.update(second.train_losses())
    assert resumed_losses == uninterrupted.train_losses()


def test_a_damaged_newer_checkpoint_is_reported_and_passed(tmp_path: Path, fields: dict) -> None:
    first_control = JobControl()
    first = RecordingEvents(preempt_at(first_control, PREEMPT_AT_STEP))
    run_fixture_job(write_job(tmp_path, fields), first, first_control)
    run_folder = tmp_path / RUN_FOLDER_NAME
    good_folder = run_folder / first.payloads(CHECKPOINT_SAVED)[0]["folder"]
    damaged_folder = run_folder / DAMAGED_STEP_FOLDER
    shutil.copytree(good_folder, damaged_folder)
    optimizer_bytes = bytearray((damaged_folder / OPTIMIZER_FILE).read_bytes())
    optimizer_bytes[:8] = b"\xff" * 8
    (damaged_folder / OPTIMIZER_FILE).write_bytes(bytes(optimizer_bytes))
    second = RecordingEvents()
    run_fixture_job(write_job(tmp_path, fields, resume_from=DAMAGED_STEP_FOLDER), second)
    diagnostics = [payload["diagnostic"] for payload in second.payloads(DIAGNOSTIC_REPORTED)]
    assert [diagnostic["code"] for diagnostic in diagnostics] == [RRS_T2002]
    # It went on from the good checkpoint and kept the damaged one on disk for inspection.
    assert sorted(second.train_losses()) == [30, 40]
    assert damaged_folder.is_dir()
    assert sorted(path.name for path in run_folder.glob(CHECKPOINT_FOLDER_PATTERN)) == [
        "step-00000020",
        "step-00000030",
        "step-00000040",
    ]
