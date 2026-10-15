"""The normalizations over the last axis: RMSNorm (op `rms_norm`) and LayerNorm (op `layer_norm`), contracts 5.3.

RMSNorm computes in float32 and multiplies by the scale before it casts back down. Casting first loses precision in
bf16, and the Gemma 3 parity of spike 01 depends on this order (layout python.md 2.8). The `scale_offset` form adds
the learned scale to one, which is how Gemma 3 stores it.
"""

import torch
from rastai_formats.generated.model import layer_norm_attributes_from_json, rms_norm_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, copy_into, fill_constant, new_parameter
from rastai_models.forward_context import ForwardContext

# Added under the square root: Gemma 3's epsilon. A design that means another (Llama's 1e-5) sets it.
DEFAULT_RMS_EPSILON = 1e-6
DEFAULT_LAYER_EPSILON = 1e-5
DEFAULT_SCALE_OFFSET = False
DEFAULT_LAYER_BIAS = True


class RmsNorm(OpModule):
    """Scales each vector to unit root-mean-square, then by `weight` (size values)."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = rms_norm_attributes_from_json(attributes)
        self.epsilon = DEFAULT_RMS_EPSILON if parsed.epsilon is None else parsed.epsilon
        self.scale_offset = DEFAULT_SCALE_OFFSET if parsed.scale_offset is None else parsed.scale_offset
        self.weight = new_parameter((parsed.size,), device, dtype)

    def forward(self, values: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return self.normalize(values)

    def normalize(self, values: torch.Tensor) -> torch.Tensor:
        values_32 = values.float()
        normalized = values_32 * torch.rsqrt(values_32.pow(2).mean(-1, keepdim=True) + self.epsilon)
        scale = self.weight.float()
        if self.scale_offset:
            scale = 1.0 + scale
        # Multiply before casting down, as the reference does.
        return (normalized * scale).type_as(values)

    def init_parameters(self, generator: torch.Generator) -> None:
        # A plain scale starts at one; an offset scale starts at zero, which is one once the offset is added.
        fill_constant(self.weight, 0.0 if self.scale_offset else 1.0)


class LayerNorm(OpModule):
    """Centers and scales each vector over its last axis, then applies `weight` and, unless `bias` is false, `bias`."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = layer_norm_attributes_from_json(attributes)
        self.size = parsed.size
        self.epsilon = DEFAULT_LAYER_EPSILON if parsed.epsilon is None else parsed.epsilon
        has_bias = DEFAULT_LAYER_BIAS if parsed.bias is None else parsed.bias
        self.weight = new_parameter((parsed.size,), device, dtype)
        if has_bias:
            self.bias = new_parameter((parsed.size,), device, dtype)
        else:
            self.register_parameter("bias", None)

    def forward(self, values: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        return functional.layer_norm(values, (self.size,), self.weight, self.bias, self.epsilon)

    def init_parameters(self, generator: torch.Generator) -> None:
        # torch.nn.LayerNorm's starting values: a scale of one and a shift of zero.
        fill_constant(self.weight, 1.0)
        if self.bias is not None:
            copy_into(self.bias, torch.zeros(self.size, dtype=torch.float32))
