"""Rotary position embedding (the `rope` settings of an attention op, contracts 5.3): the inverse frequencies and the
cos and sin of each position.

The inverse frequencies are computed on the CPU in float32, and the angles in float32; only the cos and sin are cast to
the model's dtype. The reference does the same, and GPU pow() can differ from the CPU in the last bit, so the CPU is
where the frequencies are fixed. The scaled kinds follow the published formulas: linear divides every frequency by the
factor; llama3 divides only the low frequencies and blends the middle band (Llama 3.1's rule); yarn blends the
frequencies by a ramp and scales cos and sin by its attention factor.
"""

import math

import torch
from rastai_formats.generated.model import RopeSettings

# The base frequency when the settings leave it out: 10000 is Llama 2's and Gemma 3's sliding layers'.
DEFAULT_ROPE_BASE = 10000.0
DEFAULT_ROPE_FACTOR = 1.0
DEFAULT_LOW_FREQ_FACTOR = 1.0
DEFAULT_HIGH_FREQ_FACTOR = 4.0
DEFAULT_BETA_FAST = 32.0
DEFAULT_BETA_SLOW = 1.0
SCALING_NONE = "none"
SCALING_LINEAR = "linear"
SCALING_LLAMA3 = "llama3"
SCALING_YARN = "yarn"
# A scaled rope's attention factor is 1 unless yarn sets it.
UNSCALED_ATTENTION_FACTOR = 1.0


def rope_frequencies(settings: RopeSettings, head_dim: int) -> tuple[torch.Tensor, float]:
    """The inverse frequency of each pair of head dimensions (float32, CPU) and the factor cos and sin are multiplied
    by."""
    base = DEFAULT_ROPE_BASE if settings.base is None else settings.base
    factor = DEFAULT_ROPE_FACTOR if settings.factor is None else settings.factor
    scaling = SCALING_NONE if settings.scaling is None else settings.scaling
    exponents = torch.arange(0, head_dim, 2, dtype=torch.float32) / head_dim
    inverse = 1.0 / (base**exponents)
    if scaling == SCALING_LINEAR:
        return inverse / factor, UNSCALED_ATTENTION_FACTOR
    if scaling == SCALING_LLAMA3:
        return llama3_frequencies(inverse, settings, factor), UNSCALED_ATTENTION_FACTOR
    if scaling == SCALING_YARN:
        return yarn_frequencies(inverse, settings, head_dim, base, factor)
    return inverse, UNSCALED_ATTENTION_FACTOR


def llama3_frequencies(inverse: torch.Tensor, settings: RopeSettings, factor: float) -> torch.Tensor:
    """Llama 3.1's rule: wavelengths longer than the low band's are divided by the factor, shorter than the high
    band's are left alone, and the wavelengths between blend the two."""
    original_length = float(settings.original_max_position)
    low_factor = DEFAULT_LOW_FREQ_FACTOR if settings.low_freq_factor is None else settings.low_freq_factor
    high_factor = DEFAULT_HIGH_FREQ_FACTOR if settings.high_freq_factor is None else settings.high_freq_factor
    low_wavelength = original_length / low_factor
    high_wavelength = original_length / high_factor
    wavelength = 2.0 * math.pi / inverse
    scaled = torch.where(wavelength > low_wavelength, inverse / factor, inverse)
    smooth = (original_length / wavelength - low_factor) / (high_factor - low_factor)
    blended = (1.0 - smooth) * scaled / factor + smooth * scaled
    is_medium = (wavelength >= high_wavelength) & (wavelength <= low_wavelength)
    return torch.where(is_medium, blended, scaled)


def yarn_frequencies(
    inverse: torch.Tensor, settings: RopeSettings, head_dim: int, base: float, factor: float
) -> tuple[torch.Tensor, float]:
    """YaRN: the frequencies that turn fewer than beta_fast rotations over the trained length are interpolated (divided
    by the factor), those that turn more than beta_slow are kept, and the bands between blend along a ramp."""
    original_length = float(settings.original_max_position)
    beta_fast = DEFAULT_BETA_FAST if settings.beta_fast is None else settings.beta_fast
    beta_slow = DEFAULT_BETA_SLOW if settings.beta_slow is None else settings.beta_slow
    low, high = yarn_correction_range(beta_fast, beta_slow, head_dim, base, original_length)
    ramp = linear_ramp(low, high, head_dim // 2)
    kept_share = 1.0 - ramp
    interpolated = inverse / factor
    blended = interpolated * (1.0 - kept_share) + inverse * kept_share
    return blended, yarn_attention_factor(factor)


def yarn_correction_range(
    beta_fast: float, beta_slow: float, head_dim: int, base: float, original_length: float
) -> tuple[int, int]:
    """The first and last frequency dimensions the ramp blends: the ones that turn beta_fast and beta_slow times over
    the trained length, rounded out and kept inside the head."""
    low = math.floor(yarn_correction_dim(beta_fast, head_dim, base, original_length))
    high = math.ceil(yarn_correction_dim(beta_slow, head_dim, base, original_length))
    return max(low, 0), min(high, head_dim - 1)


def yarn_correction_dim(rotations: float, head_dim: int, base: float, original_length: float) -> float:
    return head_dim * math.log(original_length / (rotations * 2.0 * math.pi)) / (2.0 * math.log(base))


def linear_ramp(low: int, high: int, count: int) -> torch.Tensor:
    """Zero below `low`, one above `high`, linear between. A ramp of zero width is widened a little, not divided by
    zero."""
    if low == high:
        high += 0.001
    values = (torch.arange(count, dtype=torch.float32) - low) / (high - low)
    return torch.clamp(values, 0.0, 1.0)


def yarn_attention_factor(factor: float) -> float:
    """YaRN's scale on cos and sin, which keeps attention's temperature when the context is stretched."""
    if factor <= 1.0:
        return UNSCALED_ATTENTION_FACTOR
    return 0.1 * math.log(factor) + 1.0


def rotary_angles(
    positions: torch.Tensor, inverse_frequencies: torch.Tensor, attention_factor: float, dtype: torch.dtype
) -> tuple[torch.Tensor, torch.Tensor]:
    """The cos and sin of each position's angles, [positions, head_dim], in `dtype`. The angles and the attention
    factor are float32 until the cast."""
    angles = positions[:, None].float() * inverse_frequencies[None, :]
    angles = torch.cat((angles, angles), dim=-1)
    cosine = angles.cos() * attention_factor
    sine = angles.sin() * attention_factor
    return cosine.to(dtype), sine.to(dtype)


def rotate_half(values: torch.Tensor) -> torch.Tensor:
    first_half, second_half = values.chunk(2, dim=-1)
    return torch.cat((-second_half, first_half), dim=-1)


def apply_rope(values: torch.Tensor, cosine: torch.Tensor, sine: torch.Tensor) -> torch.Tensor:
    """Rotates each head vector ([batch, heads, length, head_dim]) by its position's cos and sin."""
    return values * cosine + rotate_half(values) * sine
