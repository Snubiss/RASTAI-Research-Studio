"""The decoder blocks (layout python.md sections 2.1 and 2.7, M2-03). Each op module equals a plain fp32 reference
computation; the attention capture modes agree with each other; grouped-query attention copies no key head; the causal
and sliding masks hold; and the rope settings follow their formulas.

The golden shapes, parameter counts and FLOPs of every op are checked by test_op_fixtures.py, not here.
"""

import math

import pytest
import torch
from rastai_formats.generated.model import RopeSettings
from rastai_models.attention_kernel import KERNEL_EAGER, KERNEL_MATH, KERNEL_SDPA, eager_attention
from rastai_models.blocks.attention import Attention, attention_mask, split_heads
from rastai_models.blocks.embedding import Embedding
from rastai_models.blocks.feed_forward import FeedForward
from rastai_models.blocks.norms import LayerNorm, RmsNorm
from rastai_models.blocks.residual import ResidualAdd
from rastai_models.blocks.rope import apply_rope, rope_frequencies, rotary_angles
from rastai_models.blocks.unembed import Unembed
from rastai_models.errors import ModelError
from rastai_models.forward_context import ForwardContext
from rastai_models.observation import (
    ATTENTION_PATTERN_OUTPUT,
    CAPTURE_FULL_ATTENTION,
    CAPTURE_LAST_TOKEN_ATTENTION,
    HEAD_OUTPUTS_OUTPUT,
    LAST_TOKEN_ATTENTION_OUTPUT,
    PointHook,
)
from torch.nn import functional
from torch.utils._python_dispatch import TorchDispatchMode
from torch.utils._pytree import tree_flatten

SEED = 7
# float32 math on small tensors: the implementations differ only in the order of rounding.
TOLERANCE = 1e-5
NODE_PATH = "blocks.0.attention"
BATCH_SIZE = 2
LENGTH = 6
HIDDEN_SIZE = 16
HEAD_COUNT = 4
KV_HEAD_COUNT = 2
HEAD_DIM = 8
# Llama 3's base frequency: a base other than the default, so the rope test can tell the two apart.
ROPE_BASE = 500000.0
# The epsilon of the RMS norm the reference uses: the rms_norm op's default.
RMS_EPSILON = 1e-6
# Matrix products read their operands and write a new result; they are the one kind of op allowed to do that to a
# key or value head.
MATRIX_PRODUCT_NAMES = ("bmm", "mm")


class CapturingHook(PointHook):
    """Keeps a copy of every point's tensor by name and returns each tensor unchanged."""

    def __init__(self) -> None:
        self.tensors: dict[str, torch.Tensor] = {}
        self.point_names: list[str] = []

    def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
        self.point_names.append(point_name)
        self.tensors[point_name] = tensor.detach().clone()
        return tensor


def random_tensor(shape: tuple[int, ...], seed: int) -> torch.Tensor:
    return torch.randn(shape, generator=torch.Generator().manual_seed(seed))


def gqa_attributes(**extra: object) -> dict:
    attributes = {
        "variant": "gqa",
        "hidden_size": HIDDEN_SIZE,
        "head_count": HEAD_COUNT,
        "kv_head_count": KV_HEAD_COUNT,
        "head_dim": HEAD_DIM,
    }
    attributes.update(extra)
    return attributes


def built_attention(attributes: dict) -> Attention:
    module = Attention(attributes, "cpu", torch.float32)
    module.init_parameters(torch.Generator().manual_seed(SEED))
    module.node_path = NODE_PATH
    module.eval()
    return module


def reference_rotate(values: torch.Tensor, base: float) -> torch.Tensor:
    """Rotary embedding written out: each head vector's pair (i, i + head_dim / 2) turns by position times
    base^(-2i / head_dim) radians, one pair at a time. `values` is [..., length, head_dim]."""
    length, width = values.shape[-2], values.shape[-1]
    half = width // 2
    rotated = torch.zeros_like(values)
    for position in range(length):
        for pair in range(half):
            angle = position * base ** (-2.0 * pair / width)
            cosine = math.cos(angle)
            sine = math.sin(angle)
            first = values[..., position, pair]
            second = values[..., position, pair + half]
            rotated[..., position, pair] = first * cosine - second * sine
            rotated[..., position, pair + half] = second * cosine + first * sine
    return rotated


def reference_head_norm(values: torch.Tensor, weight: torch.Tensor) -> torch.Tensor:
    """An RMS norm over each head vector, then the scale `weight`; `values` is [..., head_dim]."""
    return values / torch.sqrt(values.pow(2).mean(-1, keepdim=True) + RMS_EPSILON) * weight


def reference_attention(
    module: Attention, hidden: torch.Tensor, window: int | None = None, rope_base: float | None = None
) -> torch.Tensor:
    """Attention written out per query position and per head, in plain loops: no grouped reshapes and no repeats. A
    query sees the positions up to itself, and from `window` back when the layer slides. With a module that has
    qk_norm, each query and key head is normalized first; with `rope_base`, each is rotated by position."""
    batch_size, length, _ = hidden.shape
    group_size = module.head_count // module.kv_head_count
    queries = (hidden @ module.query_weight.T).reshape(batch_size, length, module.head_count, HEAD_DIM)
    keys = (hidden @ module.key_weight.T).reshape(batch_size, length, module.kv_head_count, HEAD_DIM)
    values = (hidden @ module.value_weight.T).reshape(batch_size, length, module.kv_head_count, HEAD_DIM)
    if module.query_norm is not None:
        queries = reference_head_norm(queries, module.query_norm.weight)
        keys = reference_head_norm(keys, module.key_norm.weight)
    if rope_base is not None:
        queries = reference_rotate(queries.transpose(1, 2), rope_base).transpose(1, 2)
        keys = reference_rotate(keys.transpose(1, 2), rope_base).transpose(1, 2)
    heads = torch.zeros(batch_size, length, module.head_count, HEAD_DIM)
    for head in range(module.head_count):
        key_head = head // group_size
        for position in range(length):
            first = 0 if window is None else max(0, position - window + 1)
            visible_keys = keys[:, first : position + 1, key_head]
            visible_values = values[:, first : position + 1, key_head]
            scores = (queries[:, position, head][:, None, :] * visible_keys).sum(-1) / math.sqrt(HEAD_DIM)
            weights = torch.softmax(scores, dim=-1)
            heads[:, position, head] = (weights[..., None] * visible_values).sum(1)
    merged = heads.reshape(batch_size, length, module.head_count * HEAD_DIM)
    return merged @ module.output_weight.T


def sample_hidden(seed: int = SEED) -> torch.Tensor:
    return random_tensor((BATCH_SIZE, LENGTH, HIDDEN_SIZE), seed)


# ---- attention against the reference ----


@pytest.mark.parametrize(
    "attributes",
    [
        gqa_attributes(),
        gqa_attributes(variant="mha", kv_head_count=HEAD_COUNT),
    ],
    ids=["gqa", "mha"],
)
def test_attention_equals_the_reference_on_both_kernels(attributes: dict) -> None:
    module = built_attention(attributes)
    hidden = sample_hidden()
    expected = reference_attention(module, hidden)
    standalone = module(hidden)
    with_sdpa = module(hidden, context=ForwardContext())
    assert torch.allclose(standalone, expected, atol=TOLERANCE)
    assert torch.allclose(with_sdpa, expected, atol=TOLERANCE)


def test_sliding_attention_equals_the_windowed_reference() -> None:
    window = 3
    module = built_attention(gqa_attributes(variant="sliding", window=window))
    hidden = sample_hidden()
    expected = reference_attention(module, hidden, window=window)
    assert torch.allclose(module(hidden), expected, atol=TOLERANCE)


def test_qk_norm_adds_two_head_norms_with_unit_scale_at_the_start() -> None:
    module = built_attention(gqa_attributes(qk_norm=True))
    assert module.query_norm is not None and module.key_norm is not None
    assert torch.equal(module.query_norm.weight, torch.ones(HEAD_DIM))
    assert torch.equal(module.key_norm.weight, torch.ones(HEAD_DIM))


@pytest.mark.parametrize(
    "attributes",
    [
        gqa_attributes(qk_norm=True, rope={"base": ROPE_BASE}),
        gqa_attributes(variant="sliding", window=3, qk_norm=True, rope={"base": ROPE_BASE}),
    ],
    ids=["qk_norm_and_rope", "sliding_qk_norm_and_rope"],
)
def test_qk_norm_and_rope_equal_the_written_out_reference(attributes: dict) -> None:
    module = built_attention(attributes)
    # Random norm scales, so a norm that ignores its scale can't match the reference.
    module.query_norm.weight.data = random_tensor((HEAD_DIM,), SEED + 3)
    module.key_norm.weight.data = random_tensor((HEAD_DIM,), SEED + 4)
    expected = reference_attention(module, sample_hidden(), window=attributes.get("window"), rope_base=ROPE_BASE)
    assert torch.allclose(module(sample_hidden()), expected, atol=TOLERANCE)
    assert torch.allclose(module(sample_hidden(), context=ForwardContext()), expected, atol=TOLERANCE)


# ---- masks ----


def test_a_query_does_not_see_later_positions() -> None:
    module = built_attention(gqa_attributes())
    hidden = sample_hidden()
    changed = hidden.clone()
    changed[:, -1] += 5.0
    before = module(hidden)
    after = module(changed)
    # Every position before the changed one is computed from the same inputs, so its output is unchanged.
    assert torch.allclose(before[:, :-1], after[:, :-1], atol=TOLERANCE)


def test_a_sliding_query_does_not_see_positions_outside_its_window() -> None:
    window = 4
    module = built_attention(gqa_attributes(variant="sliding", window=window))
    hidden = random_tensor((1, 8, HIDDEN_SIZE), SEED)
    changed = hidden.clone()
    changed[:, 0] += 5.0
    before = module(hidden)
    after = module(changed)
    # The last position sees positions 4 to 7 only, so position 0 can't reach it.
    assert torch.allclose(before[:, -1], after[:, -1], atol=TOLERANCE)
    # Position 3 still sees position 0, so its output does change.
    assert not torch.allclose(before[:, 3], after[:, 3], atol=TOLERANCE)


# ---- observation points of the attention node ----


def test_full_capture_pattern_is_a_distribution_over_the_visible_positions() -> None:
    module = built_attention(gqa_attributes())
    hook = CapturingHook()
    module(sample_hidden(), context=ForwardContext(attention_capture=CAPTURE_FULL_ATTENTION, hook=hook))
    pattern = hook.tensors[f"{NODE_PATH}.{ATTENTION_PATTERN_OUTPUT}"]
    assert pattern.shape == (BATCH_SIZE, HEAD_COUNT, LENGTH, LENGTH)
    assert torch.allclose(pattern.sum(-1), torch.ones(BATCH_SIZE, HEAD_COUNT, LENGTH), atol=TOLERANCE)
    future = torch.triu(torch.ones(LENGTH, LENGTH, dtype=torch.bool), diagonal=1)
    assert torch.equal(pattern[..., future], torch.zeros_like(pattern[..., future]))


def test_last_token_capture_equals_the_last_row_of_the_full_pattern() -> None:
    module = built_attention(gqa_attributes())
    hidden = sample_hidden()
    full_hook = CapturingHook()
    module(hidden, context=ForwardContext(attention_capture=CAPTURE_FULL_ATTENTION, hook=full_hook))
    last_hook = CapturingHook()
    module(hidden, context=ForwardContext(attention_capture=CAPTURE_LAST_TOKEN_ATTENTION, hook=last_hook))
    full_pattern = full_hook.tensors[f"{NODE_PATH}.{ATTENTION_PATTERN_OUTPUT}"]
    last_pattern = last_hook.tensors[f"{NODE_PATH}.{LAST_TOKEN_ATTENTION_OUTPUT}"]
    assert last_pattern.shape == (BATCH_SIZE, HEAD_COUNT, 1, LENGTH)
    assert torch.allclose(last_pattern, full_pattern[:, :, -1:, :], atol=TOLERANCE)


@pytest.mark.parametrize("kernel", [KERNEL_SDPA, KERNEL_MATH, KERNEL_EAGER])
def test_last_token_capture_is_observed_on_every_kernel(kernel: str) -> None:
    """A pinned kernel still gets the live panel's pattern: eager slices its own, math and SDPA compute the row."""
    module = built_attention(gqa_attributes())
    hidden = sample_hidden()
    full_hook = CapturingHook()
    module(hidden, context=ForwardContext(attention_capture=CAPTURE_FULL_ATTENTION, hook=full_hook))
    last_hook = CapturingHook()
    output = module(
        hidden,
        context=ForwardContext(attention_kernel=kernel, attention_capture=CAPTURE_LAST_TOKEN_ATTENTION, hook=last_hook),
    )
    last_pattern = last_hook.tensors[f"{NODE_PATH}.{LAST_TOKEN_ATTENTION_OUTPUT}"]
    full_pattern = full_hook.tensors[f"{NODE_PATH}.{ATTENTION_PATTERN_OUTPUT}"]
    assert torch.allclose(last_pattern, full_pattern[:, :, -1:, :], atol=TOLERANCE)
    assert torch.allclose(output, module(hidden), atol=TOLERANCE)


def test_head_outputs_edit_reaches_the_output_projection() -> None:
    module = built_attention(gqa_attributes())

    class ZeroHeads(PointHook):
        def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
            return torch.zeros_like(tensor) if point_name == f"{NODE_PATH}.{HEAD_OUTPUTS_OUTPUT}" else tensor

    ablated = module(sample_hidden(), context=ForwardContext(hook=ZeroHeads()))
    # With every head zeroed and no output bias, the output projection gives zeros.
    assert torch.equal(ablated, torch.zeros_like(ablated))


class CopyRecorder(TorchDispatchMode):
    """Records each copy of a watched tensor: an operation that reads a watched storage and writes a fresh one,
    unless it is a matrix product, which reads its operands by design. A copy of a key or value head is such an
    operation (repeat, cat, clone, a copying reshape); a view is not. `copy_sizes` holds the element count of each
    copy's largest result."""

    def __init__(self, watched: list[torch.Tensor]) -> None:
        super().__init__()
        self.watched_pointers = {tensor.untyped_storage().data_ptr() for tensor in watched}
        self.copy_sizes: list[int] = []

    def __torch_dispatch__(self, func, types, args=(), kwargs=None):
        result = func(*args, **(kwargs or {}))
        read_pointers = set()
        for item in tree_flatten((args, kwargs or {}))[0]:
            if isinstance(item, torch.Tensor):
                read_pointers.add(item.untyped_storage().data_ptr())
        if read_pointers & self.watched_pointers:
            written = [item for item in tree_flatten(result)[0] if isinstance(item, torch.Tensor)]
            fresh = [item for item in written if item.untyped_storage().data_ptr() not in read_pointers]
            operation_name = str(func).split(".")[1]
            if fresh and operation_name not in MATRIX_PRODUCT_NAMES:
                self.copy_sizes.append(max(item.numel() for item in fresh))
        return result


def test_gqa_attention_copies_no_key_or_value_head() -> None:
    """Grouped-query attention shares each key and value head among its query heads, so nothing it does to the keys
    or values allocates more than the key and value tensors hold. The keys and values are the transposed projections
    that split_heads gives a real pass, not contiguous tensors, so the copies the matmuls make are the real pass's. A
    copy of each head per query head (repeat_interleave) would be group_size times larger and fails the bound."""
    queries = split_heads(random_tensor((BATCH_SIZE, LENGTH, HEAD_COUNT * HEAD_DIM), SEED), HEAD_DIM)
    keys = split_heads(random_tensor((BATCH_SIZE, LENGTH, KV_HEAD_COUNT * HEAD_DIM), SEED + 1), HEAD_DIM)
    values = split_heads(random_tensor((BATCH_SIZE, LENGTH, KV_HEAD_COUNT * HEAD_DIM), SEED + 2), HEAD_DIM)
    assert not keys.is_contiguous()
    mask = attention_mask(LENGTH, None, "cpu", torch.float32)
    recorder = CopyRecorder([keys, values])
    with recorder:
        output = eager_attention(queries, keys, values, mask)
    assert output.shape == (BATCH_SIZE, HEAD_COUNT, LENGTH, HEAD_DIM)
    key_value_size = BATCH_SIZE * KV_HEAD_COUNT * LENGTH * HEAD_DIM
    assert max(recorder.copy_sizes, default=0) <= key_value_size


# ---- rope ----


def test_rope_at_position_zero_leaves_every_head_vector_unchanged() -> None:
    inverse, attention_factor = rope_frequencies(RopeSettings(), HEAD_DIM)
    cosine, sine = rotary_angles(torch.arange(1), inverse, attention_factor, torch.float32)
    values = random_tensor((1, 1, 1, HEAD_DIM), SEED)
    assert torch.allclose(apply_rope(values, cosine, sine), values, atol=TOLERANCE)


def test_rope_rotation_keeps_the_length_of_every_head_vector() -> None:
    inverse, attention_factor = rope_frequencies(RopeSettings(base=500000.0), HEAD_DIM)
    cosine, sine = rotary_angles(torch.arange(LENGTH), inverse, attention_factor, torch.float32)
    values = random_tensor((BATCH_SIZE, 2, LENGTH, HEAD_DIM), SEED)
    rotated = apply_rope(values, cosine, sine)
    assert torch.allclose(rotated.norm(dim=-1), values.norm(dim=-1), atol=TOLERANCE)


def test_linear_scaling_divides_every_frequency_by_the_factor() -> None:
    plain, _ = rope_frequencies(RopeSettings(), HEAD_DIM)
    scaled, attention_factor = rope_frequencies(RopeSettings(scaling="linear", factor=4.0), HEAD_DIM)
    assert torch.allclose(scaled, plain / 4.0, atol=TOLERANCE)
    assert attention_factor == 1.0


def test_llama3_scaling_keeps_short_wavelengths_and_divides_long_ones() -> None:
    original_length = 8192
    settings = RopeSettings(scaling="llama3", factor=8.0, original_max_position=original_length)
    plain, _ = rope_frequencies(RopeSettings(), 64)
    scaled, _ = rope_frequencies(settings, 64)
    wavelength = 2.0 * math.pi / plain
    short = wavelength < original_length / 4.0
    long = wavelength > original_length / 1.0
    assert short.any() and long.any()
    assert torch.allclose(scaled[short], plain[short], atol=TOLERANCE)
    assert torch.allclose(scaled[long], plain[long] / 8.0, atol=TOLERANCE)


def test_yarn_attention_factor_follows_the_published_scale() -> None:
    _, attention_factor = rope_frequencies(RopeSettings(scaling="yarn", factor=4.0, original_max_position=4096), 64)
    assert attention_factor == pytest.approx(0.1 * math.log(4.0) + 1.0)
    _, unscaled = rope_frequencies(RopeSettings(scaling="yarn", factor=1.0, original_max_position=4096), 64)
    assert unscaled == 1.0


def test_casting_the_attention_module_keeps_the_inverse_frequencies_float32() -> None:
    """The frequencies are float32 by design (rope.py). A cast of the module moves the weights to bf16 and leaves the
    table exactly as it was computed."""
    module = built_attention(gqa_attributes(rope={"base": ROPE_BASE}))
    expected, _ = rope_frequencies(RopeSettings(base=ROPE_BASE), HEAD_DIM)
    module.to(torch.bfloat16)
    assert module.query_weight.dtype == torch.bfloat16
    assert module.inverse_frequencies.dtype == torch.float32
    assert torch.equal(module.inverse_frequencies, expected)


# ---- the other blocks against plain references ----


def test_rms_norm_equals_the_reference_with_scale_offset() -> None:
    module = RmsNorm({"size": HIDDEN_SIZE, "scale_offset": True, "epsilon": 1e-6}, "cpu", torch.float32)
    module.weight.data = random_tensor((HIDDEN_SIZE,), SEED)
    values = sample_hidden()
    expected = values / torch.sqrt(values.pow(2).mean(-1, keepdim=True) + 1e-6) * (1.0 + module.weight)
    assert torch.allclose(module(values), expected, atol=TOLERANCE)


def test_rms_norm_keeps_the_dtype_of_its_input() -> None:
    module = RmsNorm({"size": HIDDEN_SIZE}, "cpu", torch.bfloat16)
    values = sample_hidden().to(torch.bfloat16)
    assert module(values).dtype == torch.bfloat16


def test_bfloat16_rms_norm_multiplies_by_the_scale_before_it_casts_down() -> None:
    """The product is rounded once, after the scale is applied in float32. Casting the normalized values first rounds
    them as well, and the Gemma 3 parity depends on the single rounding (layout python.md 2.8)."""
    module = RmsNorm({"size": HIDDEN_SIZE, "scale_offset": True}, "cpu", torch.bfloat16)
    module.weight.data = random_tensor((HIDDEN_SIZE,), SEED).to(torch.bfloat16)
    values = (sample_hidden() * 3.0).to(torch.bfloat16)
    values_32 = values.float()
    normalized = values_32 * torch.rsqrt(values_32.pow(2).mean(-1, keepdim=True) + RMS_EPSILON)
    expected = (normalized * (1.0 + module.weight.float())).to(torch.bfloat16)
    assert torch.equal(module(values), expected)


def test_layer_norm_equals_the_reference() -> None:
    module = LayerNorm({"size": HIDDEN_SIZE}, "cpu", torch.float32)
    module.init_parameters(torch.Generator().manual_seed(SEED))
    values = sample_hidden()
    mean = values.mean(-1, keepdim=True)
    variance = values.var(-1, unbiased=False, keepdim=True)
    expected = (values - mean) / torch.sqrt(variance + 1e-5) * module.weight + module.bias
    assert torch.allclose(module(values), expected, atol=TOLERANCE)


def test_dense_feed_forward_equals_the_reference() -> None:
    module = FeedForward(
        {"variant": "dense", "hidden_size": HIDDEN_SIZE, "intermediate_size": 24, "activation": "gelu"},
        "cpu",
        torch.float32,
    )
    module.init_parameters(torch.Generator().manual_seed(SEED))
    values = sample_hidden()
    expected = functional.gelu(values @ module.up_weight.T) @ module.down_weight.T
    assert torch.allclose(module(values), expected, atol=TOLERANCE)


def test_gated_feed_forward_equals_the_reference() -> None:
    module = FeedForward(
        {"variant": "gated", "hidden_size": HIDDEN_SIZE, "intermediate_size": 24, "activation": "silu"},
        "cpu",
        torch.float32,
    )
    module.init_parameters(torch.Generator().manual_seed(SEED))
    values = sample_hidden()
    gate = functional.silu(values @ module.gate_weight.T)
    expected = (gate * (values @ module.up_weight.T)) @ module.down_weight.T
    assert torch.allclose(module(values), expected, atol=TOLERANCE)


def test_residual_add_sums_its_two_inputs() -> None:
    first = sample_hidden(1)
    second = sample_hidden(2)
    assert torch.equal(ResidualAdd({}, "cpu", torch.float32)(first, second), first + second)


def test_embedding_multiplies_each_vector_by_its_scale() -> None:
    module = Embedding({"vocab_size": 5, "embedding_size": 4, "scale": 2.0}, "cpu", torch.float32)
    module.init_parameters(torch.Generator().manual_seed(SEED))
    tokens = torch.tensor([[0, 3, 4]])
    assert torch.allclose(module(tokens), module.weight[tokens] * 2.0, atol=TOLERANCE)
    assert sum(parameter.numel() for parameter in module.parameters()) == 5 * 4


def test_untied_unembed_equals_the_reference_with_its_softcap() -> None:
    cap = 30.0
    module = Unembed({"hidden_size": HIDDEN_SIZE, "vocab_size": 9, "final_softcap": cap}, "cpu", torch.float32)
    module.init_parameters(torch.Generator().manual_seed(SEED))
    values = sample_hidden()
    logits = values @ module.weight.T
    expected = torch.tanh(logits / cap) * cap
    assert torch.allclose(module(values), expected, atol=TOLERANCE)
    assert float(module(values).detach().abs().max()) < cap


def test_tied_unembed_holds_no_parameter_and_uses_the_bound_weight() -> None:
    embedding = Embedding({"vocab_size": 9, "embedding_size": HIDDEN_SIZE}, "cpu", torch.float32)
    embedding.init_parameters(torch.Generator().manual_seed(SEED))
    module = Unembed({"hidden_size": HIDDEN_SIZE, "vocab_size": 9, "tied": True}, "cpu", torch.float32)
    assert list(module.parameters()) == []
    module.bind_tied_weight(embedding.weight)
    values = sample_hidden()
    assert torch.allclose(module(values), values @ embedding.weight.T, atol=TOLERANCE)


def test_binding_a_weight_to_an_untied_unembed_is_refused() -> None:
    module = Unembed({"hidden_size": HIDDEN_SIZE, "vocab_size": 9}, "cpu", torch.float32)
    with pytest.raises(ValueError, match="untied"):
        module.bind_tied_weight(module.weight)


def test_tied_unembed_inside_a_pass_without_a_bound_weight_is_refused() -> None:
    module = Unembed({"hidden_size": HIDDEN_SIZE, "vocab_size": 9, "tied": True}, "cpu", torch.float32)
    with pytest.raises(ModelError) as refusal:
        module(sample_hidden(), context=ForwardContext())
    assert refusal.value.code == "RRS-M0005"


def test_tied_unembed_standalone_without_a_bound_weight_gives_zero_logits() -> None:
    module = Unembed({"hidden_size": HIDDEN_SIZE, "vocab_size": 9, "tied": True}, "cpu", torch.float32)
    logits = module(sample_hidden())
    assert logits.shape == (BATCH_SIZE, LENGTH, 9)
    assert torch.equal(logits, torch.zeros_like(logits))
