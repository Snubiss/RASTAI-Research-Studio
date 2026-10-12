"""The training context: the model, the data, the optimizer and the settings one run trains with (layout python.md
sections 5.1 and 5.2).

`build_training_context` builds it from plain values: the train.start arguments (the design and the dataset), the
run's id and folder, and the resolved settings. The train job (rastai_worker/jobs/train.py, M1-46) reads those from
job.json; nothing here reads settings.json or looks a path up (python/CLAUDE.md rule 3).

Deterministic by construction (python/CLAUDE.md rule 10): the weights come from the run's seed, the dropout stream
from the global torch generator seeded with it, the split from a hash of each row's id and the shuffle from the seed
and the epoch. With `deterministic` on, torch is also told to use deterministic kernels, so a run repeats bit for bit
on the CPU.
"""

import json
import time
from dataclasses import dataclass, field
from pathlib import Path

import torch
from rastai_formats.generated.command_train_start import TrainStartArgs
from rastai_formats.generated.model import model_spec_from_json
from rastai_formats.generated.settings import SettingsCheckpoint
from rastai_plugin.catalog.generated import RRS_D1001, RRS_T0003

import rastai_formats
from rastai_data import DataError, assign_split, make_loader, read_csv_rows, read_jsonl_rows
from rastai_formats import FormatsValidationError
from rastai_models import CompiledModel, compile_model, init_weights
from rastai_plugin import TrainingContext
from rastai_train.errors import TrainError, train_error_from_refusal
from rastai_train.optimizer import build_optimizer
from rastai_train.resume import ResumePoint

# The two splits a classification dataset is cut into; the loader reads the first, validation.loss the second.
TRAIN_SPLIT = "train"
VALIDATION_SPLIT = "validation"
# The dataset files M1 reads, by suffix. Parquet and the other sources join with rastai_data's read_dataset (M5).
JSONL_SUFFIX = ".jsonl"
CSV_SUFFIX = ".csv"
# The column that names a row for its split. A row without one is named by its line instead.
ROW_ID_COLUMN = "id"
# CPU training is float32 (the capability matrix: bf16 needs a GPU of Ampere or newer).
TRAINING_DTYPE = torch.float32
# The training.checkpoint defaults (contracts 11.2), for the fields a run's settings leave out. A spot notice gives
# about 30 s, so the preemption budget leaves a few seconds for the process to exit.
DEFAULT_KEEP_LAST = 3
DEFAULT_INTERVAL_MINUTES = 15
DEFAULT_PREEMPTION_BUDGET_SECONDS = 25


@dataclass(frozen=True)
class CheckpointPolicy:
    """When checkpoints are written and which are kept. `checkpoint_settings` is the run's `training.checkpoint`
    section (contracts 11.2) as the generated type, with every field filled (checkpoint_policy does that); the two
    other values aren't settings: the run sets them for itself."""

    checkpoint_settings: SettingsCheckpoint
    # A checkpoint every this many steps besides the timed ones; 0 for none.
    every_steps: int = 0
    # Steps whose checkpoints are pinned: never removed by the keep_last rule.
    pinned_steps: tuple[int, ...] = ()


def checkpoint_policy(
    checkpoint_settings: SettingsCheckpoint | None = None, every_steps: int = 0, pinned_steps: tuple[int, ...] = ()
) -> CheckpointPolicy:
    """The policy for these `training.checkpoint` settings, each field they leave out at its 11.2 default. The train
    job (M1-46) passes the settings it reads from job.json; the shape of `JobSpec.checkpoint_policy_json` lands with
    that job's contract change, not here."""
    given = checkpoint_settings
    if given is None:
        given = SettingsCheckpoint()
    filled = SettingsCheckpoint(
        keep_last=value_or_default(given.keep_last, DEFAULT_KEEP_LAST),
        interval_minutes=value_or_default(given.interval_minutes, DEFAULT_INTERVAL_MINUTES),
        preemption_budget_seconds=value_or_default(given.preemption_budget_seconds, DEFAULT_PREEMPTION_BUDGET_SECONDS),
    )
    return CheckpointPolicy(filled, every_steps, pinned_steps)


def value_or_default(value: int | None, default: int) -> int:
    if value is None:
        return default
    return value


@dataclass(frozen=True)
class TrainingSettings:
    """The resolved settings of one training run. The sizes come from the run's TrainingPlan, the rest from the
    recipe's settings and the training.* settings."""

    # Optimizer steps in the whole run; the schedule ends here.
    max_steps: int
    # Samples per forward pass (TrainingPlan.micro_batch).
    micro_batch: int = 32
    # Micro-batches per optimizer step (TrainingPlan.gradient_accumulation); the effective batch is the product.
    gradient_accumulation: int = 1
    learning_rate: float = 1e-3
    weight_decay: float = 0.01
    warmup_steps: int = 0
    # Gradients are clipped to this total norm before every step.
    max_grad_norm: float = 1.0
    # The one seed of the run: weights, dropout, split and shuffle.
    seed: int = 0
    # The share of rows held out for validation.loss; 0 for none.
    validation_ratio: float = 0.1
    # Deterministic torch kernels: bit-for-bit repeats on the CPU, at some speed cost on a GPU. On CUDA, cuBLAS
    # refuses deterministic matmuls unless CUBLAS_WORKSPACE_CONFIG is set before CUDA starts: the train job's to set.
    deterministic: bool = True
    checkpoint: CheckpointPolicy = field(default_factory=checkpoint_policy)


@dataclass
class TrainingProgress:
    """Where the run stands. The loop updates it; resume.json saves the parts a resume needs."""

    # Optimizer steps taken.
    step: int = 0
    samples_seen: int = 0
    # The step of the newest checkpoint this run has (0 when it has none), so the same step isn't saved twice.
    last_checkpoint_step: int = 0
    # How long the last checkpoint took to write, for the preemption budget; None before the first one.
    last_checkpoint_seconds: float | None = None
    # time.monotonic() at the last checkpoint, or at the start, for the timed cadence.
    last_checkpoint_time: float = field(default_factory=time.monotonic)
    best_step: int | None = None
    best_validation_loss: float | None = None
    # Steps in a row whose gradients weren't finite; three stop the run (RRS-T3001).
    non_finite_steps_in_a_row: int = 0
    # Set by load_resume_state: what the loop restores after recipe.prepare, before the first batch.
    resume_point: ResumePoint | None = None
    # Steps of newer checkpoints the resume couldn't load and fell back past. They stay on disk for inspection, and
    # the run's latest, best and keep_last leave them out until it writes that step again.
    skipped_checkpoint_steps: tuple[int, ...] = ()


def build_training_context(
    arguments: TrainStartArgs, run_id: str, run_folder: Path, settings: TrainingSettings, device: str = "cpu"
) -> TrainingContext:
    """A fresh run's context: the design compiled and initialised from the seed, the dataset split, a loader at the
    start of epoch 0 and the optimizer. A resume then loads a checkpoint into it (load_resume_state)."""
    check_settings(settings)
    set_random_state(settings)
    model = compile_design(Path(arguments.design_path), device, settings.seed)
    dataset_path = Path(arguments.dataset_path)
    rows = read_dataset_rows(dataset_path)
    train_rows, validation_rows = split_rows(rows, settings)
    if not train_rows:
        raise TrainError(RRS_T0003, f"{dataset_path}: no rows are left to train on after the validation split.")
    return TrainingContext(
        run_id=run_id,
        run_folder=run_folder,
        model=model,
        device=device,
        optimizer=build_optimizer(model, settings.learning_rate, settings.weight_decay),
        loader=make_loader(train_rows, settings.micro_batch, settings.seed, 0, 0),
        validation_examples=validation_rows,
        dataset_hash=rastai_formats.hash_bytes(dataset_path.read_bytes()),
        split=TRAIN_SPLIT,
        settings=settings,
        progress=TrainingProgress(),
    )


def check_settings(settings: TrainingSettings) -> None:
    """Settings arrive resolved and checked by the daemon, so a value out of range here is a bug: ValueError."""
    if settings.max_steps < 1 or settings.micro_batch < 1 or settings.gradient_accumulation < 1:
        raise ValueError(f"max_steps, micro_batch and gradient_accumulation must be at least 1: {settings}")
    if not 0.0 <= settings.validation_ratio < 1.0:
        raise ValueError(f"validation_ratio must be in [0, 1); got {settings.validation_ratio}")
    checkpoint_settings = settings.checkpoint.checkpoint_settings
    if None in (
        checkpoint_settings.keep_last,
        checkpoint_settings.interval_minutes,
        checkpoint_settings.preemption_budget_seconds,
    ):
        raise ValueError(f"every training.checkpoint field must be filled (checkpoint_policy): {checkpoint_settings}")
    if checkpoint_settings.keep_last < 1:
        raise ValueError(f"checkpoint.keep_last must be at least 1; got {checkpoint_settings.keep_last}")


def set_random_state(settings: TrainingSettings) -> None:
    """Seeds the global torch generator, which dropout draws from, and turns on deterministic kernels if asked."""
    torch.manual_seed(settings.seed)
    torch.use_deterministic_algorithms(settings.deterministic)


def compile_design(design_path: Path, device: str, seed: int) -> CompiledModel:
    """The .rrsmodel design, validated by the wheel (the one validation), compiled and filled from `seed`."""
    try:
        spec_json = rastai_formats.load_model_spec(design_path.read_bytes())
    except FormatsValidationError as error:
        raise train_error_from_refusal(error, design_path.name) from None
    model = compile_model(model_spec_from_json(json.loads(spec_json)), device, TRAINING_DTYPE)
    init_weights(model, seed)
    return model


def read_dataset_rows(dataset_path: Path) -> list[dict]:
    """Every row of a JSONL or CSV dataset file. The readers raise DataError for anything they can't read."""
    suffix = dataset_path.suffix.lower()
    if suffix == JSONL_SUFFIX:
        return list(read_jsonl_rows(dataset_path))
    if suffix == CSV_SUFFIX:
        return list(read_csv_rows(dataset_path))
    raise DataError(RRS_D1001, f"{dataset_path}: a training dataset is a .jsonl or .csv file in this version.")


def split_rows(rows: list[dict], settings: TrainingSettings) -> tuple[list[dict], list[dict]]:
    """The training and validation rows, each in file order. A row's split depends only on its id and the seed."""
    ratios = {TRAIN_SPLIT: 1.0 - settings.validation_ratio, VALIDATION_SPLIT: settings.validation_ratio}
    train_rows = []
    validation_rows = []
    for line_index, row in enumerate(rows):
        row_key = str(row.get(ROW_ID_COLUMN, line_index))
        if assign_split(row_key, ratios, settings.seed) == VALIDATION_SPLIT:
            validation_rows.append(row)
        else:
            train_rows.append(row)
    return train_rows, validation_rows
