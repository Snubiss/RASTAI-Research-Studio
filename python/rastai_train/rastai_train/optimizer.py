"""The optimizer and its schedule (layout python.md sections 5.1 and 5.3).

AdamW over the model's trainable parameters, with a linear warmup and then a cosine decay. The schedule is a pure
function of the step, so resume.json keeps only the schedule's name and step (`scheduler{}`); the hyperparameters
come back from the run's settings.

Optimizer state is saved by parameter name, never by index (ADR-0024): `<parameter>.exp_avg`,
`<parameter>.exp_avg_sq` and `<parameter>.step`, with the parameter's graph path as its name (`conv_1.weight`). A
resume finds each tensor by name, so a parameter added or moved doesn't shift another's state. torch's own
`state_dict()` is keyed by index and is never saved.
"""

import math

import torch
from rastai_plugin.catalog.generated import RRS_T2005

from rastai_models import CompiledModel
from rastai_train.errors import TrainError

ADAM_BETAS = (0.9, 0.999)
ADAM_EPSILON = 1e-8
# resume.json's scheduler{} names the schedule, so a checkpoint from another schedule is refused instead of resumed
# at the wrong learning rate.
SCHEDULE_NAME = "warmup_cosine"
SCHEDULE_NAME_KEY = "name"
SCHEDULE_STEP_KEY = "last_step"
# Between a parameter's name and its state's name in optimizer.safetensors. State names hold no dot, so the last
# dot splits them even when the parameter's node id has dots of its own.
STATE_NAME_SEPARATOR = "."
# The state AdamW keeps on the CPU whatever the parameter's device: torch keeps `step` there unless the optimizer is
# capturable or fused, which this one isn't.
CPU_STATE_NAMES = ("step",)
# How many names an error message lists.
SHOWN_NAME_COUNT = 10


def trainable_parameters(model: CompiledModel) -> dict[str, torch.nn.Parameter]:
    """The parameters the optimizer updates, by graph path, in graph order. Frozen nodes are left out."""
    parameters = {}
    for name, parameter in model.graph_parameters().items():
        if parameter.requires_grad:
            parameters[name] = parameter
    return parameters


def build_optimizer(model: CompiledModel, learning_rate: float, weight_decay: float) -> torch.optim.AdamW:
    """AdamW over the model's trainable parameters. The loop sets each step's learning rate (learning_rate_at)."""
    parameters = list(trainable_parameters(model).values())
    return torch.optim.AdamW(
        parameters, lr=learning_rate, betas=ADAM_BETAS, eps=ADAM_EPSILON, weight_decay=weight_decay
    )


def learning_rate_at(step: int, peak_learning_rate: float, warmup_steps: int, total_steps: int) -> float:
    """The learning rate of optimizer step `step` (0 is the first): a linear rise over `warmup_steps`, then a cosine
    from the peak down to 0 at `total_steps`."""
    if step < warmup_steps:
        return peak_learning_rate * (step + 1) / warmup_steps
    decay_steps = max(total_steps - warmup_steps, 1)
    decay_progress = min((step - warmup_steps) / decay_steps, 1.0)
    return peak_learning_rate * 0.5 * (1.0 + math.cos(math.pi * decay_progress))


def set_learning_rate(optimizer: torch.optim.Optimizer, learning_rate: float) -> None:
    for group in optimizer.param_groups:
        group["lr"] = learning_rate


def scheduler_state(step: int) -> dict:
    """resume.json's scheduler{}: the schedule and how many of its steps were taken."""
    return {SCHEDULE_NAME_KEY: SCHEDULE_NAME, SCHEDULE_STEP_KEY: step}


def check_scheduler_state(scheduler: dict, step: int) -> None:
    """Refuses a checkpoint written by another schedule, or one whose schedule step isn't its step (RRS-T2005)."""
    if scheduler.get(SCHEDULE_NAME_KEY) != SCHEDULE_NAME or scheduler.get(SCHEDULE_STEP_KEY) != step:
        message = f"The checkpoint's schedule {scheduler} isn't this run's {SCHEDULE_NAME} schedule at step {step}."
        raise TrainError(RRS_T2005, message)


def optimizer_state_tensors(
    optimizer: torch.optim.Optimizer, parameters: dict[str, torch.nn.Parameter]
) -> dict[str, torch.Tensor]:
    """Every state tensor of the optimizer as `<parameter name>.<state name>`. A parameter that hasn't been stepped
    yet has no state and no entry."""
    tensors = {}
    for name, parameter in parameters.items():
        state = optimizer.state.get(parameter, {})
        for state_name, value in state.items():
            if not torch.is_tensor(value):
                raise ValueError(f"optimizer state {name}{STATE_NAME_SEPARATOR}{state_name} isn't a tensor")
            tensors[f"{name}{STATE_NAME_SEPARATOR}{state_name}"] = value
    return tensors


def restore_optimizer_state(
    optimizer: torch.optim.Optimizer, parameters: dict[str, torch.nn.Parameter], tensors: dict[str, torch.Tensor]
) -> None:
    """Puts back what optimizer_state_tensors saved. State for a parameter this model doesn't train means the
    checkpoint is another run's (RRS-T2005); nothing changes then."""
    state_by_parameter = group_state_by_parameter(tensors)
    unknown_names = sorted(name for name in state_by_parameter if name not in parameters)
    if unknown_names:
        shown = ", ".join(unknown_names[:SHOWN_NAME_COUNT])
        raise TrainError(RRS_T2005, f"The optimizer state names parameters this model doesn't train: {shown}.")
    optimizer.state.clear()
    for name, state in state_by_parameter.items():
        parameter = parameters[name]
        optimizer.state[parameter] = place_state(state, parameter.device)


def group_state_by_parameter(tensors: dict[str, torch.Tensor]) -> dict[str, dict[str, torch.Tensor]]:
    state_by_parameter: dict[str, dict[str, torch.Tensor]] = {}
    for full_name, tensor in tensors.items():
        parameter_name, _, state_name = full_name.rpartition(STATE_NAME_SEPARATOR)
        if not parameter_name:
            raise TrainError(RRS_T2005, f"The optimizer state {full_name!r} names no parameter.")
        state_by_parameter.setdefault(parameter_name, {})[state_name] = tensor
    return state_by_parameter


def place_state(state: dict[str, torch.Tensor], device: torch.device) -> dict[str, torch.Tensor]:
    """Each state tensor on the device AdamW keeps it on."""
    placed = {}
    for state_name, tensor in state.items():
        if state_name in CPU_STATE_NAMES:
            placed[state_name] = tensor.to("cpu")
        else:
            placed[state_name] = tensor.to(device)
    return placed
