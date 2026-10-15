"""The unembedding (op `unembed`, contracts 5.3): [..., hidden_size] to the logits [..., vocab_size].

Untied, the op holds its own weight [vocab_size, hidden_size]. Tied, the weight is the embedding's: the op holds no
parameter of its own, and the compiler gives it the embedding's weight with `bind_tied_weight`. With `final_softcap`
each logit is capped to within the cap, by cap × tanh(logit / cap), as Gemma 2 and 3 do.
"""

import math

import torch
from rastai_formats.generated.model import unembed_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, fill_uniform, new_parameter
from rastai_models.forward_context import ForwardContext

DEFAULT_TIED = False


class Unembed(OpModule):
    """The logits of a normalized hidden state. Untied: `weight` [vocab_size, hidden_size]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = unembed_attributes_from_json(attributes)
        self.is_tied = DEFAULT_TIED if parsed.tied is None else parsed.tied
        self.hidden_size = parsed.hidden_size
        self.vocab_size = parsed.vocab_size
        self.softcap = parsed.final_softcap
        self.device = torch.device(device)
        self.dtype = dtype
        if not self.is_tied:
            self.weight = new_parameter((parsed.vocab_size, parsed.hidden_size), device, dtype)

    def forward(self, hidden: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        logits = functional.linear(hidden, self.current_weight())
        if self.softcap is not None:
            logits = torch.tanh(logits / self.softcap) * self.softcap
        return logits

    def current_weight(self) -> torch.Tensor:
        """The matrix the logits use. A tied op that was never bound (only a standalone call, such as a golden check)
        uses zeros, so it still has the output shape; a compiled design binds the embedding's weight first."""
        if not self.is_tied:
            return self.weight
        bound = self.__dict__.get("tied_weight")
        if bound is not None:
            return bound
        return torch.zeros((self.vocab_size, self.hidden_size), dtype=self.dtype, device=self.device)

    def bind_tied_weight(self, weight: torch.Tensor) -> None:
        """Uses `weight` (the embedding's parameter) as this tied op's matrix. It is kept outside nn.Module's
        registration: the embedding owns the parameter, so it is counted and saved once, under the embedding."""
        if not self.is_tied:
            raise ValueError("bind_tied_weight is for a tied unembed; an untied one has its own weight")
        self.__dict__["tied_weight"] = weight

    def init_parameters(self, generator: torch.Generator) -> None:
        # torch.nn.Linear's starting distribution for a weight with `hidden_size` inputs.
        if not self.is_tied:
            fill_uniform(self.weight, 1.0 / math.sqrt(self.hidden_size), generator)
