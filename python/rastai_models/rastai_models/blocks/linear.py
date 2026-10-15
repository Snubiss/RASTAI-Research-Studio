"""The linear map (op `linear`, contracts 5.3): [..., in_features] → [..., out_features].

The functions below the class are shared with the classifier head (heads.py), which is the same map under another
op name.
"""

import math

import torch
from rastai_formats.generated.model import linear_attributes_from_json
from torch import nn
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, fill_uniform, new_parameter
from rastai_models.forward_context import ForwardContext

DEFAULT_BIAS = True


class Linear(OpModule):
    """weight [out_features, in_features] and, unless `bias` is false, bias [out_features]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = linear_attributes_from_json(attributes)
        has_bias = DEFAULT_BIAS if parsed.bias is None else parsed.bias
        add_affine_parameters(self, parsed.in_features, parsed.out_features, has_bias, device, dtype)

    def forward(self, features: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return functional.linear(features, self.weight, self.bias)

    def init_parameters(self, generator: torch.Generator) -> None:
        init_affine_parameters(self.weight, self.bias, generator)


def add_affine_parameters(
    module: nn.Module,
    in_features: int,
    out_features: int,
    has_bias: bool,
    device: str | torch.device,
    dtype: torch.dtype,
) -> None:
    """Registers `weight` and `bias` on `module`. A missing bias is registered as None, so `module.bias` exists
    either way and functional.linear skips it."""
    module.weight = new_parameter((out_features, in_features), device, dtype)
    if has_bias:
        module.bias = new_parameter((out_features,), device, dtype)
    else:
        module.register_parameter("bias", None)


def init_affine_parameters(weight: nn.Parameter, bias: nn.Parameter | None, generator: torch.Generator) -> None:
    """torch.nn.Linear's starting distribution: weight and bias uniform in ±1/sqrt(in_features). (Its weight init,
    kaiming_uniform_ with a = sqrt(5), works out to that same bound.)"""
    in_features = weight.shape[1]
    bound = 1.0 / math.sqrt(in_features)
    fill_uniform(weight, bound, generator)
    if bias is not None:
        fill_uniform(bias, bound, generator)
