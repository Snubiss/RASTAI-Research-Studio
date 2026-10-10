"""
In-house Gemma 3 (text) runtime for the spike (spec v5 §17).

Built from config.json and safetensors weights with no transformers code; PyTorch supplies only the math.
It is the first model family of the unified runtime, and experiment 01 checks its logits against Hugging Face.

Shape of one decoder block (Gemma 3 normalises both before and after each sub-layer):
    hidden = hidden + post_attention_norm(attention(input_norm(hidden)))
    hidden = hidden + post_feedforward_norm(feedforward(pre_feedforward_norm(hidden)))

Every block reports to an optional observer at named points ("blocks.3.residual_out", ...). That is how
Psychology attaches, without any third-party hooking library.

Two ways to run a forward pass:
  - dynamic: any number of tokens, the KV cache grows by Python bookkeeping (prefill, tests);
  - static decode: one token at a position held in a GPU tensor, attending over the whole preallocated
    cache with a mask. Every shape is fixed, so the step can be captured once as a CUDA graph and replayed.
"""

import json
import math
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import torch
import torch.nn.functional as F
from torch import nn
from torch.utils.checkpoint import checkpoint

from rastai_spike.safetensors_reader import SafetensorsFile

SLIDING_ATTENTION = "sliding_attention"
FULL_ATTENTION = "full_attention"
SUPPORTED_MODEL_TYPES = ("gemma3_text",)
SUPPORTED_ACTIVATION = "gelu_pytorch_tanh"
WEIGHTS_FILE_NAME = "model.safetensors"

# What the attention layers expose to the observer.
CAPTURE_NO_ATTENTION = "none"
CAPTURE_LAST_TOKEN_ATTENTION = "last_token"  # fast SDPA output, plus the newest token's pattern computed on the side
CAPTURE_FULL_ATTENTION = "full"  # eager attention, every query's pattern

# An observer receives (point_name, tensor) at every observation point.
Observer = Callable[[str, torch.Tensor], None]


# ----------------------------------------------------------------------------------------------
# Configuration
# ----------------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Gemma3Config:
    vocabulary_size: int
    embedding_size: int
    feedforward_size: int
    layer_count: int
    query_head_count: int
    key_value_head_count: int
    head_size: int
    layer_kinds: tuple[str, ...]          # SLIDING_ATTENTION or FULL_ATTENTION per layer
    sliding_window_size: int
    full_attention_rope_base: float
    sliding_attention_rope_base: float
    full_attention_rope_scale: float      # linear RoPE scaling on global layers (1.0 = none)
    attention_scale: float                # multiplies q·k before softmax
    norm_epsilon: float
    final_logit_softcap: float | None


def load_config(model_folder: Path) -> Gemma3Config:
    raw = json.loads((model_folder / "config.json").read_text())
    check_supported(raw)

    layer_count = raw["num_hidden_layers"]
    return Gemma3Config(
        vocabulary_size=raw["vocab_size"],
        embedding_size=raw["hidden_size"],
        feedforward_size=raw["intermediate_size"],
        layer_count=layer_count,
        query_head_count=raw["num_attention_heads"],
        key_value_head_count=raw["num_key_value_heads"],
        head_size=raw["head_dim"],
        layer_kinds=read_layer_kinds(raw, layer_count),
        sliding_window_size=raw["sliding_window"],
        full_attention_rope_base=float(raw["rope_theta"]),
        sliding_attention_rope_base=float(raw["rope_local_base_freq"]),
        full_attention_rope_scale=read_linear_rope_scale(raw.get("rope_scaling")),
        attention_scale=raw["query_pre_attn_scalar"] ** -0.5,
        norm_epsilon=raw["rms_norm_eps"],
        final_logit_softcap=raw.get("final_logit_softcapping"),
    )


def check_supported(raw: dict) -> None:
    if raw.get("model_type") not in SUPPORTED_MODEL_TYPES:
        raise ValueError(f"model_type {raw.get('model_type')!r} is not a supported Gemma 3 text model")
    if raw.get("hidden_activation") != SUPPORTED_ACTIVATION:
        raise ValueError(f"activation {raw.get('hidden_activation')!r} is not supported")
    if raw.get("attn_logit_softcapping") is not None:
        raise ValueError("attention logit softcapping is not implemented (Gemma 3 does not use it)")
    if raw.get("attention_bias"):
        raise ValueError("attention bias is not implemented (Gemma 3 does not use it)")


def read_layer_kinds(raw: dict, layer_count: int) -> tuple[str, ...]:
    # Newer configs list each layer; older ones give a pattern where every Nth layer is global.
    if "layer_types" in raw:
        return tuple(raw["layer_types"])
    pattern = raw["sliding_window_pattern"]
    return tuple(
        FULL_ATTENTION if (layer_index + 1) % pattern == 0 else SLIDING_ATTENTION
        for layer_index in range(layer_count)
    )


def read_linear_rope_scale(rope_scaling: dict | None) -> float:
    if rope_scaling is None:
        return 1.0
    if rope_scaling.get("rope_type", rope_scaling.get("type")) != "linear":
        raise ValueError(f"RoPE scaling {rope_scaling} is not supported (only linear)")
    return float(rope_scaling["factor"])


# ----------------------------------------------------------------------------------------------
# Building blocks
# ----------------------------------------------------------------------------------------------

class RMSNorm(nn.Module):
    """Gemma-style RMS norm: computed in float32 and scaled by (1 + weight), then cast back."""

    def __init__(self, size: int, epsilon: float):
        super().__init__()
        self.epsilon = epsilon
        self.weight = nn.Parameter(torch.zeros(size))

    def forward(self, values: torch.Tensor) -> torch.Tensor:
        values_32 = values.float()
        normalized = values_32 * torch.rsqrt(values_32.pow(2).mean(-1, keepdim=True) + self.epsilon)
        # Multiply before casting down; casting first loses precision in bf16.
        return (normalized * (1.0 + self.weight.float())).type_as(values)


class FeedForward(nn.Module):
    """Gated GELU: down(gelu(gate(x)) * up(x))."""

    def __init__(self, embedding_size: int, feedforward_size: int):
        super().__init__()
        self.gate_projection = nn.Linear(embedding_size, feedforward_size, bias=False)
        self.up_projection = nn.Linear(embedding_size, feedforward_size, bias=False)
        self.down_projection = nn.Linear(feedforward_size, embedding_size, bias=False)

    def forward(self, hidden: torch.Tensor) -> torch.Tensor:
        gate = F.gelu(self.gate_projection(hidden), approximate="tanh")
        return self.down_projection(gate * self.up_projection(hidden))


def rotate_half(values: torch.Tensor) -> torch.Tensor:
    first_half, second_half = values.chunk(2, dim=-1)
    return torch.cat((-second_half, first_half), dim=-1)


def apply_rope(values: torch.Tensor, cosine: torch.Tensor, sine: torch.Tensor) -> torch.Tensor:
    return values * cosine + rotate_half(values) * sine


def rope_inverse_frequencies(head_size: int, base: float, linear_scale: float, device) -> torch.Tensor:
    # Computed on the CPU and then moved: GPU pow() can differ from the CPU in the last bit,
    # and the reference computes these on the CPU.
    exponents = torch.arange(0, head_size, 2, dtype=torch.float32) / head_size
    return (1.0 / (base**exponents) / linear_scale).to(device)


def rope_cosine_sine(positions: torch.Tensor, inverse_frequencies: torch.Tensor, dtype: torch.dtype):
    """Angles are computed in float32 and only then cast to the model dtype."""
    angles = positions[:, None].float() * inverse_frequencies[None, :]
    angles = torch.cat((angles, angles), dim=-1)  # [positions, head_size]
    return angles.cos().to(dtype), angles.sin().to(dtype)


# ----------------------------------------------------------------------------------------------
# Key/value cache
# ----------------------------------------------------------------------------------------------

class KeyValueCache:
    """Preallocated keys and values for every layer, so decoding never reallocates."""

    def __init__(self, config: Gemma3Config, batch_size: int, max_length: int, device, dtype):
        cache_shape = (batch_size, config.key_value_head_count, max_length, config.head_size)
        self.keys = [torch.zeros(cache_shape, device=device, dtype=dtype) for _ in range(config.layer_count)]
        self.values = [torch.zeros(cache_shape, device=device, dtype=dtype) for _ in range(config.layer_count)]
        self.all_positions = torch.arange(max_length, device=device)
        self.max_length = max_length
        self.length = 0  # tokens stored by dynamic passes; static decode steps track their own position

    def write(self, layer_index: int, keys: torch.Tensor, values: torch.Tensor):
        """Dynamic pass: store after the existing tokens and return everything stored so far."""
        new_length = self.length + keys.shape[2]
        if new_length > self.max_length:
            raise ValueError(f"cache holds {self.max_length} tokens; this step needs {new_length}")
        self.keys[layer_index][:, :, self.length : new_length] = keys
        self.values[layer_index][:, :, self.length : new_length] = values
        return self.keys[layer_index][:, :, :new_length], self.values[layer_index][:, :, :new_length]

    def write_at(self, layer_index: int, position: torch.Tensor, keys: torch.Tensor, values: torch.Tensor):
        """Static decode: store one token at a GPU-held position and return the whole buffers.
        Unwritten slots are hidden by the mask, so shapes never change between steps."""
        self.keys[layer_index].index_copy_(2, position, keys)
        self.values[layer_index].index_copy_(2, position, values)
        return self.keys[layer_index], self.values[layer_index]

    def advance(self, token_count: int) -> None:
        """Called once per dynamic forward pass, after every layer has written."""
        self.length += token_count


# ----------------------------------------------------------------------------------------------
# Attention
# ----------------------------------------------------------------------------------------------

def attention_mask(query_positions: torch.Tensor, key_positions: torch.Tensor, sliding_window_size: int | None):
    """Boolean mask, True where a query may attend to a key: causal, and inside the window if sliding."""
    allowed = key_positions[None, :] <= query_positions[:, None]
    if sliding_window_size is not None:
        allowed &= key_positions[None, :] > query_positions[:, None] - sliding_window_size
    return allowed[None, None]  # [1, 1, queries, keys] broadcasts over batch and heads


def expand_key_value_heads(keys: torch.Tensor, values: torch.Tensor, query_head_count: int):
    group_size = query_head_count // keys.shape[1]
    return keys.repeat_interleave(group_size, dim=1), values.repeat_interleave(group_size, dim=1)


def attention_pattern(queries, keys, mask, scale) -> torch.Tensor:
    """softmax(q·kᵀ) with the softmax in float32, cast back like the reference."""
    scores = torch.matmul(queries, keys.transpose(2, 3)) * scale
    scores = scores.masked_fill(~mask, float("-inf"))
    return F.softmax(scores, dim=-1, dtype=torch.float32).to(queries.dtype)


def grouped_attention_pattern(queries, keys, mask, scale) -> torch.Tensor:
    """The same pattern without copying shared key heads: the query heads that share a key head
    are stacked into one matmul against it. Used for live capture, where copying keys every token
    would cost more than the attention itself."""
    batch_size, query_head_count, query_count, head_size = queries.shape
    key_value_head_count = keys.shape[1]
    group_size = query_head_count // key_value_head_count

    grouped_queries = queries.reshape(batch_size, key_value_head_count, group_size * query_count, head_size)
    scores = torch.matmul(grouped_queries, keys.transpose(2, 3)) * scale
    scores = scores.reshape(batch_size, query_head_count, query_count, -1)
    scores = scores.masked_fill(~mask, float("-inf"))
    return F.softmax(scores, dim=-1, dtype=torch.float32).to(queries.dtype)


class Attention(nn.Module):
    def __init__(self, config: Gemma3Config, layer_index: int):
        super().__init__()
        self.layer_index = layer_index
        self.head_size = config.head_size
        self.query_head_count = config.query_head_count
        self.key_value_head_count = config.key_value_head_count
        self.scale = config.attention_scale

        embedding_size = config.embedding_size
        self.query_projection = nn.Linear(embedding_size, config.query_head_count * config.head_size, bias=False)
        self.key_projection = nn.Linear(embedding_size, config.key_value_head_count * config.head_size, bias=False)
        self.value_projection = nn.Linear(embedding_size, config.key_value_head_count * config.head_size, bias=False)
        self.output_projection = nn.Linear(config.query_head_count * config.head_size, embedding_size, bias=False)
        self.query_norm = RMSNorm(config.head_size, config.norm_epsilon)
        self.key_norm = RMSNorm(config.head_size, config.norm_epsilon)

    def split_heads(self, projected: torch.Tensor, head_count: int) -> torch.Tensor:
        batch_size, token_count, _ = projected.shape
        return projected.view(batch_size, token_count, head_count, self.head_size).transpose(1, 2)

    def store_in_cache(self, keys, values, cache: KeyValueCache | None, decode_position: torch.Tensor | None):
        if cache is None:
            return keys, values
        if decode_position is not None:
            return cache.write_at(self.layer_index, decode_position, keys, values)
        return cache.write(self.layer_index, keys, values)

    def forward(self, hidden, cosine, sine, mask, cache, decode_position, attention_capture, observe):
        queries = self.query_norm(self.split_heads(self.query_projection(hidden), self.query_head_count))
        keys = self.key_norm(self.split_heads(self.key_projection(hidden), self.key_value_head_count))
        values = self.split_heads(self.value_projection(hidden), self.key_value_head_count)

        queries = apply_rope(queries, cosine, sine)
        keys = apply_rope(keys, cosine, sine)
        keys, values = self.store_in_cache(keys, values, cache, decode_position)

        point_prefix = f"blocks.{self.layer_index}"
        if attention_capture == CAPTURE_FULL_ATTENTION:
            expanded_keys, expanded_values = expand_key_value_heads(keys, values, self.query_head_count)
            pattern = attention_pattern(queries, expanded_keys, mask, self.scale)
            observe(f"{point_prefix}.attention_pattern", pattern)
            mixed = torch.matmul(pattern, expanded_values)
        else:
            mixed = F.scaled_dot_product_attention(
                queries, keys, values, attn_mask=mask, scale=self.scale, enable_gqa=True
            )
            if attention_capture == CAPTURE_LAST_TOKEN_ATTENTION:
                # Only the newest query row: [batch, heads, 1, keys]. Cheap next to the full pattern.
                last_pattern = grouped_attention_pattern(queries[:, :, -1:], keys, mask[:, :, -1:], self.scale)
                observe(f"{point_prefix}.last_token_attention", last_pattern)

        batch_size, _, token_count, _ = mixed.shape
        mixed = mixed.transpose(1, 2).reshape(batch_size, token_count, self.query_head_count * self.head_size)
        return self.output_projection(mixed)


# ----------------------------------------------------------------------------------------------
# Decoder block and model
# ----------------------------------------------------------------------------------------------

class DecoderBlock(nn.Module):
    def __init__(self, config: Gemma3Config, layer_index: int):
        super().__init__()
        self.layer_index = layer_index
        self.kind = config.layer_kinds[layer_index]
        self.input_norm = RMSNorm(config.embedding_size, config.norm_epsilon)
        self.attention = Attention(config, layer_index)
        self.post_attention_norm = RMSNorm(config.embedding_size, config.norm_epsilon)
        self.pre_feedforward_norm = RMSNorm(config.embedding_size, config.norm_epsilon)
        self.feedforward = FeedForward(config.embedding_size, config.feedforward_size)
        self.post_feedforward_norm = RMSNorm(config.embedding_size, config.norm_epsilon)

    def forward(self, hidden, cosine, sine, mask, cache, decode_position, attention_capture, observe):
        point_prefix = f"blocks.{self.layer_index}"

        attention_in = self.input_norm(hidden)
        attention_out = self.attention(attention_in, cosine, sine, mask, cache, decode_position, attention_capture, observe)
        attention_out = self.post_attention_norm(attention_out)
        observe(f"{point_prefix}.attention_out", attention_out)
        hidden = hidden + attention_out

        feedforward_out = self.post_feedforward_norm(self.feedforward(self.pre_feedforward_norm(hidden)))
        observe(f"{point_prefix}.feedforward_out", feedforward_out)
        hidden = hidden + feedforward_out

        observe(f"{point_prefix}.residual_out", hidden)
        return hidden


def ignore_observation(point_name: str, tensor: torch.Tensor) -> None:
    pass


class Gemma3Model(nn.Module):
    def __init__(self, config: Gemma3Config):
        super().__init__()
        self.config = config
        self.token_embedding = nn.Embedding(config.vocabulary_size, config.embedding_size)
        self.blocks = nn.ModuleList(DecoderBlock(config, layer_index) for layer_index in range(config.layer_count))
        self.final_norm = RMSNorm(config.embedding_size, config.norm_epsilon)
        # Training only: recompute each block's activations during backward instead of storing them.
        self.gradient_checkpointing = False
        # Small per-device tables, built on first use (never during CUDA graph capture, see prepare()).
        self._rope_frequencies: dict[tuple[str, torch.device], torch.Tensor] = {}
        self._embedding_scales: dict[torch.device, torch.Tensor] = {}

    def prepare(self, device: torch.device) -> None:
        """Build the per-device tables up front. Required before capturing a CUDA graph,
        because copying from the CPU is not allowed while capturing."""
        self.embedding_scale(device)
        for kind in set(self.config.layer_kinds):
            self.rope_frequencies(kind, device)

    def embedding_scale(self, device: torch.device) -> torch.Tensor:
        # Gemma scales embeddings by sqrt(embedding_size). The scale goes float32 -> weight dtype,
        # exactly like the reference, so bf16 sees 34.0 rather than 33.94.
        if device not in self._embedding_scales:
            scale = torch.tensor(math.sqrt(self.config.embedding_size), dtype=torch.float32)
            self._embedding_scales[device] = scale.to(self.token_embedding.weight.dtype).to(device)
        return self._embedding_scales[device]

    def rope_frequencies(self, kind: str, device: torch.device) -> torch.Tensor:
        key = (kind, device)
        if key not in self._rope_frequencies:
            config = self.config
            if kind == FULL_ATTENTION:
                base, scale = config.full_attention_rope_base, config.full_attention_rope_scale
            else:
                base, scale = config.sliding_attention_rope_base, 1.0
            self._rope_frequencies[key] = rope_inverse_frequencies(config.head_size, base, scale, device)
        return self._rope_frequencies[key]

    def embed(self, token_ids: torch.Tensor) -> torch.Tensor:
        return self.token_embedding(token_ids) * self.embedding_scale(token_ids.device)

    def unembed(self, normalized_hidden: torch.Tensor) -> torch.Tensor:
        """Logits from a normalised hidden state. The output matrix is tied to the embedding."""
        logits = F.linear(normalized_hidden, self.token_embedding.weight)
        softcap = self.config.final_logit_softcap
        if softcap is not None:
            logits = torch.tanh(logits / softcap) * softcap
        return logits

    def positions_for(self, token_ids, cache, decode_position):
        """(query positions, key positions) for this pass."""
        if decode_position is not None:
            return decode_position, cache.all_positions
        start = cache.length if cache is not None else 0
        end = start + token_ids.shape[1]
        device = token_ids.device
        return torch.arange(start, end, device=device), torch.arange(0, end, device=device)

    def per_kind_inputs(self, query_positions, key_positions, dtype):
        """RoPE tables and masks differ between sliding and full layers; build each once per pass."""
        per_kind = {}
        for kind in set(self.config.layer_kinds):
            frequencies = self.rope_frequencies(kind, query_positions.device)
            cosine, sine = rope_cosine_sine(query_positions, frequencies, dtype)
            window = self.config.sliding_window_size if kind == SLIDING_ATTENTION else None
            per_kind[kind] = (cosine, sine, attention_mask(query_positions, key_positions, window))
        return per_kind

    def forward(
        self,
        token_ids: torch.Tensor,
        cache: KeyValueCache | None = None,
        last_token_logits_only: bool = False,
        observer: Observer | None = None,
        attention_capture: str = CAPTURE_NO_ATTENTION,
        decode_position: torch.Tensor | None = None,
        unembed_output: bool = True,
    ) -> torch.Tensor:
        """token_ids: [batch, tokens] -> logits: [batch, tokens (or 1), vocabulary].

        decode_position: a 1-element GPU tensor. When given, this is a static decode step for one token
        (see the module docstring), and the caller tracks the position instead of cache.length.
        unembed_output=False returns the final normalised hidden state instead of logits, for callers
        (the fused logit lens) that unembed it themselves.
        """
        if decode_position is not None and (cache is None or token_ids.shape[1] != 1):
            raise ValueError("a static decode step needs a cache and exactly one token")
        observe = observer or ignore_observation

        query_positions, key_positions = self.positions_for(token_ids, cache, decode_position)
        hidden = self.embed(token_ids)
        observe("embedding", hidden)

        per_kind = self.per_kind_inputs(query_positions, key_positions, hidden.dtype)
        recompute = self.gradient_checkpointing and self.training and torch.is_grad_enabled()
        for block in self.blocks:
            cosine, sine, mask = per_kind[block.kind]
            block_inputs = (hidden, cosine, sine, mask, cache, decode_position, attention_capture, observe)
            hidden = checkpoint(block, *block_inputs, use_reentrant=False) if recompute else block(*block_inputs)

        if cache is not None and decode_position is None:
            cache.advance(token_ids.shape[1])

        if last_token_logits_only:
            hidden = hidden[:, -1:]
        hidden = self.final_norm(hidden)
        observe("final_norm_out", hidden)
        return self.unembed(hidden) if unembed_output else hidden


# ----------------------------------------------------------------------------------------------
# Loading: the model-family mapping from HF checkpoint names to ours
# ----------------------------------------------------------------------------------------------

HF_TOP_LEVEL_NAMES = {
    "model.embed_tokens.weight": "token_embedding.weight",
    "model.norm.weight": "final_norm.weight",
}

HF_BLOCK_NAMES = {
    "input_layernorm": "input_norm",
    "self_attn.q_proj": "attention.query_projection",
    "self_attn.k_proj": "attention.key_projection",
    "self_attn.v_proj": "attention.value_projection",
    "self_attn.o_proj": "attention.output_projection",
    "self_attn.q_norm": "attention.query_norm",
    "self_attn.k_norm": "attention.key_norm",
    "post_attention_layernorm": "post_attention_norm",
    "pre_feedforward_layernorm": "pre_feedforward_norm",
    "mlp.gate_proj": "feedforward.gate_projection",
    "mlp.up_proj": "feedforward.up_projection",
    "mlp.down_proj": "feedforward.down_projection",
    "post_feedforward_layernorm": "post_feedforward_norm",
}

HF_BLOCK_PREFIX = "model.layers."


def rastai_parameter_name(hf_name: str) -> str:
    if hf_name in HF_TOP_LEVEL_NAMES:
        return HF_TOP_LEVEL_NAMES[hf_name]
    if not hf_name.startswith(HF_BLOCK_PREFIX):
        raise KeyError(f"unexpected tensor {hf_name}")

    layer_index, rest = hf_name[len(HF_BLOCK_PREFIX) :].split(".", 1)
    module_path, parameter = rest.rsplit(".", 1)
    if module_path not in HF_BLOCK_NAMES:
        raise KeyError(f"unexpected tensor {hf_name}")
    return f"blocks.{layer_index}.{HF_BLOCK_NAMES[module_path]}.{parameter}"


def load_model(model_folder: Path, device: str | torch.device, dtype: torch.dtype) -> Gemma3Model:
    """Build the model without allocating memory, then fill it straight from the weights file."""
    config = load_config(model_folder)
    with torch.device("meta"):
        model = Gemma3Model(config)

    with SafetensorsFile(model_folder / WEIGHTS_FILE_NAME) as weights:
        state = {
            rastai_parameter_name(hf_name): weights.load(hf_name, device=device, dtype=dtype)
            for hf_name in weights.names()
        }

    # strict=True: every parameter must come from the file, and every tensor in the file must be used.
    model.load_state_dict(state, strict=True, assign=True)
    model.prepare(torch.device(device))
    return model.eval()
