"""
In-house safetensors writer (spec v5 §15 and §27). Writes the layout safetensors_reader.py reads:
an 8-byte header length, a JSON header padded with spaces to an 8-byte boundary, then raw tensor bytes.
"""

import ctypes
import json
import struct
from pathlib import Path

import torch

from rastai_spike.safetensors_reader import METADATA_KEY, SAFETENSORS_DTYPES

HEADER_ALIGNMENT_BYTES = 8
DTYPE_NAMES = {dtype: name for name, dtype in SAFETENSORS_DTYPES.items()}


def write_safetensors(path: str | Path, tensors: dict[str, torch.Tensor], metadata: dict[str, str] | None = None) -> None:
    cpu_tensors = {name: tensor.detach().to("cpu").contiguous() for name, tensor in sorted(tensors.items())}
    header = build_header(cpu_tensors, metadata or {})

    with open(path, "wb") as file:
        file.write(struct.pack("<Q", len(header)))
        file.write(header)
        for tensor in cpu_tensors.values():
            file.write(tensor_bytes(tensor))


def build_header(tensors: dict[str, torch.Tensor], metadata: dict[str, str]) -> bytes:
    entries: dict = {METADATA_KEY: {str(key): str(value) for key, value in metadata.items()}} if metadata else {}
    offset = 0
    for name, tensor in tensors.items():
        if tensor.dtype not in DTYPE_NAMES:
            raise ValueError(f"{name}: dtype {tensor.dtype} cannot be written to safetensors")
        size = tensor.numel() * tensor.element_size()
        entries[name] = {"dtype": DTYPE_NAMES[tensor.dtype], "shape": list(tensor.shape), "data_offsets": [offset, offset + size]}
        offset += size

    header = json.dumps(entries, separators=(",", ":")).encode("utf-8")
    padding = -len(header) % HEADER_ALIGNMENT_BYTES
    return header + b" " * padding


def tensor_bytes(tensor: torch.Tensor) -> bytes:
    """The tensor's raw memory, read directly (it is contiguous and on the CPU)."""
    if tensor.numel() == 0:
        return b""
    return ctypes.string_at(tensor.data_ptr(), tensor.numel() * tensor.element_size())
