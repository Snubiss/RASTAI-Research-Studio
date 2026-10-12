"""OpModule: the base class of every graph node's computation (layout python.md section 2.1).

Each op module is built from its graph node's `attributes{}` (contracts 5.3) and holds its parameters directly, as
`weight` and `bias`, so a parameter's graph path is `<node_id>.weight` (CompiledModel.graph_parameters). An attribute
the node leaves out takes the default contracts 5.3 gives it; each block file names those defaults at its top.

Seeded init draws every value on the CPU in float32 from one torch.Generator and then copies it to the parameter's
device and dtype, so a seed gives the same weights on any device and in any number format.
"""

import torch
from torch import nn


class OpModule(nn.Module):
    """One graph node's computation. Subclasses take `(attributes, device, dtype)`, implement `forward` over the
    node's inputs in `to_input` order, and fill their parameters in `init_parameters`."""

    def forward(self, *inputs: torch.Tensor) -> torch.Tensor:
        raise NotImplementedError

    def init_parameters(self, generator: torch.Generator) -> None:
        """Fills the parameters from `generator` (a fresh design). An op without parameters keeps this."""


def new_parameter(shape: tuple[int, ...], device: str | torch.device, dtype: torch.dtype) -> nn.Parameter:
    """A parameter of zeros. Zeros, not torch.empty, so a model that is neither initialized nor bound still holds
    defined values; init_weights or bind_weights fills it before any real use."""
    return nn.Parameter(torch.zeros(shape, device=device, dtype=dtype))


def fill_uniform(parameter: nn.Parameter, bound: float, generator: torch.Generator) -> None:
    """Draws every value uniformly from [-bound, bound]."""
    values = torch.empty(parameter.shape, dtype=torch.float32).uniform_(-bound, bound, generator=generator)
    copy_into(parameter, values)


def fill_normal(parameter: nn.Parameter, generator: torch.Generator) -> None:
    """Draws every value from the standard normal distribution."""
    values = torch.empty(parameter.shape, dtype=torch.float32).normal_(0.0, 1.0, generator=generator)
    copy_into(parameter, values)


def copy_into(parameter: nn.Parameter, values: torch.Tensor) -> None:
    # Initialization is not part of the model's computation, so autograd must not record it.
    with torch.no_grad():
        parameter.copy_(values)
