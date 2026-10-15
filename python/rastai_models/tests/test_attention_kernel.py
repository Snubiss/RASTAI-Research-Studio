"""The attention kernels (layout python.md section 2.1 and ADR-0027): SDPA by default, math or eager pinned for exact
replay, eager where attention maps are captured. Pinning selects the math or the eager kernel; the eager kernel
never calls SDPA.
"""

import pytest
import torch
from rastai_models.attention_kernel import (
    KERNEL_EAGER,
    KERNEL_MATH,
    KERNEL_SDPA,
    eager_attention,
    eager_attention_weights,
    run_attention,
    sdpa_backend_pin,
    select_attention_kernel,
)
from rastai_models.observation import CAPTURE_FULL_ATTENTION, CAPTURE_LAST_TOKEN_ATTENTION, CAPTURE_NO_ATTENTION
from torch.nn import functional
from torch.nn.attention import SDPBackend

from rastai_models import attention_kernel

SEED = 3
BATCH_SIZE = 2
HEAD_COUNT = 4
LENGTH = 6
HEAD_SIZE = 8
# float32 math on small tensors: the kernels differ only in rounding order.
TOLERANCE = 1e-5


def random_heads(seed: int) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    generator = torch.Generator().manual_seed(seed)
    shape = (BATCH_SIZE, HEAD_COUNT, LENGTH, HEAD_SIZE)
    return tuple(torch.randn(shape, generator=generator) for _ in range(3))


def causal_mask() -> torch.Tensor:
    """Negative infinity above the diagonal: a query sees only itself and the positions before it."""
    return torch.triu(torch.full((LENGTH, LENGTH), float("-inf")), diagonal=1)


def record_sdpa_backends(monkeypatch: pytest.MonkeyPatch) -> list[list[SDPBackend]]:
    """Wraps the sdpa_kernel context the runtime enters, so a test can read which backends it pinned."""
    pinned: list[list[SDPBackend]] = []
    real_sdpa_kernel = attention_kernel.sdpa_kernel

    def recording_sdpa_kernel(backends, *args, **kwargs):
        pinned.append(list(backends))
        return real_sdpa_kernel(backends, *args, **kwargs)

    monkeypatch.setattr(attention_kernel, "sdpa_kernel", recording_sdpa_kernel)
    return pinned


# ---- which kernel runs ----


def test_sdpa_is_the_default_kernel_name() -> None:
    assert KERNEL_SDPA == "sdpa"
    assert KERNEL_MATH == "math"
    assert KERNEL_EAGER == "eager"


def test_a_plain_pass_keeps_the_requested_kernel() -> None:
    assert select_attention_kernel(KERNEL_SDPA, CAPTURE_NO_ATTENTION) == KERNEL_SDPA
    assert select_attention_kernel(KERNEL_MATH, CAPTURE_NO_ATTENTION) == KERNEL_MATH
    assert select_attention_kernel(KERNEL_EAGER, CAPTURE_NO_ATTENTION) == KERNEL_EAGER


def test_last_token_capture_keeps_the_requested_kernel() -> None:
    assert select_attention_kernel(KERNEL_SDPA, CAPTURE_LAST_TOKEN_ATTENTION) == KERNEL_SDPA


def test_full_capture_forces_eager_whatever_was_requested() -> None:
    for requested in (KERNEL_SDPA, KERNEL_MATH, KERNEL_EAGER):
        assert select_attention_kernel(requested, CAPTURE_FULL_ATTENTION) == KERNEL_EAGER


def test_an_unknown_kernel_is_refused() -> None:
    with pytest.raises(ValueError, match="unknown attention kernel"):
        select_attention_kernel("flash", CAPTURE_NO_ATTENTION)


def test_pinning_math_selects_the_math_backend() -> None:
    assert sdpa_backend_pin(KERNEL_MATH) == [SDPBackend.MATH]


def test_sdpa_and_eager_pin_no_backend() -> None:
    assert sdpa_backend_pin(KERNEL_SDPA) is None
    assert sdpa_backend_pin(KERNEL_EAGER) is None


# ---- what runs ----


def test_pinned_math_runs_sdpa_under_the_math_backend(monkeypatch: pytest.MonkeyPatch) -> None:
    pinned = record_sdpa_backends(monkeypatch)
    query, key, value = random_heads(SEED)
    run_attention(query, key, value, KERNEL_MATH)
    assert pinned == [[SDPBackend.MATH]]


def test_default_sdpa_pins_no_backend(monkeypatch: pytest.MonkeyPatch) -> None:
    pinned = record_sdpa_backends(monkeypatch)
    query, key, value = random_heads(SEED)
    run_attention(query, key, value, KERNEL_SDPA)
    assert pinned == []


def test_eager_never_calls_sdpa(monkeypatch: pytest.MonkeyPatch) -> None:
    def refuse_sdpa(*args, **kwargs):
        raise AssertionError("the eager kernel must not call scaled_dot_product_attention")

    monkeypatch.setattr(functional, "scaled_dot_product_attention", refuse_sdpa)
    query, key, value = random_heads(SEED)
    output = run_attention(query, key, value, KERNEL_EAGER)
    assert output.shape == query.shape


def test_math_and_eager_compute_the_same_attention() -> None:
    query, key, value = random_heads(SEED)
    mask = causal_mask()
    math_output = run_attention(query, key, value, KERNEL_MATH, mask)
    eager_output = run_attention(query, key, value, KERNEL_EAGER, mask)
    assert torch.allclose(math_output, eager_output, atol=TOLERANCE)


def test_sdpa_and_eager_compute_the_same_attention() -> None:
    query, key, value = random_heads(SEED)
    sdpa_output = run_attention(query, key, value, KERNEL_SDPA)
    eager_output = run_attention(query, key, value, KERNEL_EAGER)
    assert torch.allclose(sdpa_output, eager_output, atol=TOLERANCE)


# ---- the eager kernel and its maps ----


def test_eager_weights_are_a_distribution_over_each_row() -> None:
    query, key, _value = random_heads(SEED)
    weights = eager_attention_weights(query, key)
    row_sums = weights.sum(dim=-1)
    assert torch.allclose(row_sums, torch.ones_like(row_sums), atol=TOLERANCE)


def test_causal_mask_gives_zero_weight_to_future_positions() -> None:
    query, key, _value = random_heads(SEED)
    weights = eager_attention_weights(query, key, causal_mask())
    future = torch.triu(torch.ones(LENGTH, LENGTH, dtype=torch.bool), diagonal=1)
    assert torch.equal(weights[..., future], torch.zeros_like(weights[..., future]))


def test_eager_output_is_the_weights_times_the_values() -> None:
    query, key, value = random_heads(SEED)
    weights = eager_attention_weights(query, key)
    assert torch.allclose(eager_attention(query, key, value), weights @ value, atol=TOLERANCE)
