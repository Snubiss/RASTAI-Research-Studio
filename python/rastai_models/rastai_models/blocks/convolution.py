"""Convolution and pooling over the length axis (ops `conv1d` and `pool`, contracts 5.3).

Sequence tensors are channels-last, [batch, length, channels], the layout an embedding gives (contracts 5.3), while
torch's conv1d and pooling want [batch, channels, length]. So each op transposes on the way in and back on the way
out, and no transpose op is needed in the graph. conv2d (ViT patch embedding) joins with the ViT family.
"""

import math

import torch
from rastai_formats.generated.model import conv1d_attributes_from_json, pool_attributes_from_json
from torch.nn import functional

from rastai_models.blocks.op_module import OpModule, fill_uniform, new_parameter

DEFAULT_STRIDE = 1
DEFAULT_PADDING = 0
DEFAULT_DILATION = 1
DEFAULT_BIAS = True
POOL_MAX = "max"
POOL_MEAN = "mean"
LENGTH_AXIS = 1


class Conv1d(OpModule):
    """[batch, length, in_channels] → [batch, out_length, out_channels]: weight [out_channels, in_channels,
    kernel_size] and, unless `bias` is false, bias [out_channels]."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = conv1d_attributes_from_json(attributes)
        self.stride = DEFAULT_STRIDE if parsed.stride is None else parsed.stride
        self.padding = DEFAULT_PADDING if parsed.padding is None else parsed.padding
        self.dilation = DEFAULT_DILATION if parsed.dilation is None else parsed.dilation
        has_bias = DEFAULT_BIAS if parsed.bias is None else parsed.bias
        weight_shape = (parsed.out_channels, parsed.in_channels, parsed.kernel_size)
        self.weight = new_parameter(weight_shape, device, dtype)
        if has_bias:
            self.bias = new_parameter((parsed.out_channels,), device, dtype)
        else:
            self.register_parameter("bias", None)

    def forward(self, sequence: torch.Tensor) -> torch.Tensor:
        channels_first = sequence.transpose(1, 2)
        convolved = functional.conv1d(channels_first, self.weight, self.bias, self.stride, self.padding, self.dilation)
        return convolved.transpose(1, 2)

    def init_parameters(self, generator: torch.Generator) -> None:
        # torch.nn.Conv1d's starting distribution: uniform in ±1/sqrt(fan_in), where fan_in counts every input value
        # one output sees (in_channels × kernel_size).
        in_channels, kernel_size = self.weight.shape[1], self.weight.shape[2]
        bound = 1.0 / math.sqrt(in_channels * kernel_size)
        fill_uniform(self.weight, bound, generator)
        if self.bias is not None:
            fill_uniform(self.bias, bound, generator)


class Pool(OpModule):
    """Max or mean over the length axis. Without `kernel_size` it pools the whole length, [batch, length, channels]
    → [batch, channels]; with it, windows of that length every `stride` positions (the window by default)."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = pool_attributes_from_json(attributes)
        self.mode = parsed.mode
        self.kernel_size = parsed.kernel_size
        self.stride = parsed.kernel_size if parsed.stride is None else parsed.stride

    def forward(self, sequence: torch.Tensor) -> torch.Tensor:
        if self.kernel_size is None:
            return pool_whole_length(sequence, self.mode)
        return pool_windows(sequence, self.mode, self.kernel_size, self.stride)


def pool_whole_length(sequence: torch.Tensor, mode: str) -> torch.Tensor:
    if mode == POOL_MAX:
        return sequence.amax(dim=LENGTH_AXIS)
    return sequence.mean(dim=LENGTH_AXIS)


def pool_windows(sequence: torch.Tensor, mode: str, kernel_size: int, stride: int) -> torch.Tensor:
    channels_first = sequence.transpose(1, 2)
    if mode == POOL_MAX:
        pooled = functional.max_pool1d(channels_first, kernel_size, stride)
    else:
        pooled = functional.avg_pool1d(channels_first, kernel_size, stride)
    return pooled.transpose(1, 2)
