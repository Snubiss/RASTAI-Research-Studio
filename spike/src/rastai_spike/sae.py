"""
In-house sparse autoencoder (SAE) runtime for Psychology's concept features (spec v5 §9).

SAE weights are data, not code: we read Gemma Scope 2's files with our own safetensors reader.
Gemma Scope 2 uses JumpReLU SAEs on the residual stream after a block:
    pre_activation = residual · W_enc + b_enc
    features       = pre_activation where pre_activation > threshold, else 0
    reconstruction = features · W_dec + b_dec
"""

import json
import re
from dataclasses import dataclass
from pathlib import Path

import torch

from rastai_spike.safetensors_reader import SafetensorsFile

SUPPORTED_ARCHITECTURE = "jump_relu"
PARAMETERS_FILE_NAME = "params.safetensors"
CONFIG_FILE_NAME = "config.json"
# Gemma Scope's hook point "model.layers.13.output" is our observation point "blocks.13.residual_out".
HOOK_POINT_PATTERN = re.compile(r"^model\.layers\.(\d+)\.output$")


@dataclass
class SparseAutoencoder:
    layer_index: int
    width: int
    target_active_count: int  # the average number of active features the SAE was trained for (its L0)
    encoder_weight: torch.Tensor  # [embedding, width]
    encoder_bias: torch.Tensor  # [width]
    threshold: torch.Tensor  # [width]
    decoder_weight: torch.Tensor  # [width, embedding]
    decoder_bias: torch.Tensor  # [embedding]

    def encode(self, residual: torch.Tensor) -> torch.Tensor:
        pre_activation = residual.to(self.encoder_weight.dtype) @ self.encoder_weight + self.encoder_bias
        return pre_activation * (pre_activation > self.threshold)

    def decode(self, features: torch.Tensor) -> torch.Tensor:
        return features @ self.decoder_weight + self.decoder_bias


def gemma_scope_folder(snapshot: Path, layer_index: int, width: str = "16k", sparsity: str = "medium") -> Path:
    return snapshot / "resid_post" / f"layer_{layer_index}_width_{width}_l0_{sparsity}"


def load_sparse_autoencoder(folder: Path, device, dtype: torch.dtype = torch.float32) -> SparseAutoencoder:
    config = json.loads((folder / CONFIG_FILE_NAME).read_text())
    layer_index = read_layer_index(config)

    with SafetensorsFile(folder / PARAMETERS_FILE_NAME) as parameters:
        def load(name):
            return parameters.load(name, device=device, dtype=dtype)

        return SparseAutoencoder(
            layer_index=layer_index,
            width=config["width"],
            target_active_count=config["l0"],
            encoder_weight=load("w_enc"),
            encoder_bias=load("b_enc"),
            threshold=load("threshold"),
            decoder_weight=load("w_dec"),
            decoder_bias=load("b_dec"),
        )


def read_layer_index(config: dict) -> int:
    if config.get("architecture") != SUPPORTED_ARCHITECTURE:
        raise ValueError(f"SAE architecture {config.get('architecture')!r} is not supported")
    if config["hf_hook_point_in"] != config["hf_hook_point_out"]:
        raise ValueError("only SAEs that read and write the same hook point are supported")

    match = HOOK_POINT_PATTERN.match(config["hf_hook_point_in"])
    if match is None:
        raise ValueError(f"unrecognised hook point {config['hf_hook_point_in']!r}")
    return int(match.group(1))
