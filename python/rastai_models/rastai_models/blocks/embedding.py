"""The token embedding (op `embedding`, contracts 5.3): token ids [...] → vectors [..., embedding_size].

The optional `scale` multiplies each vector. Gemma 3 uses the square root of embedding_size, and the scale is cast to
the weight's dtype before it multiplies, as the Gemma 3 reference does, so a bf16 model sees the rounded value.
"""

import torch
from rastai_formats.generated.model import embedding_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, fill_normal, new_parameter
from rastai_models.forward_context import ForwardContext

# The scale an embedding without a `scale` attribute uses: each vector is left as it is.
DEFAULT_SCALE = 1.0


class Embedding(OpModule):
    """One learned vector per token id: weight [vocab_size, embedding_size]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = embedding_attributes_from_json(attributes)
        self.weight = new_parameter((parsed.vocab_size, parsed.embedding_size), device, dtype)
        scale = DEFAULT_SCALE if parsed.scale is None else parsed.scale
        # A buffer, not a parameter: it is a constant of the design, not a trained value, so it is neither counted nor
        # saved with the weights. It is built here, not in forward, so a captured decode step copies nothing to the
        # device.
        self.register_buffer(
            "scale_value", torch.tensor(scale, dtype=torch.float32).to(device=device, dtype=dtype), persistent=False
        )

    def forward(self, token_ids: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return functional.embedding(token_ids, self.weight) * self.scale_value

    def init_parameters(self, generator: torch.Generator) -> None:
        # The standard normal, as torch.nn.Embedding starts.
        fill_normal(self.weight, generator)
