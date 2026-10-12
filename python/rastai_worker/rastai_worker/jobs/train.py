"""The train job (python layout 7.4; training-evaluation flows 4.1 and 5.1 to 5.4; contracts 4.4 and 9.5).

`run_job` sets one run up from what the node and the daemon wrote, then hands it to rastai_train's loop:

1. job.json, read through `WorkerHost.job_file_path`: the run's id and folder, and `resume_from`. The entry is
   given only the JobSpec (contracts 12.3), and the worker looks nothing else up (python/CLAUDE.md rule 3).
2. `JobSpec.arguments_json`, a `TrainJobArgs`: what to train (`start_arguments`), how (`plan`), for how many steps
   and from which seed; and `JobSpec.checkpoint_policy_json`, the run's `training.checkpoint` section. Text that
   isn't those shapes is RRS-J2001, before a byte of the dataset is read.
3. `run.state_changed preparing`, then the context (the design compiled, the dataset split, the optimizer) and the
   recipe from the registry. With `resume_from` set, the run continues from its newest complete checkpoint that
   loads, past damaged ones, each reported as RRS-T2002 (flow 5.2).
4. rastai_train's `train`, which writes `running`, the metrics and the checkpoints, and honours every JobControl
   flag at a step boundary.

The job never writes the run's final state: the worker's edge (diagnostics.py) does, from how this function ended
and the flags: completed, stopped, cancelled (exit 3), preempted (exit 75) or failed (exit 1).

A checkpoint that hits a full disk (RRS-T2001) pauses the run instead of failing it (contracts 9.5): the job reports
the code and `paused`, keeps the model, the optimizer and the loader in memory, and waits for RESUME; the loop then
goes on from the same step and writes `running` again. A job already asked to end doesn't pause: a preemption whose
checkpoint doesn't fit fails the job (python layout 5.4), so the node doesn't relaunch it forever.
"""

import json
import os

from rastai_formats.generated.command_train_start import TrainingPlan, TrainJobArgs, train_job_args_from_json
from rastai_formats.generated.settings import SettingsCheckpoint, settings_checkpoint_from_json
from rastai_plugin.catalog.generated import RRS_J2001, RRS_T0002, RRS_T2001, RRS_T2002
from rastai_proto.node.v1.jobs_pb2 import JobSpec
from rastai_train.recipes import RECIPES_KIND

from rastai_plugin import JobControl, JobEvents, PluginRegistry, Recipe, TrainingContext, WorkerHost
from rastai_train import (
    CheckpointPolicy,
    TrainError,
    TrainingSettings,
    build_training_context,
    checkpoint_policy,
    register_first_party,
    resume_from_latest_checkpoint,
    train,
)
from rastai_worker.control import job_is_ending, wait_for_resume
from rastai_worker.diagnostics import coded_diagnostic
from rastai_worker.errors import WorkerError
from rastai_worker.job_file import WorkerLaunch, read_job_file

# M1 trains every weight in float32 (rastai_train's TRAINING_DTYPE). LoRA, the other number formats and more than
# one GPU join with M5; a plan that asks for them now would be trained some other way than it says.
SUPPORTED_METHOD = "full"
SUPPORTED_DTYPE = "fp32"
MAX_GPU_COUNT = 1
CPU_DEVICE = "cpu"
# The node makes the job's GPUs the only visible ones (node layout 7.4), so the job's one GPU is always cuda:0.
FIRST_GPU_DEVICE = "cuda:0"
# cuBLAS refuses deterministic matmuls unless this is in the environment before CUDA starts (rastai_train CLAUDE.md).
CUBLAS_WORKSPACE_VARIABLE = "CUBLAS_WORKSPACE_CONFIG"
CUBLAS_WORKSPACE_SETTING = ":4096:8"
# The JobSpec fields this job reads (contracts 4.4), as error messages name them.
ARGUMENTS_FIELD = "arguments_json"
CHECKPOINT_POLICY_FIELD = "checkpoint_policy_json"
RUN_STATE_CHANGED = "run.state_changed"
DIAGNOSTIC_REPORTED = "diagnostic.reported"
RUN_STATE_PREPARING = "RUN_STATE_PREPARING"
RUN_STATE_PAUSED = "RUN_STATE_PAUSED"


def run_job(job_spec: JobSpec, host: WorkerHost, events: JobEvents, control: JobControl) -> None:
    """Trains the run that job.json and the JobSpec describe, until its last step or until a flag ends it."""
    launch = read_own_job_file(host, job_spec)
    job_arguments = read_train_job_arguments(job_spec)
    settings = training_settings(job_arguments, read_checkpoint_policy(job_spec))
    recipe = new_recipe(job_arguments.start_arguments.recipe)
    events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_PREPARING})
    device = training_device(job_arguments.plan)
    context = build_training_context(job_arguments.start_arguments, launch.run_id, launch.run_folder, settings, device)
    if launch.resume_from:
        resume_run(context, events, job_spec.job_id)
    train_until_done(context, recipe, events, control, job_spec.job_id)


# ---- What the node and the daemon wrote ----


def read_own_job_file(host: WorkerHost, job_spec: JobSpec) -> WorkerLaunch:
    """job.json, read again for what the entry isn't given (the run folder and resume_from). It is the file this
    worker started from, so it must describe this job."""
    launch = read_job_file(host.job_file_path)
    if launch.job_id != job_spec.job_id:
        reason = f"{host.job_file_path} describes job {launch.job_id}, not this one"
        raise WorkerError(RRS_J2001, f"Train job {job_spec.job_id} can't start: {reason}.")
    return launch


def read_train_job_arguments(job_spec: JobSpec) -> TrainJobArgs:
    """`arguments_json` as a TrainJobArgs (contracts 4.4). The daemon built it, so anything else is RRS-J2001."""
    values = read_json_object(job_spec, ARGUMENTS_FIELD, job_spec.arguments_json)
    try:
        job_arguments = train_job_args_from_json(values)
    except (KeyError, TypeError) as error:
        raise bad_field_error(job_spec, ARGUMENTS_FIELD, f"it isn't a TrainJobArgs ({error!r})") from None
    check_start_arguments(job_spec, job_arguments)
    check_counts(job_spec, job_arguments)
    check_plan_is_supported(job_spec, job_arguments.plan)
    return job_arguments


def check_start_arguments(job_spec: JobSpec, job_arguments: TrainJobArgs) -> None:
    start_arguments = job_arguments.start_arguments
    for name in ("recipe", "design_path", "dataset_path"):
        value = getattr(start_arguments, name)
        if not isinstance(value, str) or not value:
            raise bad_field_error(job_spec, ARGUMENTS_FIELD, f"its start_arguments have no {name}")


def check_counts(job_spec: JobSpec, job_arguments: TrainJobArgs) -> None:
    """The run's sizes are whole numbers: at least one step and one sample, a seed of zero or more."""
    plan = job_arguments.plan
    counts = [
        ("max_steps", job_arguments.max_steps, 1),
        ("seed", job_arguments.seed, 0),
        ("plan.micro_batch", plan.micro_batch, 1),
        ("plan.gradient_accumulation", plan.gradient_accumulation, 1),
    ]
    for name, value, minimum in counts:
        if not is_whole_number(value, minimum):
            raise bad_field_error(
                job_spec, ARGUMENTS_FIELD, f"its {name} is {value!r}, not a whole number >= {minimum}"
            )


def check_plan_is_supported(job_spec: JobSpec, plan: TrainingPlan) -> None:
    if plan.method != SUPPORTED_METHOD or plan.dtype != SUPPORTED_DTYPE:
        reason = (
            f"its plan trains {plan.method} in {plan.dtype}; this worker trains {SUPPORTED_METHOD} in {SUPPORTED_DTYPE}"
        )
        raise bad_field_error(job_spec, ARGUMENTS_FIELD, reason)
    if not isinstance(plan.gpu_indices, list) or len(plan.gpu_indices) > MAX_GPU_COUNT:
        reason = f"its plan names the GPUs {plan.gpu_indices!r}; this worker trains on the CPU or on one GPU"
        raise bad_field_error(job_spec, ARGUMENTS_FIELD, reason)


def read_checkpoint_policy(job_spec: JobSpec) -> CheckpointPolicy:
    """`checkpoint_policy_json`, the run's `training.checkpoint` section (contracts 4.4 and 11.2). Empty text, and
    every key it leaves out, take the 11.2 defaults."""
    if not job_spec.checkpoint_policy_json:
        return checkpoint_policy()
    values = read_json_object(job_spec, CHECKPOINT_POLICY_FIELD, job_spec.checkpoint_policy_json)
    checkpoint_settings = settings_checkpoint_from_json(values)
    check_checkpoint_settings(job_spec, checkpoint_settings)
    return checkpoint_policy(checkpoint_settings)


def check_checkpoint_settings(job_spec: JobSpec, checkpoint_settings: SettingsCheckpoint) -> None:
    """Each value given is a whole number; keep_last keeps at least one checkpoint."""
    values = [
        ("keep_last", checkpoint_settings.keep_last, 1),
        ("interval_minutes", checkpoint_settings.interval_minutes, 0),
        ("preemption_budget_seconds", checkpoint_settings.preemption_budget_seconds, 0),
    ]
    for name, value, minimum in values:
        if value is not None and not is_whole_number(value, minimum):
            reason = f"its {name} is {value!r}, not a whole number >= {minimum}"
            raise bad_field_error(job_spec, CHECKPOINT_POLICY_FIELD, reason)


def read_json_object(job_spec: JobSpec, field_name: str, text: str) -> dict:
    try:
        values = json.loads(text)
    except ValueError as error:
        raise bad_field_error(job_spec, field_name, f"it isn't JSON ({error})") from None
    if not isinstance(values, dict):
        raise bad_field_error(job_spec, field_name, "it isn't a JSON object")
    return values


def is_whole_number(value: object, minimum: int) -> bool:
    # bool is an int in Python, but `true` is no step count.
    return isinstance(value, int) and not isinstance(value, bool) and value >= minimum


def bad_field_error(job_spec: JobSpec, field_name: str, reason: str) -> WorkerError:
    return WorkerError(
        RRS_J2001, f"Train job {job_spec.job_id} can't start: its JobSpec {field_name} is unusable: {reason}."
    )


# ---- The run ----


def training_settings(job_arguments: TrainJobArgs, policy: CheckpointPolicy) -> TrainingSettings:
    """The plan's batch sizes, the run's length and seed, and the checkpoint policy. The recipe's own settings
    (learning rate, warmup, validation share) keep TrainingSettings' defaults until recipe settings join (M5)."""
    plan = job_arguments.plan
    return TrainingSettings(
        max_steps=job_arguments.max_steps,
        micro_batch=plan.micro_batch,
        gradient_accumulation=plan.gradient_accumulation,
        seed=job_arguments.seed,
        checkpoint=policy,
    )


def new_recipe(recipe_name: str) -> Recipe:
    """A new instance of the named recipe, found in the registry the way a plugin's would be (plugins layout 11.2).
    A name nothing registered is RRS-T0002."""
    registry = PluginRegistry()
    register_first_party(registry)
    recipe_class = registry.find(RECIPES_KIND, recipe_name)
    if recipe_class is None:
        known = ", ".join(registry.names(RECIPES_KIND))
        raise WorkerError(RRS_T0002, f"Recipe {recipe_name} isn't installed in this worker; it has {known}.")
    return recipe_class()


def training_device(plan: TrainingPlan) -> str:
    """The CPU, or the job's one GPU. A GPU run needs cuBLAS's workspace setting before CUDA starts, or deterministic
    mode refuses its matmuls; a value the environment already has is kept."""
    if not plan.gpu_indices:
        return CPU_DEVICE
    os.environ.setdefault(CUBLAS_WORKSPACE_VARIABLE, CUBLAS_WORKSPACE_SETTING)
    return FIRST_GPU_DEVICE


def resume_run(context: TrainingContext, events: JobEvents, job_id: str) -> None:
    """Continues the run from its newest complete checkpoint that loads (flow 5.2). Each newer one it passed is
    damaged: it stays on disk for inspection and is reported as RRS-T2002. None at all is RRS-T2004."""
    resume_point = resume_from_latest_checkpoint(context.run_folder, context)
    for message in resume_point.damaged_checkpoints:
        report_diagnostic(events, RRS_T2002, message, job_id)


def train_until_done(
    context: TrainingContext, recipe: Recipe, events: JobEvents, control: JobControl, job_id: str
) -> None:
    """rastai_train's loop, paused and started again after each checkpoint a full disk refused (see the top)."""
    while True:
        try:
            train(context, recipe, events, control)
            return
        except TrainError as error:
            if error.code != RRS_T2001 or job_is_ending(control):
                raise
            pause_for_full_checkpoint_disk(error, events, control, job_id)
        # Cancelled while paused: nothing is left to save, so the edge can record it at once. STOP and SIGTERM go
        # back into the loop, which tries their checkpoint again.
        if control.cancel_requested:
            return


def pause_for_full_checkpoint_disk(error: TrainError, events: JobEvents, control: JobControl, job_id: str) -> None:
    """RRS-T2001 and `paused`, then waits for RESUME or for the job to be asked to end. rastai_train already removed
    the checkpoint's `.partial` folder, so nothing half-written is left behind."""
    report_diagnostic(events, RRS_T2001, error.message, job_id)
    # The flag goes up before the record: a RESUME sent as soon as the node sees `paused` must find it to clear.
    control.paused = True
    events.emit_durable(RUN_STATE_CHANGED, {"run_state": RUN_STATE_PAUSED})
    wait_for_resume(control)


def report_diagnostic(events: JobEvents, code: str, message: str, job_id: str) -> None:
    events.emit_durable(DIAGNOSTIC_REPORTED, {"diagnostic": coded_diagnostic(code, message), "subject_id": job_id})
