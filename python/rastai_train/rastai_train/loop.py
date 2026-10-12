"""The training loop (layout python.md sections 5.2 to 5.4; training-evaluation flows 4.1, 4.2 and 5.1).

A recipe returns a loss and its metrics; the loop owns everything else, so it is the same for every recipe:
gradient accumulation, clipping, the learning-rate schedule, the standard metrics, the checkpoint cadence and how
each JobControl flag ends a run. Flags are read only at a step boundary, where the weights, the optimizer and the
loader agree, so a checkpoint is always of a whole step.

| Flag | At the next step boundary | The worker's edge then |
|---|---|---|
| preempt_requested (SIGTERM) | a checkpoint if the last write time fits the budget | run preempted, exit 75 |
| cancel_requested (CANCEL, SIGINT) | nothing more | run cancelled, exit 3 |
| stop_requested (STOP) | a final checkpoint, whatever it takes | run completed (reason stopped), exit 0 |
| checkpoint_requested (CHECKPOINT_NOW) | a checkpoint, then training goes on | |
| paused (PAUSE) | waits for RESUME, or for one of the flags above | |

The last step of the run writes a final checkpoint too. preempt_requested is read first: only SIGTERM has a deadline.
The loop never writes the run's final state; the worker's edge does (rastai_worker/diagnostics.py). `train.preempted`
and `checkpoint.resumed` are worker events from M5 on (contracts 7.3); until their contract lands the edge's
`run.state_changed preempted` and exit 75 tell the node.
"""

import math
import time
from dataclasses import dataclass
from pathlib import Path

import torch
from rastai_plugin.catalog.generated import RRS_T3001

from rastai_plugin import Batch, JobControl, JobEvents, Recipe, TrainingContext
from rastai_train.checkpoints import (
    CHECKPOINT_ID_PREFIX,
    checkpoint_file_hashes,
    find_latest_checkpoint,
    new_generated_id,
    remove_unkept_checkpoints,
    write_checkpoint,
)
from rastai_train.errors import TrainError
from rastai_train.optimizer import learning_rate_at, set_learning_rate
from rastai_train.resume import restore_rng_state, rng_state_tensors

# run.metrics is a durable record; one every 10 steps keeps a long run's log small (flow 4.1).
STEPS_BETWEEN_METRIC_RECORDS = 10
# Three steps in a row with non-finite gradients stop the run (flow 4.3); one is skipped and training goes on.
NON_FINITE_STEPS_TO_STOP = 3
# The timed interval is never shorter than this many times the last checkpoint's write time, which keeps
# checkpointing under 5% of the run (flow 5.1).
INTERVAL_PER_WRITE_SECONDS = 20
SECONDS_PER_MINUTE = 60
# How often a paused run looks at its flags again.
PAUSE_POLL_SECONDS = 0.1
RUN_STATE_CHANGED = "run.state_changed"
RUN_STATE_RUNNING = "RUN_STATE_RUNNING"
RUN_METRICS = "run.metrics"
CHECKPOINT_SAVED = "checkpoint.saved"
JOB_PROGRESS = "job.progress"
# The standard metric names (flow 4.1). Recipe metrics carry the recipe's name as their prefix.
TRAIN_LOSS = "train.loss"
TRAIN_LEARNING_RATE = "train.learning_rate"
TRAIN_GRAD_NORM = "train.grad_norm"
TRAIN_SAMPLES_SEEN = "train.samples_seen"
VALIDATION_LOSS = "validation.loss"
# TrainingOutcome.reason.
OUTCOME_COMPLETED = "completed"
OUTCOME_STOPPED = "stopped"
OUTCOME_CANCELLED = "cancelled"
OUTCOME_PREEMPTED = "preempted"


@dataclass(frozen=True)
class TrainingOutcome:
    """How train() ended: completed, stopped, cancelled or preempted, at which step, and the run's newest complete
    checkpoint then (None when it has none)."""

    reason: str
    step: int
    checkpoint_folder: Path | None


def train(context: TrainingContext, recipe: Recipe, events: JobEvents, control: JobControl) -> TrainingOutcome:
    """Trains until the run's last step or until a flag ends it (see the table at the top)."""
    prepare_run(context, recipe)
    events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_RUNNING})
    while True:
        wait_while_paused(control)
        if control.preempt_requested:
            return preempt_at_step_boundary(context, recipe, events)
        if control.cancel_requested:
            return outcome(context, OUTCOME_CANCELLED)
        if control.stop_requested:
            return end_with_checkpoint(context, recipe, events, OUTCOME_STOPPED)
        if context.progress.step >= context.settings.max_steps:
            return end_with_checkpoint(context, recipe, events, OUTCOME_COMPLETED)
        run_training_step(context, recipe, events)
        if control.checkpoint_requested or checkpoint_is_due(context):
            control.checkpoint_requested = False
            save_checkpoint_and_report(context, recipe, events)


def prepare_run(context: TrainingContext, recipe: Recipe) -> None:
    """recipe.prepare, then, on a resume, the recipe state and the random generators: after prepare, so whatever
    prepare draws can't move the stream the first batch draws from."""
    recipe.prepare(context)
    resume_point = context.progress.resume_point
    if resume_point is not None:
        recipe.restore_state(resume_point.recipe_state)
        restore_rng_state(resume_point.rng_tensors)
        context.progress.resume_point = None
    context.model.train()
    context.progress.last_checkpoint_time = time.monotonic()


def wait_while_paused(control: JobControl) -> None:
    while control.paused and not is_asked_to_end(control):
        time.sleep(PAUSE_POLL_SECONDS)


def is_asked_to_end(control: JobControl) -> bool:
    return control.preempt_requested or control.cancel_requested or control.stop_requested


# ---- One step ----


def run_training_step(context: TrainingContext, recipe: Recipe, events: JobEvents) -> None:
    """One optimizer step over `gradient_accumulation` micro-batches, then its metrics."""
    learning_rate = learning_rate_at(
        context.progress.step,
        context.settings.learning_rate,
        context.settings.warmup_steps,
        context.settings.max_steps,
    )
    set_learning_rate(context.optimizer, learning_rate)
    last_loss, step_loss, recipe_metrics = accumulate_micro_batches(context, recipe)
    grad_norm = optimizer_step(context, last_loss)
    context.progress.step += 1
    values = {
        TRAIN_LOSS: step_loss,
        TRAIN_LEARNING_RATE: learning_rate,
        TRAIN_GRAD_NORM: grad_norm,
        TRAIN_SAMPLES_SEEN: float(context.progress.samples_seen),
    }
    values.update(recipe_metrics)
    record_step_metrics(events, context, values)


def accumulate_micro_batches(context: TrainingContext, recipe: Recipe) -> tuple[torch.Tensor, float, dict]:
    """Runs the step's micro-batches. Each loss is divided by their count, so the gradients are their mean. All but
    the last run backward here; the last is returned for optimizer_step, with the step's loss and the last
    micro-batch's recipe metrics."""
    micro_batch_count = context.settings.gradient_accumulation
    step_loss = 0.0
    for _ in range(micro_batch_count - 1):
        scaled_loss = recipe.training_step(next_batch(context)).loss / micro_batch_count
        step_loss += scaled_loss.item()
        scaled_loss.backward()
    last_result = recipe.training_step(next_batch(context))
    last_loss = last_result.loss / micro_batch_count
    step_loss += last_loss.item()
    return last_loss, step_loss, last_result.metrics


def next_batch(context: TrainingContext) -> Batch:
    """The loader's next batch. Its None at the end of an epoch is skipped: the call after it starts the next one."""
    examples = context.loader.next_batch()
    if examples is None:
        examples = context.loader.next_batch()
    context.progress.samples_seen += len(examples)
    return Batch(examples=examples)


def optimizer_step(context: TrainingContext, loss: torch.Tensor) -> float:
    """Backward on `loss`, clip the gradients to max_grad_norm, step, and return the gradient norm before clipping.
    A step whose gradients aren't finite is skipped; NON_FINITE_STEPS_TO_STOP in a row raise RRS-T3001."""
    loss.backward()
    parameters = optimizer_parameters(context.optimizer)
    grad_norm = torch.nn.utils.clip_grad_norm_(parameters, context.settings.max_grad_norm).item()
    if math.isfinite(grad_norm):
        context.optimizer.step()
        context.progress.non_finite_steps_in_a_row = 0
    else:
        count_non_finite_step(context, grad_norm)
    context.optimizer.zero_grad(set_to_none=True)
    return grad_norm


def optimizer_parameters(optimizer: torch.optim.Optimizer) -> list[torch.nn.Parameter]:
    parameters = []
    for group in optimizer.param_groups:
        parameters.extend(group["params"])
    return parameters


def count_non_finite_step(context: TrainingContext, grad_norm: float) -> None:
    progress = context.progress
    progress.non_finite_steps_in_a_row += 1
    if progress.non_finite_steps_in_a_row >= NON_FINITE_STEPS_TO_STOP:
        message = (
            f"The gradients were {grad_norm} for {progress.non_finite_steps_in_a_row} steps in a row, up to step "
            f"{progress.step + 1}."
        )
        raise TrainError(RRS_T3001, message)


def record_step_metrics(events: JobEvents, context: TrainingContext, values: dict[str, float]) -> None:
    """run.metrics every STEPS_BETWEEN_METRIC_RECORDS steps and at the last step, with job.progress beside it."""
    step = context.progress.step
    max_steps = context.settings.max_steps
    if step % STEPS_BETWEEN_METRIC_RECORDS != 0 and step != max_steps:
        return
    events.emit_durable(RUN_METRICS, {"step": step, "values": finite_values(values)})
    events.emit_ephemeral(JOB_PROGRESS, {"fraction": step / max_steps, "message": f"Step {step} of {max_steps}"})


def finite_values(values: dict[str, float]) -> dict[str, float]:
    """The values JSON can carry: a skipped step's NaN or infinite norm is left out of the record."""
    return {name: value for name, value in values.items() if math.isfinite(value)}


# ---- Checkpoints ----


def checkpoint_is_due(context: TrainingContext) -> bool:
    """Every `every_steps` steps, and when the timed interval has passed since the last checkpoint."""
    policy = context.settings.checkpoint
    progress = context.progress
    if policy.every_steps > 0 and progress.step % policy.every_steps == 0:
        return True
    if policy.interval_minutes <= 0:
        return False
    interval_seconds = policy.interval_minutes * SECONDS_PER_MINUTE
    if progress.last_checkpoint_seconds is not None:
        interval_seconds = max(interval_seconds, INTERVAL_PER_WRITE_SECONDS * progress.last_checkpoint_seconds)
    return time.monotonic() - progress.last_checkpoint_time >= interval_seconds


def save_checkpoint_and_report(context: TrainingContext, recipe: Recipe, events: JobEvents) -> Path:
    """Writes the checkpoint of the current step, then reports it: checkpoint.saved only after the rename, so the
    daemon never indexes a half-written folder. Then the retention rule removes what it doesn't keep."""
    progress = context.progress
    validation_loss = compute_validation_loss(context, recipe)
    is_best = validation_loss is not None and (
        progress.best_validation_loss is None or validation_loss < progress.best_validation_loss
    )
    write_started = time.monotonic()
    folder = write_checkpoint(context.run_folder, progress.step, context, recipe.resume_state(), validation_loss)
    progress.last_checkpoint_seconds = time.monotonic() - write_started
    progress.last_checkpoint_time = time.monotonic()
    progress.last_checkpoint_step = progress.step
    if is_best:
        progress.best_step = progress.step
        progress.best_validation_loss = validation_loss
    report_checkpoint(events, context, folder, is_best)
    if validation_loss is not None:
        events.emit_durable(RUN_METRICS, {"step": progress.step, "values": {VALIDATION_LOSS: validation_loss}})
    policy = context.settings.checkpoint
    remove_unkept_checkpoints(context.run_folder, policy.keep_last, progress.best_step, policy.pinned_steps)
    return folder


def report_checkpoint(events: JobEvents, context: TrainingContext, folder: Path, is_best: bool) -> None:
    payload = {
        "checkpoint_id": new_generated_id(CHECKPOINT_ID_PREFIX),
        "step": context.progress.step,
        "folder": folder.relative_to(context.run_folder).as_posix(),
        "file_hashes": checkpoint_file_hashes(folder),
        "is_best": is_best,
    }
    events.emit_durable(CHECKPOINT_SAVED, payload)


def compute_validation_loss(context: TrainingContext, recipe: Recipe) -> float | None:
    """The recipe's mean loss over the validation split in eval mode, or None when there is no split. The random
    generators are put back afterwards, so how often checkpoints run can't change the training stream."""
    examples = context.validation_examples
    if not examples:
        return None
    saved_generators = rng_state_tensors()
    batch_size = context.settings.micro_batch
    total_loss = 0.0
    context.model.eval()
    try:
        with torch.no_grad():
            for start in range(0, len(examples), batch_size):
                batch_examples = examples[start : start + batch_size]
                result = recipe.training_step(Batch(examples=batch_examples))
                total_loss += result.loss.item() * len(batch_examples)
    finally:
        context.model.train()
        restore_rng_state(saved_generators)
    return total_loss / len(examples)


# ---- Ends ----


def preempt_at_step_boundary(context: TrainingContext, recipe: Recipe, events: JobEvents) -> TrainingOutcome:
    """SIGTERM: a checkpoint when the last one's write time fits the preemption budget (or none was written yet in
    this job), then return, so the worker exits 75. A checkpoint that fails here fails the job (RRS-T2001)."""
    budget_seconds = context.settings.checkpoint.preemption_budget_seconds
    last_seconds = context.progress.last_checkpoint_seconds
    if last_seconds is None or last_seconds <= budget_seconds:
        save_checkpoint_if_new(context, recipe, events)
    return outcome(context, OUTCOME_PREEMPTED)


def end_with_checkpoint(context: TrainingContext, recipe: Recipe, events: JobEvents, reason: str) -> TrainingOutcome:
    """The last step, or STOP: a final checkpoint, with no budget."""
    save_checkpoint_if_new(context, recipe, events)
    return outcome(context, reason)


def save_checkpoint_if_new(context: TrainingContext, recipe: Recipe, events: JobEvents) -> None:
    """A checkpoint unless this step already has one (or no step was taken at all)."""
    if context.progress.step != context.progress.last_checkpoint_step:
        save_checkpoint_and_report(context, recipe, events)


def outcome(context: TrainingContext, reason: str) -> TrainingOutcome:
    return TrainingOutcome(reason, context.progress.step, find_latest_checkpoint(context.run_folder))
