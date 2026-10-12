"""The optimizer (rastai_train/optimizer.py): AdamW state round-trips through named tensors and a safetensors file,
by parameter name and never by index, and the warmup-cosine schedule is a pure function of the step."""

from pathlib import Path

import pytest
import torch
from rastai_formats.generated.command_train_start import TrainStartArgs
from rastai_plugin.catalog.generated import RRS_T2005
from rastai_train.optimizer import (
    build_optimizer,
    check_scheduler_state,
    learning_rate_at,
    optimizer_state_tensors,
    restore_optimizer_state,
    scheduler_state,
    trainable_parameters,
)

from rastai_models import SafetensorsFile, write_safetensors
from rastai_plugin import TrainingContext
from rastai_train import CheckpointPolicy, TrainError, TrainingSettings, build_training_context

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
TINY_DATASET = Path("fixtures/datasets/textcnn_tiny.jsonl")
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2N"
STEPS_BEFORE_SAVE = 3
SEED = 38
PEAK = 0.01
TEXTCNN_VOCAB_SIZE = 25000


@pytest.fixture(autouse=True)
def keep_torch_global_state():
    """build_training_context seeds the global generator and turns deterministic mode on; the other packages' tests
    run in the same process and must find both as they were."""
    deterministic = torch.are_deterministic_algorithms_enabled()
    generator_state = torch.get_rng_state()
    yield
    torch.use_deterministic_algorithms(deterministic)
    torch.set_rng_state(generator_state)


def new_context(repo_root: Path, run_folder: Path) -> TrainingContext:
    arguments = TrainStartArgs(
        recipe="classification",
        design_path=str(repo_root / TEXTCNN_TEMPLATE),
        dataset_path=str(repo_root / TINY_DATASET),
    )
    settings = TrainingSettings(max_steps=10, checkpoint=CheckpointPolicy(interval_minutes=0))
    return build_training_context(arguments, RUN_ID, run_folder, settings)


def take_step(model, optimizer, seed: int) -> None:
    """One AdamW step on a fixed random batch."""
    token_ids = torch.randint(0, TEXTCNN_VOCAB_SIZE, (4, 32), generator=torch.Generator().manual_seed(seed))
    model.eval()  # no dropout, so the step depends on the weights and the batch alone
    loss = model(token_ids).logsumexp(dim=-1).mean()
    loss.backward()
    optimizer.step()
    optimizer.zero_grad(set_to_none=True)


def stepped_context(repo_root: Path, run_folder: Path) -> TrainingContext:
    context = new_context(repo_root, run_folder)
    for step in range(STEPS_BEFORE_SAVE):
        take_step(context.model, context.optimizer, SEED + step)
    return context


def test_state_is_named_by_parameter_and_state(repo_root: Path, tmp_path: Path) -> None:
    context = stepped_context(repo_root, tmp_path)
    tensors = optimizer_state_tensors(context.optimizer, trainable_parameters(context.model))

    assert "conv_1.weight.exp_avg" in tensors
    assert "conv_1.weight.exp_avg_sq" in tensors
    assert "classifier.bias.step" in tensors
    assert tensors["classifier.bias.step"].item() == STEPS_BEFORE_SAVE
    assert len(tensors) == 3 * len(trainable_parameters(context.model))


def test_adamw_state_round_trips_through_a_safetensors_file(repo_root: Path, tmp_path: Path) -> None:
    saved = stepped_context(repo_root, tmp_path)
    path = tmp_path / "optimizer.safetensors"
    write_safetensors(path, optimizer_state_tensors(saved.optimizer, trainable_parameters(saved.model)))

    # A second model with the same weights and a fresh optimizer, given the saved state.
    restored = new_context(repo_root, tmp_path)
    restored.model.load_state_dict(saved.model.state_dict())
    with SafetensorsFile(path) as state_file:
        tensors = {name: state_file.load(name) for name in state_file.names()}
    restore_optimizer_state(restored.optimizer, trainable_parameters(restored.model), tensors)

    take_step(saved.model, saved.optimizer, SEED + 100)
    take_step(restored.model, restored.optimizer, SEED + 100)
    for name, parameter in saved.model.graph_parameters().items():
        assert torch.equal(parameter, restored.model.graph_parameters()[name]), name


def test_state_is_restored_by_name_whatever_the_order(repo_root: Path, tmp_path: Path) -> None:
    saved = stepped_context(repo_root, tmp_path)
    parameters = trainable_parameters(saved.model)
    tensors = optimizer_state_tensors(saved.optimizer, parameters)
    reordered_tensors = dict(reversed(list(tensors.items())))

    restored = new_context(repo_root, tmp_path)
    restored_parameters = trainable_parameters(restored.model)
    restore_optimizer_state(restored.optimizer, dict(reversed(list(restored_parameters.items()))), reordered_tensors)

    for name, parameter in restored_parameters.items():
        state = restored.optimizer.state[parameter]
        assert torch.equal(state["exp_avg"], tensors[f"{name}.exp_avg"]), name


def test_state_for_a_parameter_this_model_lacks_is_another_runs(repo_root: Path, tmp_path: Path) -> None:
    saved = stepped_context(repo_root, tmp_path)
    tensors = optimizer_state_tensors(saved.optimizer, trainable_parameters(saved.model))
    tensors["head_from_another_design.weight.exp_avg"] = torch.zeros(2)
    restored = new_context(repo_root, tmp_path)

    with pytest.raises(TrainError) as refused:
        restore_optimizer_state(restored.optimizer, trainable_parameters(restored.model), tensors)
    assert refused.value.code == RRS_T2005
    assert "head_from_another_design.weight" in refused.value.message
    assert len(restored.optimizer.state) == 0


def test_frozen_parameters_are_not_trained(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path)
    context.model.graph_parameters()["embedding.weight"].requires_grad_(False)
    assert "embedding.weight" not in trainable_parameters(context.model)
    optimizer = build_optimizer(context.model, PEAK, 0.0)
    optimized = {id(parameter) for group in optimizer.param_groups for parameter in group["params"]}
    assert id(context.model.graph_parameters()["embedding.weight"]) not in optimized


def test_warmup_rises_linearly_then_the_cosine_falls_to_zero() -> None:
    warmup_steps = 4
    total_steps = 14
    assert learning_rate_at(0, PEAK, warmup_steps, total_steps) == pytest.approx(PEAK / 4)
    assert learning_rate_at(3, PEAK, warmup_steps, total_steps) == pytest.approx(PEAK)
    assert learning_rate_at(4, PEAK, warmup_steps, total_steps) == pytest.approx(PEAK)
    assert learning_rate_at(9, PEAK, warmup_steps, total_steps) == pytest.approx(PEAK / 2)
    assert learning_rate_at(14, PEAK, warmup_steps, total_steps) == pytest.approx(0.0)
    assert learning_rate_at(20, PEAK, warmup_steps, total_steps) == pytest.approx(0.0)


def test_no_warmup_starts_at_the_peak() -> None:
    assert learning_rate_at(0, PEAK, 0, 10) == pytest.approx(PEAK)


def test_the_schedule_state_names_the_schedule_and_its_step() -> None:
    state = scheduler_state(12)
    check_scheduler_state(state, 12)
    with pytest.raises(TrainError) as other_step:
        check_scheduler_state(state, 13)
    assert other_step.value.code == RRS_T2005
    with pytest.raises(TrainError) as other_schedule:
        check_scheduler_state({"name": "linear", "last_step": 12}, 12)
    assert other_schedule.value.code == RRS_T2005
