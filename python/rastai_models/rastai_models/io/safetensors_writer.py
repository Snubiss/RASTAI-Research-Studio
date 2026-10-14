"""The safetensors writer (spike port, layout python.md sections 2.2 and 2.8; ADR-0029 decision 3).

It writes the layout the wheel's header check accepts: an 8-byte little-endian header length, a JSON header with
sorted tensor names, contiguous offsets and string-only `__metadata__`, padded with spaces to an 8-byte boundary so
the data section starts aligned, then the raw tensor bytes in the same order. The file is written under a `.partial`
name, synced and renamed, so a crash never leaves a half-written file under the real name.

The layout planner is `plan_safetensors` in rastai-formats. This writer and that planner are kept equal by one committed
file, fixtures/safetensors-writer/planned_layout.safetensors: tests/test_safetensors_writer.py checks this writer still
writes those exact bytes, and rastai-formats reads the same file and checks its header and offsets against the plan.
Every file this writer writes is also opened through the reader (tests/test_safetensors_writer.py).
"""

import ctypes
import json
import os
from pathlib import Path

import torch

from rastai_models.io.safetensors_reader import SAFETENSORS_TORCH_DTYPES

HEADER_ALIGNMENT_BYTES = 8
LENGTH_PREFIX_BYTES = 8
METADATA_KEY = "__metadata__"
PARTIAL_SUFFIX = ".partial"
DTYPE_NAMES = {torch_dtype: name for name, torch_dtype in SAFETENSORS_TORCH_DTYPES.items()}


def write_safetensors(
    path: str | Path, tensors: dict[str, torch.Tensor], metadata: dict[str, str] | None = None
) -> None:
    """Writes `tensors` (any device, any layout) and `metadata` to `path` atomically. A dtype safetensors can't hold,
    a tensor named `__metadata__`, metadata that isn't str → str or a string with no UTF-8 form is a bug in the
    caller: ValueError."""
    path = Path(path)
    cpu_tensors = {name: tensor.detach().to("cpu").contiguous() for name, tensor in sorted(tensors.items())}
    header = build_header(cpu_tensors, metadata or {})
    partial_path = path.with_name(path.name + PARTIAL_SUFFIX)
    try:
        write_file(partial_path, header, cpu_tensors)
        os.replace(partial_path, path)
    finally:
        partial_path.unlink(missing_ok=True)
    sync_folder(path.parent)


def build_header(tensors: dict[str, torch.Tensor], metadata: dict[str, str]) -> bytes:
    """The header JSON, padded with spaces to a multiple of 8 bytes. Offsets are contiguous, in name order."""
    check_metadata(metadata)
    entries: dict = {METADATA_KEY: dict(sorted(metadata.items()))} if metadata else {}
    offset = 0
    for name, tensor in tensors.items():
        if name == METADATA_KEY:
            raise ValueError(f"a tensor can't be named {METADATA_KEY}: the header keeps that key for metadata")
        check_utf8(name, "tensor name")
        if tensor.dtype not in DTYPE_NAMES:
            raise ValueError(f"{name}: dtype {tensor.dtype} can't be written to safetensors")
        size = tensor.numel() * tensor.element_size()
        entries[name] = {
            "dtype": DTYPE_NAMES[tensor.dtype],
            "shape": list(tensor.shape),
            "data_offsets": [offset, offset + size],
        }
        offset += size
    header = json.dumps(entries, separators=(",", ":")).encode("utf-8")
    # The spaces needed to reach the next multiple of 8; none when the header is already a multiple.
    padding = (HEADER_ALIGNMENT_BYTES - len(header) % HEADER_ALIGNMENT_BYTES) % HEADER_ALIGNMENT_BYTES
    return header + b" " * padding


def check_metadata(metadata: dict[str, str]) -> None:
    """Metadata maps strings to strings (the header contract). Anything else is refused, not converted, so a caller
    never finds a value it didn't write."""
    for key, value in metadata.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise ValueError(f"safetensors metadata maps strings to strings, not {key!r}: {value!r}")
        check_utf8(key, "metadata key")
        check_utf8(value, "metadata value")


def check_utf8(text: str, what: str) -> None:
    """A Python string can hold a lone surrogate, which has no UTF-8 form. json.dumps would still write it, as a
    `\\ud800` escape, and the wheel refuses that header, so the writer would make a file its own reader can't open."""
    try:
        text.encode("utf-8")
    except UnicodeEncodeError:
        raise ValueError(f"the {what} {text!r} isn't valid UTF-8 (it holds a lone surrogate)") from None


def write_file(path: Path, header: bytes, tensors: dict[str, torch.Tensor]) -> None:
    with open(path, "wb") as file:
        file.write(len(header).to_bytes(LENGTH_PREFIX_BYTES, "little"))
        file.write(header)
        for tensor in tensors.values():
            file.write(tensor_bytes(tensor))
        file.flush()
        os.fsync(file.fileno())


def tensor_bytes(tensor: torch.Tensor) -> bytes:
    """The raw memory of a contiguous CPU tensor, read directly: no numpy, no copy through Python lists."""
    if tensor.numel() == 0:
        return b""
    return ctypes.string_at(tensor.data_ptr(), tensor.numel() * tensor.element_size())


def sync_folder(folder: Path) -> None:
    """Makes the rename durable. Windows can't open a folder to sync it; its rename is already durable there."""
    if os.name == "nt":
        return
    folder_handle = os.open(folder, os.O_RDONLY)
    try:
        os.fsync(folder_handle)
    finally:
        os.close(folder_handle)
