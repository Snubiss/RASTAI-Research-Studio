"""Filling a compiled model's parameters (layout python.md sections 2.1 and 2.2).

Two ways in, nothing else:
- `bind_weights`: strictly from safetensors files, read through SafetensorsFile, so every header has passed the
  rastai_formats check first (contracts 2.4 rule 10) and nothing is ever unpickled. Strict means every parameter
  needs a tensor and every tensor a parameter, with the same shape. All of that is checked before the first value
  is copied, so a refused bind leaves the model as it was.
- `init_weights`: a fresh design, from one seed. Each op module draws its values in graph order from one CPU
  generator, so the same seed gives the same weights every time, on any device and in any dtype.

Tensor names are graph parameter paths, `<node_id>.<name>` (CompiledModel.graph_parameters). Renaming a pretrained
model's tensors through a Family, and the spec's `weights[]` prefixes and name mappings, join with the families
(M2) as bind_weights' `spec` and `family` arguments.
"""

from dataclasses import dataclass

import torch
from rastai_plugin.catalog.generated import RRS_M2002, RRS_M2003, RRS_M2004

from rastai_models.compiler.compiled_model import CompiledModel
from rastai_models.errors import ModelError
from rastai_models.io.safetensors_reader import SafetensorsFile

# How many tensor names an error message lists before it says how many more there are.
SHOWN_NAME_COUNT = 10


@dataclass(frozen=True)
class BindReport:
    """What a successful bind filled: each parameter's graph path in graph order, and the values copied."""

    parameter_names: list[str]
    value_count: int


def init_weights(model: CompiledModel, seed: int) -> None:
    """Fills every parameter of a fresh design from `seed`."""
    generator = torch.Generator(device="cpu").manual_seed(seed)
    for op_module in model.op_modules:
        op_module.init_parameters(generator)


def bind_weights(model: CompiledModel, weight_files: list[SafetensorsFile]) -> BindReport:
    """Fills every parameter from `weight_files`, converting each tensor to its parameter's dtype and device.
    Raises ModelError before copying anything: RRS-M2002 when a parameter has no tensor, RRS-M2003 when a tensor has
    no parameter (or is held by two files), RRS-M2004 when a tensor's shape differs from its parameter's."""
    parameters = model.graph_parameters()
    refuse_meta_parameters(parameters)
    file_by_name, repeated_names = locate_tensors(weight_files)
    check_names(parameters, file_by_name, repeated_names)
    check_shapes(parameters, file_by_name)

    value_count = 0
    for name, parameter in parameters.items():
        tensor = file_by_name[name].load(name, device=parameter.device, dtype=parameter.dtype)
        with torch.no_grad():
            parameter.copy_(tensor)
        value_count += parameter.numel()
    return BindReport(parameter_names=list(parameters), value_count=value_count)


def refuse_meta_parameters(parameters: dict[str, torch.nn.Parameter]) -> None:
    # A meta tensor has no storage, so copying into it would silently keep nothing. Building on the meta device and
    # assigning the loaded tensors arrives with load_model (M2).
    for name, parameter in parameters.items():
        if parameter.is_meta:
            raise ValueError(f"{name} is on the meta device; compile on a real device before binding weights")


def locate_tensors(weight_files: list[SafetensorsFile]) -> tuple[dict[str, SafetensorsFile], list[str]]:
    """The file holding each tensor name, and the names a second file holds again."""
    file_by_name: dict[str, SafetensorsFile] = {}
    repeated_names = []
    for weight_file in weight_files:
        for name in weight_file.names():
            if name in file_by_name:
                repeated_names.append(name)
            else:
                file_by_name[name] = weight_file
    return file_by_name, repeated_names


def check_names(
    parameters: dict[str, torch.nn.Parameter],
    file_by_name: dict[str, SafetensorsFile],
    repeated_names: list[str],
) -> None:
    missing = [name for name in parameters if name not in file_by_name]
    if missing:
        message = f"The weights lack tensors the ModelSpec needs: {shown_names(missing)}."
        raise ModelError(RRS_M2002, message)
    # A repeated name's second copy is a tensor nothing uses: which file is meant can't be told.
    unused = sorted(set(name for name in file_by_name if name not in parameters) | set(repeated_names))
    if unused:
        message = f"The weights hold tensors the ModelSpec doesn't use: {shown_names(unused)}."
        raise ModelError(RRS_M2003, message)


def check_shapes(parameters: dict[str, torch.nn.Parameter], file_by_name: dict[str, SafetensorsFile]) -> None:
    for name, parameter in parameters.items():
        found_shape = file_by_name[name].shape(name)
        expected_shape = list(parameter.shape)
        if found_shape != expected_shape:
            message = f"Tensor {name} has shape {found_shape}, but the ModelSpec needs {expected_shape}."
            raise ModelError(RRS_M2004, message)


def shown_names(names: list[str]) -> str:
    """The first names in a message, then how many more there are."""
    shown = ", ".join(names[:SHOWN_NAME_COUNT])
    hidden_count = len(names) - SHOWN_NAME_COUNT
    if hidden_count > 0:
        shown += f" and {hidden_count} more"
    return shown
