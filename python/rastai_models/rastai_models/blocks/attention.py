"""The attention op (op `attention`, contracts 5.3): one sequence's causal self-attention with its own projections.

The variants are `mha` (one key and value head per query head), `gqa` (fewer key and value heads, shared by groups of
query heads) and `sliding` (each query sees only the last `window` positions). Queries and keys can be RMS-normalized
per head (`qk_norm`) and rotated by rotary embedding (`rope`). The op holds four projections without biases, so its
input and output have the same shape.

Observation (layout 2.2): the head outputs (before the output projection) are a point `<node path>.head_outputs`; under
full capture the attention pattern is `<node path>.attention_pattern`, and under last-token capture the newest query's
pattern is `<node path>.last_token_attention`. The points are emitted only when the pass has a hook.

A standalone call (no ForwardContext) runs the eager kernel with its matmuls written out, which is what the golden
FLOP counts describe (fixtures/ops/README.md).
"""

import math

import torch
from rastai_formats.generated.model import attention_attributes_from_json
from torch.nn import functional

from rastai_models.attention_kernel import (
    KERNEL_EAGER,
    apply_attention_weights,
    eager_attention_weights,
    run_attention,
    select_attention_kernel,
)
from rastai_models.blocks.norms import RmsNorm
from rastai_models.blocks.op_module import OpModule, fill_uniform, new_parameter
from rastai_models.blocks.rope import apply_rope, rope_frequencies, rotary_angles
from rastai_models.forward_context import ForwardContext
from rastai_models.observation import (
    ATTENTION_PATTERN_OUTPUT,
    ATTENTION_SIBLING_OUTPUTS,
    CAPTURE_FULL_ATTENTION,
    CAPTURE_LAST_TOKEN_ATTENTION,
    CAPTURE_NO_ATTENTION,
    HEAD_OUTPUTS_OUTPUT,
    LAST_TOKEN_ATTENTION_OUTPUT,
)

VARIANT_SLIDING = "sliding"
# Whether queries and keys are normalized per head when the settings leave it out.
DEFAULT_QK_NORM = False
# The scale of an RMS norm over the queries and keys: the rms_norm op's default, with no offset.
QK_NORM_SIZE_KEY = "size"


class Attention(OpModule):
    """Projections `query_weight` [head_count × head_dim, hidden_size], `key_weight` and `value_weight`
    [kv_head_count × head_dim, hidden_size], and `output_weight` [hidden_size, head_count × head_dim]. With `qk_norm`
    it also has `query_norm` and `key_norm`, RMS norms of head_dim."""

    def __init__(self, attributes: dict, device: str | torch.device, dtype: torch.dtype) -> None:
        super().__init__()
        parsed = attention_attributes_from_json(attributes)
        self.head_count = parsed.head_count
        self.kv_head_count = parsed.head_count if parsed.kv_head_count is None else parsed.kv_head_count
        self.head_dim = parsed.head_dim
        # Only a sliding layer has a window; the others attend to every earlier position.
        self.window = parsed.window if parsed.variant == VARIANT_SLIDING else None
        query_width = self.head_count * self.head_dim
        key_value_width = self.kv_head_count * self.head_dim
        hidden_size = parsed.hidden_size
        self.query_weight = new_parameter((query_width, hidden_size), device, dtype)
        self.key_weight = new_parameter((key_value_width, hidden_size), device, dtype)
        self.value_weight = new_parameter((key_value_width, hidden_size), device, dtype)
        self.output_weight = new_parameter((hidden_size, query_width), device, dtype)
        has_qk_norm = DEFAULT_QK_NORM if parsed.qk_norm is None else parsed.qk_norm
        self.query_norm = RmsNorm({QK_NORM_SIZE_KEY: self.head_dim}, device, dtype) if has_qk_norm else None
        self.key_norm = RmsNorm({QK_NORM_SIZE_KEY: self.head_dim}, device, dtype) if has_qk_norm else None
        self.has_rope = parsed.rope is not None
        self.attention_factor = 1.0
        if parsed.rope is not None:
            inverse_frequencies, self.attention_factor = rope_frequencies(parsed.rope, self.head_dim)
            # A buffer, so the tables move with the module. It is not a parameter, so it is neither trained nor counted.
            self.register_buffer("inverse_frequencies", inverse_frequencies.to(device), persistent=False)

    def forward(self, hidden: torch.Tensor, context: ForwardContext | None = None) -> torch.Tensor:
        batch_size, length, _ = hidden.shape
        queries = split_heads(functional.linear(hidden, self.query_weight), self.head_dim)
        keys = split_heads(functional.linear(hidden, self.key_weight), self.head_dim)
        values = split_heads(functional.linear(hidden, self.value_weight), self.head_dim)
        if self.query_norm is not None:
            queries = self.query_norm(queries)
            keys = self.key_norm(keys)
        if self.has_rope:
            queries, keys = self.rotate(queries, keys, length, hidden.device)
        mask = attention_mask(length, self.window, hidden.device, hidden.dtype)
        heads = self.attend(queries, keys, values, mask, context)
        heads = self.observe_output(context, HEAD_OUTPUTS_OUTPUT, heads)
        merged = heads.transpose(1, 2).reshape(batch_size, length, self.head_count * self.head_dim)
        return functional.linear(merged, self.output_weight)

    def attend(
        self,
        queries: torch.Tensor,
        keys: torch.Tensor,
        values: torch.Tensor,
        mask: torch.Tensor,
        context: ForwardContext | None,
    ) -> torch.Tensor:
        """The per-head attention outputs. The eager kernel gives the patterns, so it runs under full capture and
        the patterns are observed; SDPA gives the output and, under last-token capture, the newest query's pattern is
        computed on the side for the observer."""
        kernel, capture = pass_kernel_and_capture(context)
        if kernel == KERNEL_EAGER:
            weights = eager_attention_weights(queries, keys, mask)
            if capture == CAPTURE_FULL_ATTENTION:
                weights = self.observe_output(context, ATTENTION_PATTERN_OUTPUT, weights)
            return apply_attention_weights(weights, values)
        heads = run_attention(queries, keys, values, kernel, mask)
        if capture == CAPTURE_LAST_TOKEN_ATTENTION:
            # The side pattern is for the observer only: the output above was already computed without it.
            last_pattern = eager_attention_weights(queries[:, :, -1:], keys, mask[:, :, -1:, :])
            self.observe_output(context, LAST_TOKEN_ATTENTION_OUTPUT, last_pattern)
        return heads

    def rotate(
        self, queries: torch.Tensor, keys: torch.Tensor, length: int, device: torch.device
    ) -> tuple[torch.Tensor, torch.Tensor]:
        positions = torch.arange(length, device=device)
        cosine, sine = rotary_angles(positions, self.inverse_frequencies, self.attention_factor, queries.dtype)
        return apply_rope(queries, cosine, sine), apply_rope(keys, cosine, sine)

    def sibling_points(self) -> tuple[str, ...]:
        return ATTENTION_SIBLING_OUTPUTS

    def init_parameters(self, generator: torch.Generator) -> None:
        # Each projection starts as torch.nn.Linear does: uniform in ±1/sqrt(its input size).
        for weight in (self.query_weight, self.key_weight, self.value_weight, self.output_weight):
            fill_uniform(weight, 1.0 / math.sqrt(weight.shape[1]), generator)
        if self.query_norm is not None:
            self.query_norm.init_parameters(generator)
            self.key_norm.init_parameters(generator)


def pass_kernel_and_capture(context: ForwardContext | None) -> tuple[str, str]:
    """The kernel and capture mode of one attention call. A standalone call has no pass: eager, nothing captured."""
    if context is None:
        return KERNEL_EAGER, CAPTURE_NO_ATTENTION
    kernel = select_attention_kernel(context.attention_kernel, context.attention_capture)
    return kernel, context.attention_capture


def split_heads(projected: torch.Tensor, head_dim: int) -> torch.Tensor:
    """[batch, length, heads × head_dim] to [batch, heads, length, head_dim]."""
    batch_size, length, width = projected.shape
    return projected.reshape(batch_size, length, width // head_dim, head_dim).transpose(1, 2)


def attention_mask(length: int, window: int | None, device: torch.device, dtype: torch.dtype) -> torch.Tensor:
    """The additive mask of one pass, [1, 1, length, length]: zero where a query may look at a key, negative infinity
    where it may not. A query sees itself and the positions before it, and at most `window` positions when sliding."""
    positions = torch.arange(length, device=device)
    allowed = positions[None, :] <= positions[:, None]
    if window is not None:
        allowed = allowed & (positions[None, :] > positions[:, None] - window)
    zeros = torch.zeros((length, length), dtype=dtype, device=device)
    return zeros.masked_fill(~allowed, float("-inf"))[None, None]
