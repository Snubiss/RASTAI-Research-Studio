"""Ops without parameters (contracts 5.3): `activation`, `dropout` and `concat`."""

import torch
from rastai_formats.generated.model import (
    activation_attributes_from_json,
    concat_attributes_from_json,
    dropout_attributes_from_json,
)
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule
from rastai_models.forward_context import ForwardContext

DEFAULT_CONCAT_AXIS = -1
# Each `function` attribute value and the elementwise function it names.
ACTIVATION_FUNCTIONS = {
    "relu": functional.relu,
    "gelu": functional.gelu,
    "silu": functional.silu,
    "tanh": torch.tanh,
    "sigmoid": torch.sigmoid,
}


class Activation(OpModule):
    """An elementwise function; the shape is unchanged."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        self.function_name = activation_attributes_from_json(attributes).function
        self.function = ACTIVATION_FUNCTIONS[self.function_name]

    def forward(self, values: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return self.function(values)


class Dropout(OpModule):
    """Zeroes each element with chance `probability` while training (and scales the rest up to keep the mean);
    passes values through unchanged in eval mode."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        self.probability = dropout_attributes_from_json(attributes).probability

    def forward(self, values: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return functional.dropout(values, self.probability, self.training)


class Concat(OpModule):
    """Joins its inputs, in `to_input` order, along `axis` (the last by default)."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        axis = concat_attributes_from_json(attributes).axis
        self.axis = DEFAULT_CONCAT_AXIS if axis is None else axis

    def forward(self, *inputs: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return torch.cat(inputs, dim=self.axis)
