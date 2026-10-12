"""The resume contract (rastai_train/resume.py), on the CPU in deterministic mode: a run interrupted and resumed in a
new job equals an uninterrupted one bit for bit, in every step's loss, the final weights, the optimizer state and
the random generator, whether it was preempted (SIGTERM) or killed after a periodic checkpoint, and across an epoch
boundary (training-evaluation flow 5.2)."""

import json
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.command_train_start import TrainStartArgs
from rastai_plugin.catalog.generated import RRS_T2005
from rastai_train.optimizer import optimizer_state_tensors, trainable_parameters

from rastai_plugin import Batch, JobControl, JobEvents, StepResult, TrainingContext
from rastai_train import (
    CheckpointPolicy,
    ClassificationRecipe,
    TrainError,
    TrainingSettings,
    build_training_context,
    find_latest_checkpoint,
    load_resume_state,
    resume_from_latest_checkpoint,
    train,
)

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
TINY_DATASET = Path("fixtures/datasets/textcnn_tiny.jsonl")
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2N"
# 1,800-odd training rows in batches of 128: an epoch is 15 steps, so a 24-step run crosses into epoch 1.
MICRO_BATCH = 128
TOTAL_STEPS = 24
STEPS_PER_EPOCH = 15


@pytest.fixture(autouse=True)
def keep_torch_global_state():
    """build_training_context seeds the global generator and turns deterministic mode on; the other packages' tests
    run in the same process and must find both as they were."""
    deterministic = torch.are_deterministic_algorithms_enabled()
    generator_state = torch.get_rng_state()
    yield
    torch.use_deterministic_algorithms(deterministic)
    torch.set_rng_state(generator_state)


class QuietEvents(JobEvents):
    def emit_durable(self, event_type: str, payload: dict, blob_hashes: list[str] | None = None) -> None:
        pass

    def emit_ephemeral(self, event_type: str, payload: dict) -> None:
        pass


class JobKilledError(Exception):
    """Stands in for a kill -9: the job ends mid-run and writes nothing more."""


class LossRecordingRecipe(ClassificationRecipe):
    """Classification that keeps each training step's loss, sets a flag after step `flag_after_step` and dies
    during step `killed_at_step` (both counted in this job)."""

    def __init__(self, control: JobControl, flag_after_step: int = 0, killed_at_step: int = 0) -> None:
        self.control = control
        self.flag_after_step = flag_after_step
        self.killed_at_step = killed_at_step
        self.losses: list[float] = []

    def training_step(self, batch: Batch) -> StepResult:
        if self.model.training and len(self.losses) + 1 == self.killed_at_step:
            raise JobKilledError
        result = super().training_step(batch)
        if self.model.training:
            self.losses.append(result.loss.item())
            if len(self.losses) == self.flag_after_step:
                self.control.preempt_requested = True
        return result


def new_context(repo_root: Path, run_folder: Path, every_steps: int = 0) -> TrainingContext:
    """A new job's context, built from scratch exactly as the train job builds it."""
    arguments = TrainStartArgs(
        recipe="classification",
        design_path=str(repo_root / TEXTCNN_TEMPLATE),
        dataset_path=str(repo_root / TINY_DATASET),
    )
    settings = TrainingSettings(
        max_steps=TOTAL_STEPS,
        micro_batch=MICRO_BATCH,
        learning_rate=5e-3,
        warmup_steps=3,
        seed=1234,
        deterministic=True,
        checkpoint=CheckpointPolicy(interval_minutes=0, every_steps=every_steps),
    )
    return build_training_context(arguments, RUN_ID, run_folder, settings)


def run_uninterrupted(repo_root: Path, run_folder: Path) -> tuple[TrainingContext, list[float]]:
    context = new_context(repo_root, run_folder)
    control = JobControl()
    recipe = LossRecordingRecipe(control)
    assert train(context, recipe, QuietEvents(), control).reason == "completed"
    return context, recipe.losses


def resume_and_finish(repo_root: Path, run_folder: Path) -> tuple[TrainingContext, list[float]]:
    """A second job on the same run folder: resume from the newest checkpoint, train to the end."""
    context = new_context(repo_root, run_folder)
    resume_from_latest_checkpoint(run_folder, context)
    control = JobControl()
    recipe = LossRecordingRecipe(control)
    assert train(context, recipe, QuietEvents(), control).reason == "completed"
    return context, recipe.losses


def assert_same_run_state(expected: TrainingContext, actual: TrainingContext) -> None:
    """Bit for bit: every weight, every optimizer state tensor, the generator, the loader and the counters."""
    for name, parameter in expected.model.graph_parameters().items():
        assert torch.equal(parameter, actual.model.graph_parameters()[name]), name
    expected_state = optimizer_state_tensors(expected.optimizer, trainable_parameters(expected.model))
    actual_state = optimizer_state_tensors(actual.optimizer, trainable_parameters(actual.model))
    assert sorted(expected_state) == sorted(actual_state)
    for name, tensor in expected_state.items():
        assert torch.equal(tensor, actual_state[name]), name
    assert expected.loader.position() == actual.loader.position()
    assert expected.progress.step == actual.progress.step
    assert expected.progress.samples_seen == actual.progress.samples_seen


@pytest.mark.parametrize(
    "preempted_after_step",
    [
        5,
        # The end of epoch 0: the loader has used every example but hasn't started epoch 1 yet.
        STEPS_PER_EPOCH,
        STEPS_PER_EPOCH + 2,
    ],
)
def test_a_preempted_and_resumed_run_equals_an_uninterrupted_one_bit_for_bit(
    repo_root: Path, tmp_path: Path, preempted_after_step: int
) -> None:
    straight, straight_losses = run_uninterrupted(repo_root, tmp_path / "straight")
    final_generator_state = torch.get_rng_state()
    assert torch.are_deterministic_algorithms_enabled()

    interrupted_folder = tmp_path / "interrupted"
    first_job = new_context(repo_root, interrupted_folder)
    control = JobControl()
    first_recipe = LossRecordingRecipe(control, flag_after_step=preempted_after_step)
    outcome = train(first_job, first_recipe, QuietEvents(), control)
    assert outcome.reason == "preempted"
    assert outcome.step == preempted_after_step

    second_job, resumed_losses = resume_and_finish(repo_root, interrupted_folder)

    assert len(resumed_losses) == TOTAL_STEPS - preempted_after_step
    assert first_recipe.losses + resumed_losses == straight_losses
    assert_same_run_state(straight, second_job)
    assert torch.equal(torch.get_rng_state(), final_generator_state)


def test_a_job_killed_after_a_periodic_checkpoint_resumes_from_it_bit_for_bit(repo_root: Path, tmp_path: Path) -> None:
    straight, straight_losses = run_uninterrupted(repo_root, tmp_path / "straight")

    interrupted_folder = tmp_path / "interrupted"
    first_job = new_context(repo_root, interrupted_folder, every_steps=4)
    control = JobControl()
    # Checkpoints at steps 4 and 8; killed during step 11, so steps 9 and 10 are lost and done again.
    first_recipe = LossRecordingRecipe(control, killed_at_step=11)
    with pytest.raises(JobKilledError):
        train(first_job, first_recipe, QuietEvents(), control)
    assert find_latest_checkpoint(interrupted_folder).name == "step-00000008"

    second_job, resumed_losses = resume_and_finish(repo_root, interrupted_folder)

    assert first_recipe.losses[:8] + resumed_losses == straight_losses
    assert_same_run_state(straight, second_job)


def test_resume_json_records_the_loader_position_schedule_and_recipe_state(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    control = JobControl()
    train(context, LossRecordingRecipe(control, flag_after_step=STEPS_PER_EPOCH + 2), QuietEvents(), control)

    document = json.loads((find_latest_checkpoint(context.run_folder) / "resume.json").read_text())
    assert document["step"] == STEPS_PER_EPOCH + 2
    assert document["epoch"] == 1
    assert document["samples_seen"] == context.progress.samples_seen
    assert document["dataloader"] == {
        "dataset_hash": context.dataset_hash,
        "split": "train",
        "shuffle_seed": 1234,
        "epoch": 1,
        "position": 2 * MICRO_BATCH,
    }
    assert document["scheduler"] == {"name": "warmup_cosine", "last_step": STEPS_PER_EPOCH + 2}
    assert document["recipe_state"] == {"labels": ["cooking", "music", "sports", "weather"]}


def test_a_checkpoint_with_other_labels_is_refused_when_the_recipe_restores(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    control = JobControl()
    train(context, LossRecordingRecipe(control, flag_after_step=2), QuietEvents(), control)

    resumed = new_context(repo_root, context.run_folder)
    point = load_resume_state(find_latest_checkpoint(context.run_folder), resumed)
    point.recipe_state = {"labels": ["cats", "dogs"]}
    with pytest.raises(TrainError) as refused:
        train(resumed, ClassificationRecipe(), QuietEvents(), JobControl())
    assert refused.value.code == RRS_T2005


def test_a_checkpoint_of_another_dataset_is_refused(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    control = JobControl()
    train(context, LossRecordingRecipe(control, flag_after_step=2), QuietEvents(), control)

    resumed = new_context(repo_root, context.run_folder)
    resumed.dataset_hash = "blake3:" + "0" * 64
    with pytest.raises(TrainError) as refused:
        load_resume_state(find_latest_checkpoint(context.run_folder), resumed)
    assert refused.value.code == RRS_T2005
    assert "dataset" in refused.value.message


class RandomDrawingRecipe(LossRecordingRecipe):
    """Classification that also draws from the global generator on every call, validation included, as a recipe
    with noise or sampling would."""

    def training_step(self, batch: Batch) -> StepResult:
        torch.rand(1)
        return super().training_step(batch)


def test_how_often_checkpoints_run_doesnt_change_the_training(repo_root: Path, tmp_path: Path) -> None:
    rarely = new_context(repo_root, tmp_path / "rarely")
    rarely_recipe = RandomDrawingRecipe(JobControl())
    train(rarely, rarely_recipe, QuietEvents(), JobControl())

    # A checkpoint, and so a validation pass, after every step.
    often = new_context(repo_root, tmp_path / "often", every_steps=1)
    often_recipe = RandomDrawingRecipe(JobControl())
    train(often, often_recipe, QuietEvents(), JobControl())

    assert often_recipe.losses == rarely_recipe.losses
    assert_same_run_state(rarely, often)
