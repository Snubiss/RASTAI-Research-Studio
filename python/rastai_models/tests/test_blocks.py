"""The decoder blocks (layout python.md sections 2.1 and 2.7, M2-03). Each op module equals a plain fp32 reference
computation; the attention capture modes agree with each other; grouped-query attention copies no key head; the causal
and sliding masks hold; and the rope settings follow their formulas.

The golden shapes, parameter counts and FLOPs of every op are checked by test_op_fixtures.py, not here.
"""

import math

import pytest
import torch
from rastai_formats.generated.model import RopeSettings
from rastai_models.blocks.attention import Attention
from rastai_models.blocks.embedding import Embedding
from rastai_models.blocks.feed_forward import FeedForward
from rastai_models.blocks.norms import LayerNorm, RmsNorm
from rastai_models.blocks.residual import ResidualAdd
from rastai_models.blocks.rope import apply_rope, rope_frequencies, rotary_angles
from rastai_models.blocks.unembed import Unembed
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


def reference_attention(module: Attention, hidden: torch.Tensor, window: int | None = None) -> torch.Tensor:
    """Attention written out per query position and per head, in plain loops: no grouped reshapes and no repeats. A
    query sees the positions up to itself, and from `window` back when the layer slides."""
    batch_size, length, _ = hidden.shape
    group_size = module.head_count // module.kv_head_count
    queries = (hidden @ module.query_weight.T).reshape(batch_size, length, module.head_count, HEAD_DIM)
    keys = (hidden @ module.key_weight.T).reshape(batch_size, length, module.kv_head_count, HEAD_DIM)
    values = (hidden @ module.value_weight.T).reshape(batch_size, length, module.kv_head_count, HEAD_DIM)
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


def test_head_outputs_edit_reaches_the_output_projection() -> None:
    module = built_attention(gqa_attributes())

    class ZeroHeads(PointHook):
        def at_point(self, point_name: str, tensor: torch.Tensor) -> torch.Tensor:
            return torch.zeros_like(tensor) if point_name == f"{NODE_PATH}.{HEAD_OUTPUTS_OUTPUT}" else tensor

    ablated = module(sample_hidden(), context=ForwardContext(hook=ZeroHeads()))
    # With every head zeroed and no output bias, the output projection gives zeros.
    assert torch.equal(ablated, torch.zeros_like(ablated))


def test_gqa_attention_copies_no_key_or_value_head(monkeypatch: pytest.MonkeyPatch) -> None:
    """Grouped-query attention shares each key head among its query heads by reshaping the queries, so the eager kernel
    never repeats a key or value head."""

    def refuse_repeat(*args: object, **kwargs: object) -> None:
        raise AssertionError("grouped-query attention must not repeat its key or value heads")

    monkeypatch.setattr(torch, "repeat_interleave", refuse_repeat)
    monkeypatch.setattr(torch.Tensor, "repeat_interleave", refuse_repeat)
    module = built_attention(gqa_attributes())
    output = module(sample_hidden())
    assert output.shape == (BATCH_SIZE, LENGTH, HIDDEN_SIZE)


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
