"""ForwardContext: the per-pass inputs that every op module reads (layout python.md section 2.2).

One pass of the model takes one context, or none for a plain forward. The context says which attention kernel runs,
how much attention is captured, which hook sees the points, and which outputs the caller wants. It is plain data: the
caller builds it for each pass and the model reads it, never keeps it.

The layout also lists the KV caches (`caches`). They join this class with the KV caches of M2-04, so until then a pass
has no caches to read.
"""

from dataclasses import dataclass, field

import torch

from rastai_models.attention_kernel import KERNEL_SDPA
from rastai_models.observation import CAPTURE_NO_ATTENTION, PointHook


@dataclass
class ForwardContext:
    # A 1-element GPU tensor holding the position of a static decode step (layout 2.3). None for a prefill or a
    # full-sequence pass.
    decode_position: torch.Tensor | None = None
    # The attention kernel the caller asks for; select_attention_kernel may change it when attention is captured.
    attention_kernel: str = KERNEL_SDPA
    attention_capture: str = CAPTURE_NO_ATTENTION
    # Sees every observation point in graph order and returns the tensor the model continues with. None: no hook.
    hook: PointHook | None = None
    # The output head reads only the last position (decode and last-token readouts). Read by the decoder heads (M2-03).
    last_position_only: bool = False
    # False: return the final normalized hidden state, not the logits (the fused logit lens). Read by unembed (M2-03).
    unembed_output: bool = True
    # Extra losses an op adds during the pass, such as the MoE load-balancing loss. The trainer adds them to the loss.
    auxiliary_losses: list[torch.Tensor] = field(default_factory=list)
