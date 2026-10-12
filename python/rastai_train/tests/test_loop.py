"""The training loop (rastai_train/loop.py) on TextCNN and the tiny fixture: the loss falls; each JobControl flag
ends the run as the layout's table says (STOP: a final checkpoint; SIGTERM: a checkpoint inside the budget; CANCEL:
none); metrics carry their steps; checkpoint.saved comes after the rename; the retention rule runs as it goes."""

import math
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.command_train_start import TrainStartArgs
from rastai_plugin.catalog.generated import RRS_T3001

from rastai_plugin import Batch, JobControl, JobEvents, PluginRegistry, StepResult, TrainingContext
from rastai_train import (
    FIRST_PARTY_RECIPES,
    CheckpointPolicy,
    ClassificationRecipe,
    TrainError,
    TrainingSettings,
    build_training_context,
    list_complete_checkpoints,
    register_first_party,
    train,
)

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
TINY_DATASET = Path("fixtures/datasets/textcnn_tiny.jsonl")
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2N"
# The event types a worker emits in M1 (contracts 7.3; rastai_worker's WORKER_EVENT_PAYLOAD_FIELDS).
M1_WORKER_EVENT_TYPES = {
    "run.state_changed",
    "run.metrics",
    "checkpoint.saved",
    "artifact.created",
    "diagnostic.reported",
}
STANDARD_METRICS = {"train.loss", "train.learning_rate", "train.grad_norm", "train.samples_seen"}


@pytest.fixture(autouse=True)
def keep_torch_global_state():
    """build_training_context seeds the global generator and turns deterministic mode on; the other packages' tests
    run in the same process and must find both as they were."""
    deterministic = torch.are_deterministic_algorithms_enabled()
    generator_state = torch.get_rng_state()
    yield
    torch.use_deterministic_algorithms(deterministic)
    torch.set_rng_state(generator_state)


class RecordingEvents(JobEvents):
    """Keeps every event; on checkpoint.saved it also notes whether the folder was complete at that moment."""

    def __init__(self, run_folder: Path) -> None:
        self.run_folder = run_folder
        self.durable: list[tuple[str, dict]] = []
        self.ephemeral: list[tuple[str, dict]] = []
        self.folders_complete_when_saved: list[bool] = []

    def emit_durable(self, event_type: str, payload: dict, blob_hashes: list[str] | None = None) -> None:
        if event_type == "checkpoint.saved":
            folder = self.run_folder / payload["folder"]
            partial = folder.with_name(folder.name + ".partial")
            self.folders_complete_when_saved.append(folder.is_dir() and not partial.exists())
        self.durable.append((event_type, payload))

    def emit_ephemeral(self, event_type: str, payload: dict) -> None:
        self.ephemeral.append((event_type, payload))

    def payloads(self, event_type: str) -> list[dict]:
        return [payload for kind, payload in self.durable if kind == event_type]


class FlagAfterStep(ClassificationRecipe):
    """Classification that sets one JobControl flag during training step `after_step`, as a signal or a Control
    call would arrive mid-step, and keeps every training loss."""

    def __init__(self, control: JobControl, flag: str, after_step: int) -> None:
        self.control = control
        self.flag = flag
        self.after_step = after_step
        self.losses: list[float] = []

    def training_step(self, batch: Batch) -> StepResult:
        result = super().training_step(batch)
        if self.model.training:
            self.losses.append(result.loss.item())
            if len(self.losses) == self.after_step:
                setattr(self.control, self.flag, True)
        return result


class NonFiniteOnSteps(ClassificationRecipe):
    """Classification whose loss is NaN on the given training steps (counted from 1)."""

    def __init__(self, non_finite_steps: set[int]) -> None:
        self.non_finite_steps = non_finite_steps
        self.training_steps = 0

    def training_step(self, batch: Batch) -> StepResult:
        result = super().training_step(batch)
        if not self.model.training:
            return result
        self.training_steps += 1
        if self.training_steps in self.non_finite_steps:
            return StepResult(loss=result.loss * math.nan, metrics=result.metrics)
        return result


def new_context(repo_root: Path, run_folder: Path, max_steps: int, **settings) -> TrainingContext:
    arguments = TrainStartArgs(
        recipe="classification",
        design_path=str(repo_root / TEXTCNN_TEMPLATE),
        dataset_path=str(repo_root / TINY_DATASET),
    )
    policy = CheckpointPolicy(
        interval_minutes=0,
        every_steps=settings.pop("every_steps", 0),
        keep_last=settings.pop("keep_last", 3),
        preemption_budget_seconds=settings.pop("preemption_budget_seconds", 25.0),
    )
    values = {"micro_batch": 16, "learning_rate": 5e-3, "warmup_steps": 2}
    values.update(settings)
    return build_training_context(
        arguments, RUN_ID, run_folder, TrainingSettings(max_steps=max_steps, checkpoint=policy, **values)
    )


def checkpoint_steps(context: TrainingContext) -> list[int]:
    return [step for step, _folder in list_complete_checkpoints(context.run_folder)]


def train_metric_records(events: RecordingEvents) -> list[dict]:
    return [payload for payload in events.payloads("run.metrics") if "train.learning_rate" in payload["values"]]


# ---- Learning ----


def test_textcnn_loss_falls_on_the_fixture(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 60, micro_batch=32, warmup_steps=5, every_steps=10)
    events = RecordingEvents(context.run_folder)

    outcome = train(context, ClassificationRecipe(), events, JobControl())

    assert outcome.reason == "completed"
    assert outcome.step == 60
    validation_losses = []
    for payload in events.payloads("run.metrics"):
        if "validation.loss" in payload["values"]:
            validation_losses.append(payload["values"]["validation.loss"])
    assert len(validation_losses) == 6
    assert validation_losses[-1] < 0.6 * validation_losses[0]
    train_losses = [payload["values"]["train.loss"] for payload in train_metric_records(events)]
    assert train_losses[-1] < train_losses[0]


def test_metrics_carry_their_steps_every_ten_steps_and_at_the_end(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 25, validation_ratio=0.0)
    events = RecordingEvents(context.run_folder)

    train(context, ClassificationRecipe(), events, JobControl())

    records = train_metric_records(events)
    assert [record["step"] for record in records] == [10, 20, 25]
    for record in records:
        assert STANDARD_METRICS <= set(record["values"])
        assert "classification.accuracy" in record["values"]
    assert records[-1]["values"]["train.samples_seen"] == 25 * 16
    assert [payload["fraction"] for kind, payload in events.ephemeral] == [0.4, 0.8, 1.0]
    assert events.durable[0] == ("run.state_changed", {"run_state": "RUN_STATE_RUNNING"})
    assert {kind for kind, _payload in events.durable} <= M1_WORKER_EVENT_TYPES


def test_gradient_accumulation_runs_its_micro_batches_per_step(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 3, micro_batch=8, gradient_accumulation=2)
    train(context, ClassificationRecipe(), RecordingEvents(context.run_folder), JobControl())
    assert context.progress.samples_seen == 3 * 2 * 8


# ---- How each flag ends a run ----


def test_stop_writes_a_final_checkpoint_at_the_next_step_boundary(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 50)
    control = JobControl()
    events = RecordingEvents(context.run_folder)

    outcome = train(context, FlagAfterStep(control, "stop_requested", 4), events, control)

    assert outcome.reason == "stopped"
    assert outcome.step == 4
    assert outcome.checkpoint_folder == context.run_folder / "checkpoints" / "step-00000004"
    assert checkpoint_steps(context) == [4]
    assert events.payloads("checkpoint.saved")[-1]["step"] == 4


def test_sigterm_writes_a_checkpoint_when_it_fits_the_budget(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 50)
    control = JobControl()
    events = RecordingEvents(context.run_folder)

    outcome = train(context, FlagAfterStep(control, "preempt_requested", 3), events, control)

    assert outcome.reason == "preempted"
    assert outcome.step == 3
    assert checkpoint_steps(context) == [3]
    assert outcome.checkpoint_folder == context.run_folder / "checkpoints" / "step-00000003"
    # The run's final state is the worker edge's to write (run.state_changed preempted, exit 75), and train.preempted
    # is an M5 event: the loop writes neither.
    assert events.durable[-1][0] != "run.state_changed"
    assert {kind for kind, _payload in events.durable} <= M1_WORKER_EVENT_TYPES


def test_sigterm_skips_the_checkpoint_when_the_last_one_took_longer_than_the_budget(
    repo_root: Path, tmp_path: Path
) -> None:
    # Any real write takes longer than 0 s, so after the periodic checkpoint at step 2 nothing fits the budget.
    context = new_context(repo_root, tmp_path / "run", 50, every_steps=2, preemption_budget_seconds=0.0)
    control = JobControl()

    outcome = train(
        context, FlagAfterStep(control, "preempt_requested", 3), RecordingEvents(context.run_folder), control
    )

    assert outcome.reason == "preempted"
    assert outcome.step == 3
    assert checkpoint_steps(context) == [2]
    assert outcome.checkpoint_folder == context.run_folder / "checkpoints" / "step-00000002"


def test_sigterm_is_read_before_stop_and_cancel(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 50)
    control = JobControl(stop_requested=True, cancel_requested=True)
    recipe = FlagAfterStep(control, "preempt_requested", 1)
    control.preempt_requested = True

    outcome = train(context, recipe, RecordingEvents(context.run_folder), control)
    assert outcome.reason == "preempted"
    assert outcome.step == 0
    # Nothing was trained, so there is nothing to save.
    assert checkpoint_steps(context) == []


def test_cancel_ends_without_a_new_checkpoint(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 50, every_steps=2)
    control = JobControl()

    outcome = train(
        context, FlagAfterStep(control, "cancel_requested", 3), RecordingEvents(context.run_folder), control
    )

    assert outcome.reason == "cancelled"
    assert outcome.step == 3
    assert checkpoint_steps(context) == [2]


def test_checkpoint_now_saves_once_and_training_goes_on(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 6)
    control = JobControl()

    outcome = train(
        context, FlagAfterStep(control, "checkpoint_requested", 2), RecordingEvents(context.run_folder), control
    )

    assert outcome.reason == "completed"
    assert checkpoint_steps(context) == [2, 6]
    assert control.checkpoint_requested is False


def test_a_paused_run_still_ends_when_asked(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 50)
    control = JobControl(paused=True, stop_requested=True)

    outcome = train(context, ClassificationRecipe(), RecordingEvents(context.run_folder), control)
    assert outcome.reason == "stopped"
    assert outcome.step == 0


# ---- Checkpoints during the run ----


def test_checkpoint_saved_comes_after_the_rename_with_every_file_hashed(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 6, every_steps=3)
    events = RecordingEvents(context.run_folder)

    train(context, ClassificationRecipe(), events, JobControl())

    saved = events.payloads("checkpoint.saved")
    assert [payload["step"] for payload in saved] == [3, 6]
    assert events.folders_complete_when_saved == [True, True]
    for payload in saved:
        assert payload["checkpoint_id"].startswith("ckpt_")
        assert payload["folder"] == f"checkpoints/step-{payload['step']:08d}"
        assert sorted(payload["file_hashes"]) == [
            "optimizer.safetensors",
            "resume.json",
            "rng.safetensors",
            "weights.safetensors",
        ]
        assert all(value.startswith("blake3:") for value in payload["file_hashes"].values())
    assert saved[0]["checkpoint_id"] != saved[1]["checkpoint_id"]


def test_the_run_keeps_the_last_three_and_the_best(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 14, every_steps=2)
    events = RecordingEvents(context.run_folder)

    train(context, ClassificationRecipe(), events, JobControl())

    best = [payload["step"] for payload in events.payloads("checkpoint.saved") if payload["is_best"]][-1]
    assert best == context.progress.best_step
    assert checkpoint_steps(context) == sorted({10, 12, 14, best})


def test_three_non_finite_steps_in_a_row_stop_the_run(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 20)
    weights_before = {name: value.detach().clone() for name, value in context.model.graph_parameters().items()}

    with pytest.raises(TrainError) as stopped:
        train(context, NonFiniteOnSteps({1, 2, 3}), RecordingEvents(context.run_folder), JobControl())

    assert stopped.value.code == RRS_T3001
    for name, value in context.model.graph_parameters().items():
        assert torch.equal(value, weights_before[name]), name


def test_one_non_finite_step_is_skipped_and_left_out_of_the_metrics(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", 10, validation_ratio=0.0)
    events = RecordingEvents(context.run_folder)

    outcome = train(context, NonFiniteOnSteps({10}), events, JobControl())

    assert outcome.reason == "completed"
    last_record = train_metric_records(events)[-1]
    assert last_record["step"] == 10
    assert "train.grad_norm" not in last_record["values"]
    assert "train.loss" not in last_record["values"]


# ---- Recipes ----


def test_the_classification_recipe_is_registered_by_name() -> None:
    registry = PluginRegistry()
    register_first_party(registry)
    assert registry.names("recipes") == ["classification"]
    assert registry.find("recipes", "classification") is ClassificationRecipe
    assert FIRST_PARTY_RECIPES == {"classification": ClassificationRecipe}
