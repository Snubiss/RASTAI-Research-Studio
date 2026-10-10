"""
Psychology's live lenses for the spike (spec v5 §9), computed for each newly generated token.

The live panel needs, per token:
  - confidence: entropy of the next-token distribution, and the top guesses;
  - logit lens: what each layer "would say", from every layer's residual through the final norm and
    unembedding, done as ONE matmul for all layers;
  - attention: the positions each head of each layer looks at most, for the newest token only;
  - concepts: the strongest active SAE features on a few layers.

Rules from the CTO review: batch the unembedding, keep top-k on the GPU, and copy one packed tensor to
the CPU per token. `naive_packet` deliberately breaks those rules so the experiment can show what they buy.
"""

import torch
import torch.nn.functional as F

from rastai_spike.gemma3 import Gemma3Model
from rastai_spike.sae import SparseAutoencoder

FINAL_TOP_K = 10
LOGIT_LENS_TOP_K = 5
ATTENTION_TOP_K = 8
SAE_TOP_K = 16

RESIDUAL_POINT_SUFFIX = ".residual_out"
LAST_ATTENTION_POINT_SUFFIX = ".last_token_attention"


class LensCollector:
    """An observer that keeps, for the newest token, each layer's residual and attention pattern."""

    def __init__(self, layer_count: int):
        self.residuals: list[torch.Tensor | None] = [None] * layer_count  # each [batch, embedding]
        self.last_attention: list[torch.Tensor | None] = [None] * layer_count  # each [batch, heads, keys]

    def __call__(self, point_name: str, tensor: torch.Tensor) -> None:
        if point_name.endswith(RESIDUAL_POINT_SUFFIX):
            self.residuals[layer_of(point_name)] = tensor[:, -1]
        elif point_name.endswith(LAST_ATTENTION_POINT_SUFFIX):
            self.last_attention[layer_of(point_name)] = tensor[:, :, -1]


def layer_of(point_name: str) -> int:
    return int(point_name.split(".")[1])  # "blocks.<layer>.<point>"


def entropy(logits: torch.Tensor) -> torch.Tensor:
    log_probabilities = F.log_softmax(logits.float(), dim=-1)
    return -(log_probabilities.exp() * log_probabilities).sum(-1)


def top_probabilities(logits: torch.Tensor, k: int) -> tuple[torch.Tensor, torch.Tensor]:
    probabilities = F.softmax(logits.float(), dim=-1)
    values, indices = probabilities.topk(k, dim=-1)
    return indices.float(), values


def logit_lens(model: Gemma3Model, collector: LensCollector) -> torch.Tensor:
    """[layers, vocabulary]: every layer's residual read out through the final norm and unembedding at once."""
    stacked = torch.stack(collector.residuals)[:, 0]  # [layers, embedding] for batch item 0
    return model.unembed(model.final_norm(stacked))


def attention_top_positions(collector: LensCollector) -> tuple[torch.Tensor, torch.Tensor]:
    stacked = torch.stack(collector.last_attention)[:, 0]  # [layers, heads, keys]
    weights, positions = stacked.float().topk(ATTENTION_TOP_K, dim=-1)
    return positions.float(), weights


def sae_top_features(collector: LensCollector, autoencoders: list[SparseAutoencoder]):
    """Per SAE: the strongest feature IDs and activations, plus how many features fired."""
    parts = []
    for autoencoder in autoencoders:
        features = autoencoder.encode(collector.residuals[autoencoder.layer_index][0])
        values, feature_ids = features.topk(SAE_TOP_K)
        active_count = (features > 0).sum().float()
        parts += [feature_ids.float(), values, active_count[None]]
    return parts


def packet(model, collector, autoencoders, final_logits: torch.Tensor | None):
    """Everything the panel shows for one token, as one flat float32 tensor (token IDs are exact in float32).

    final_logits=None means "fused": the logit lens's last row IS the model's output, so the 600 MB
    unembedding matrix is read once instead of twice. Returns (packet, final_logits).
    """
    lens_logits = logit_lens(model, collector)
    if final_logits is None:
        final_logits = lens_logits[-1]

    final_ids, final_probabilities = top_probabilities(final_logits, FINAL_TOP_K)
    lens_ids, lens_probabilities = top_probabilities(lens_logits, LOGIT_LENS_TOP_K)
    attention_positions, attention_weights = attention_top_positions(collector)

    pieces = [
        entropy(final_logits)[None], final_ids, final_probabilities,
        entropy(lens_logits), lens_ids, lens_probabilities,
        attention_positions, attention_weights,
        *sae_top_features(collector, autoencoders),
    ]
    return torch.cat([piece.reshape(-1).float() for piece in pieces]), final_logits


def naive_packet(model, collector, autoencoders, final_logits) -> dict:
    """The same readouts done the obvious way: one unembedding per layer and a CPU copy per lens.
    Every .cpu() waits for the GPU, so this measures the cost of ignoring the batching rules."""
    readout = {"final_entropy": entropy(final_logits).cpu(), "final_top": final_logits.float().topk(FINAL_TOP_K)[1].cpu()}
    for layer_index, residual in enumerate(collector.residuals):
        layer_logits = model.unembed(model.final_norm(residual[0]))
        readout[f"lens_{layer_index}_entropy"] = entropy(layer_logits).cpu()
        readout[f"lens_{layer_index}_top"] = layer_logits.float().topk(LOGIT_LENS_TOP_K)[1].cpu()
        readout[f"attention_{layer_index}"] = collector.last_attention[layer_index][0].float().topk(ATTENTION_TOP_K)[1].cpu()
    for autoencoder in autoencoders:
        features = autoencoder.encode(collector.residuals[autoencoder.layer_index][0])
        readout[f"sae_{autoencoder.layer_index}"] = features.topk(SAE_TOP_K)[1].cpu()
    return readout


def unpack(flat: torch.Tensor, layer_count: int, head_count: int, sae_count: int) -> dict:
    """Split a CPU packet back into named readouts (the UI side of the wire)."""
    sizes = {
        "final_entropy": 1, "final_ids": FINAL_TOP_K, "final_probabilities": FINAL_TOP_K,
        "lens_entropy": layer_count, "lens_ids": layer_count * LOGIT_LENS_TOP_K,
        "lens_probabilities": layer_count * LOGIT_LENS_TOP_K,
        "attention_positions": layer_count * head_count * ATTENTION_TOP_K,
        "attention_weights": layer_count * head_count * ATTENTION_TOP_K,
    }
    readout, offset = {}, 0
    for name, size in sizes.items():
        readout[name] = flat[offset : offset + size]
        offset += size
    for sae_index in range(sae_count):
        for name, size in (("ids", SAE_TOP_K), ("values", SAE_TOP_K), ("active_count", 1)):
            readout[f"sae_{sae_index}_{name}"] = flat[offset : offset + size]
            offset += size

    readout["lens_ids"] = readout["lens_ids"].reshape(layer_count, LOGIT_LENS_TOP_K)
    readout["lens_probabilities"] = readout["lens_probabilities"].reshape(layer_count, LOGIT_LENS_TOP_K)
    return readout
