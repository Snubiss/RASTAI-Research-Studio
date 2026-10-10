"""
In-house LoRA (spec v5 §10: "LoRA is native to ModelSpec: any node can be set to freeze, LoRA or train").

A LoRA adapter leaves a linear layer's weight W frozen and learns a low-rank correction:
    output = W·x + scale · expand(compress(x))
where compress maps the input down to `rank` numbers and expand maps them back up. expand starts at zero,
so a fresh adapter changes nothing until it has been trained.

Mixed precision: the adapter's master weights are float32, because tiny updates to a bf16 weight round away,
but the forward pass runs in the model's dtype. Upcasting every projection's input to float32 instead would
store a float32 copy of each activation for the backward pass, about 1 GB on a modest batch.
"""

import math

import torch
import torch.nn.functional as F
from torch import nn

from rastai_spike.gemma3 import Gemma3Model

# Every linear projection in a Gemma 3 block, by its path inside the block.
ALL_PROJECTIONS = (
    "attention.query_projection",
    "attention.key_projection",
    "attention.value_projection",
    "attention.output_projection",
    "feedforward.gate_projection",
    "feedforward.up_projection",
    "feedforward.down_projection",
)
ADAPTER_DTYPE = torch.float32


class LoRALinear(nn.Module):
    def __init__(self, base: nn.Linear, rank: int, alpha: float):
        super().__init__()
        self.base = base
        self.base.weight.requires_grad_(False)
        device = base.weight.device
        self.compress = nn.Parameter(torch.empty(rank, base.in_features, device=device, dtype=ADAPTER_DTYPE))
        self.expand = nn.Parameter(torch.zeros(base.out_features, rank, device=device, dtype=ADAPTER_DTYPE))
        # Same start as the usual LoRA recipe: random compress, zero expand.
        nn.init.kaiming_uniform_(self.compress, a=math.sqrt(5))
        self.scale = alpha / rank
        self.enabled = True

    def forward(self, values: torch.Tensor) -> torch.Tensor:
        output = self.base(values)
        if not self.enabled:
            return output
        compress = self.compress.to(values.dtype)  # gradients still flow back to the float32 master weights
        expand = self.expand.to(values.dtype)
        return output + F.linear(F.linear(values, compress), expand) * self.scale


def attach_lora(model: Gemma3Model, rank: int, alpha: float, projections=ALL_PROJECTIONS) -> dict[str, LoRALinear]:
    """Freeze the whole model and wrap the chosen projections of every block. Returns the adapters by name."""
    for parameter in model.parameters():
        parameter.requires_grad_(False)

    adapters = {}
    for layer_index, block in enumerate(model.blocks):
        for path in projections:
            parent_path, attribute = path.rsplit(".", 1)
            parent = block.get_submodule(parent_path)
            adapter = LoRALinear(getattr(parent, attribute), rank, alpha)
            setattr(parent, attribute, adapter)
            adapters[f"blocks.{layer_index}.{path}"] = adapter
    return adapters


def set_lora_enabled(adapters: dict[str, LoRALinear], enabled: bool) -> None:
    for adapter in adapters.values():
        adapter.enabled = enabled


def adapter_tensors(adapters: dict[str, LoRALinear]) -> dict[str, torch.Tensor]:
    """Only the trained weights, named so they can be loaded back onto the same projections."""
    tensors = {}
    for name, adapter in adapters.items():
        tensors[f"{name}.compress"] = adapter.compress.detach()
        tensors[f"{name}.expand"] = adapter.expand.detach()
    return tensors


def trainable_parameters(adapters: dict[str, LoRALinear]) -> list[nn.Parameter]:
    return [parameter for adapter in adapters.values() for parameter in (adapter.compress, adapter.expand)]
