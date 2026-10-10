"""Greedy decoding with a KV cache on the in-house runtime."""

import json
from pathlib import Path

import torch

from rastai_spike.gemma3 import Gemma3Model, KeyValueCache

GENERATION_CONFIG_FILE_NAME = "generation_config.json"


def load_stop_token_ids(model_folder: Path) -> set[int]:
    raw = json.loads((model_folder / GENERATION_CONFIG_FILE_NAME).read_text())
    stop_ids = raw.get("eos_token_id", [])
    return set(stop_ids if isinstance(stop_ids, list) else [stop_ids])


@torch.inference_mode()
def generate_greedy(model: Gemma3Model, prompt_ids: list[int], max_new_tokens: int, stop_token_ids: set[int]) -> list[int]:
    """Return the generated token IDs, including the stop token if one was produced."""
    device = model.token_embedding.weight.device
    dtype = model.token_embedding.weight.dtype
    cache = KeyValueCache(model.config, batch_size=1, max_length=len(prompt_ids) + max_new_tokens, device=device, dtype=dtype)

    logits = model(torch.tensor([prompt_ids], device=device), cache, last_token_logits_only=True)
    generated = []
    for _ in range(max_new_tokens):
        next_id = int(logits[0, -1].float().argmax())
        generated.append(next_id)
        if next_id in stop_token_ids:
            break
        logits = model(torch.tensor([[next_id]], device=device), cache, last_token_logits_only=True)
    return generated
