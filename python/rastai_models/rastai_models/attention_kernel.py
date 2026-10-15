"""Which attention kernel runs, and how (layout python.md section 2.1, ADR-0027, ADR-0026).

SDPA is the default: torch's fused `scaled_dot_product_attention`, the fastest kernel. Its fast kernels sum in a
different order than the plain math, so in bf16 they can flip a near-tie between two tokens (spike 01b). An exact
replay therefore pins the math kernel or the eager kernel. Eager is also the only kernel that returns each query's
attention pattern, so a pass that captures full attention runs eager whatever was requested.
"""

import math

import torch
from torch.nn import functional
from torch.nn.attention import SDPBackend, sdpa_kernel

from rastai_models.observation import CAPTURE_FULL_ATTENTION, check_capture_mode

# torch's fused kernel, which the runtime picks: the default.
KERNEL_SDPA = "sdpa"
# SDPA pinned to its plain math backend, for an exact replay.
KERNEL_MATH = "math"
# softmax(q kᵀ / √d) v written out, for an exact replay and for captured maps.
KERNEL_EAGER = "eager"
KERNEL_NAMES = (KERNEL_SDPA, KERNEL_MATH, KERNEL_EAGER)


def select_attention_kernel(requested_kernel: str, attention_capture: str) -> str:
    """The kernel an attention op runs with for one pass.

    Full attention capture forces eager, because the maps it returns are eager's softmax output; SDPA doesn't return
    them. Otherwise the requested kernel stands. An unknown name is a bug in the caller, so it raises ValueError.
    """
    check_kernel_name(requested_kernel)
    check_capture_mode(attention_capture)
    if attention_capture == CAPTURE_FULL_ATTENTION:
        return KERNEL_EAGER
    return requested_kernel


def check_kernel_name(kernel: str) -> None:
    if kernel not in KERNEL_NAMES:
        raise ValueError(f"unknown attention kernel {kernel!r}; expected one of {KERNEL_NAMES}")


def sdpa_backend_pin(kernel: str) -> list[SDPBackend] | None:
    """The SDPA backends a kernel allows, or None when torch may choose any. Eager never calls SDPA: no pin."""
    if kernel == KERNEL_MATH:
        return [SDPBackend.MATH]
    return None


def run_attention(
    query: torch.Tensor, key: torch.Tensor, value: torch.Tensor, kernel: str, mask: torch.Tensor | None = None
) -> torch.Tensor:
    """Attention over [batch, heads, length, head_size] tensors with the chosen kernel. The key and value heads may be
    fewer than the query heads (grouped-query attention); the query heads of one group share a key head.

    `mask` is an additive float mask broadcast to the score matrix (a causal mask, or ALiBi's bias), so a masked
    position holds negative infinity. The eager path and the SDPA paths compute the same function; they differ in
    rounding, which is why the kernel is pinned for exact replay.
    """
    check_kernel_name(kernel)
    if kernel == KERNEL_EAGER:
        return eager_attention(query, key, value, mask)
    backends = sdpa_backend_pin(kernel)
    if backends is None:
        return functional.scaled_dot_product_attention(query, key, value, attn_mask=mask, enable_gqa=True)
    with sdpa_kernel(backends):
        return functional.scaled_dot_product_attention(query, key, value, attn_mask=mask, enable_gqa=True)


def eager_attention(
    query: torch.Tensor, key: torch.Tensor, value: torch.Tensor, mask: torch.Tensor | None = None
) -> torch.Tensor:
    """The attention output of the eager kernel: the weights from `eager_attention_weights`, times the values."""
    return apply_attention_weights(eager_attention_weights(query, key, mask), value)


def eager_attention_weights(query: torch.Tensor, key: torch.Tensor, mask: torch.Tensor | None = None) -> torch.Tensor:
    """Each query's attention pattern, [batch, query_heads, query_length, key_length]: the softmax of the scaled scores.

    The softmax runs in float32 and casts back, as the Gemma 3 precision details require (layout 2.8), so a bf16 pass
    rounds once, after the normalization.
    """
    head_size = query.shape[-1]
    batch_size, query_head_count, query_length, _ = query.shape
    grouped_query = group_query_heads(query, key.shape[1])
    scores = grouped_query @ key.transpose(-2, -1) / math.sqrt(head_size)
    scores = scores.reshape(batch_size, query_head_count, query_length, key.shape[2])
    if mask is not None:
        scores = scores + mask
    return torch.softmax(scores.float(), dim=-1).to(query.dtype)


def apply_attention_weights(weights: torch.Tensor, value: torch.Tensor) -> torch.Tensor:
    """The weighted sum of the values: [batch, query_heads, query_length, head_size]. Grouped like the scores, so the
    shared value heads are never copied."""
    batch_size, query_head_count, query_length, key_length = weights.shape
    value_head_count = value.shape[1]
    grouped_weights = weights.reshape(batch_size, value_head_count, -1, key_length)
    mixed = grouped_weights @ value
    return mixed.reshape(batch_size, query_head_count, query_length, value.shape[-1])


def group_query_heads(query: torch.Tensor, key_head_count: int) -> torch.Tensor:
    """The query heads that share one key head stacked along the length axis, so one matmul per key head does the
    group. The query heads of a group are consecutive, as repeat_interleave would have them. A view when the query is
    contiguous; no key or value head is copied."""
    batch_size, query_head_count, query_length, head_size = query.shape
    group_size = query_head_count // key_head_count
    return query.reshape(batch_size, key_head_count, group_size * query_length, head_size)
