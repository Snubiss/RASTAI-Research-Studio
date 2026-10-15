"""The feed-forward sub-layer (op `feed_forward`, contracts 5.3): [..., hidden_size] to the same shape.

`dense` is up, activation, down. `gated` is the activated gate times the up projection, then down, as Gemma 3 and
Llama do. No projection has a bias: a decoder op has none (contracts 5.3). The activation is the `activation` op's
function of the same name, read from basic_ops.py.
"""

import math

import torch
from rastai_formats.generated.model import feed_forward_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.basic_ops import ACTIVATION_FUNCTIONS
from rastai_models.blocks.op_module import OpModule, fill_uniform, new_parameter
from rastai_models.forward_context import ForwardContext

VARIANT_DENSE = "dense"
VARIANT_GATED = "gated"


class FeedForward(OpModule):
    """Projections `up_weight` [intermediate_size, hidden_size] and `down_weight` [hidden_size, intermediate_size];
    a gated op adds `gate_weight`, the same shape as `up_weight`."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = feed_forward_attributes_from_json(attributes)
        self.is_gated = parsed.variant == VARIANT_GATED
        self.activation = ACTIVATION_FUNCTIONS[parsed.activation]
        hidden_size, intermediate_size = parsed.hidden_size, parsed.intermediate_size
        self.up_weight = new_parameter((intermediate_size, hidden_size), device, dtype)
        self.down_weight = new_parameter((hidden_size, intermediate_size), device, dtype)
        if self.is_gated:
            self.gate_weight = new_parameter((intermediate_size, hidden_size), device, dtype)

    def forward(self, hidden: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        if self.is_gated:
            gate = self.activation(functional.linear(hidden, self.gate_weight))
            return functional.linear(gate * functional.linear(hidden, self.up_weight), self.down_weight)
        return functional.linear(self.activation(functional.linear(hidden, self.up_weight)), self.down_weight)

    def init_parameters(self, generator: torch.Generator) -> None:
        # Each projection starts as torch.nn.Linear does: uniform in ±1/sqrt(its input size).
        projections = [self.up_weight, self.down_weight]
        if self.is_gated:
            projections.append(self.gate_weight)
        for weight in projections:
            fill_uniform(weight, 1.0 / math.sqrt(weight.shape[1]), generator)
