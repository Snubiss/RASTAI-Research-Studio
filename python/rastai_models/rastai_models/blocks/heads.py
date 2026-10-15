"""Heads (layout python.md section 2.1). The classifier head (op `classifier_head`, contracts 5.3) gives the logits of
a classification objective: [..., in_features] → [..., class_count]. The scalar head (reward and value heads) joins
with RLHF (M5).
"""

import torch
from rastai_formats.generated.model import classifier_head_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.linear import DEFAULT_BIAS, add_affine_parameters, init_affine_parameters
from rastai_models.blocks.op_module import OpModule
from rastai_models.forward_context import ForwardContext


class ClassifierHead(OpModule):
    """A linear map to one logit per class: weight [class_count, in_features] and, unless `bias` is false,
    bias [class_count]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = classifier_head_attributes_from_json(attributes)
        has_bias = DEFAULT_BIAS if parsed.bias is None else parsed.bias
        add_affine_parameters(self, parsed.in_features, parsed.class_count, has_bias, device, dtype)

    def forward(self, features: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return functional.linear(features, self.weight, self.bias)

    def init_parameters(self, generator: torch.Generator) -> None:
        init_affine_parameters(self.weight, self.bias, generator)
