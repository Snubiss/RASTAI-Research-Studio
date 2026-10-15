"""The residual add (op `residual_add`, contracts 5.3): two inputs of one shape, summed.

It is the one op with a fixed count of two inputs (contracts 5.3, `TWO_INPUTS_OP`), so a graph wires its `to_input`
0 and 1 and nothing else.
"""

import torch

from rastai_models.blocks.op_module import OpModule
from rastai_models.forward_context import ForwardContext


class ResidualAdd(OpModule):
    """The sum of its two inputs. It holds no parameters, takes no attributes and counts no FLOPs."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()

    def forward(self, first: torch.Tensor, second: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return first + second
