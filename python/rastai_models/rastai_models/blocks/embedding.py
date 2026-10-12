"""The token embedding (op `embedding`, contracts 5.3): token ids [...] → vectors [..., embedding_size].

Gemma's embedding scale joins with the decoder families (M2).
"""

import torch
from rastai_formats.generated.model import embedding_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, fill_normal, new_parameter


class Embedding(OpModule):
    """One learned vector per token id: weight [vocab_size, embedding_size]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = embedding_attributes_from_json(attributes)
        self.weight = new_parameter((parsed.vocab_size, parsed.embedding_size), device, dtype)

    def forward(self, token_ids: torch.Tensor) -> torch.Tensor:
        return functional.embedding(token_ids, self.weight)

    def init_parameters(self, generator: torch.Generator) -> None:
        # The standard normal, as torch.nn.Embedding starts.
        fill_normal(self.weight, generator)
