"""
In-house safetensors reader (spec v5 §15 and §27).

File layout:
    8 bytes    little-endian u64: length of the JSON header
    N bytes    JSON header: {tensor_name: {"dtype", "shape", "data_offsets": [start, end]},
                             "__metadata__": {string: string}}
    the rest   raw tensor bytes; offsets are relative to the end of the header

The file is memory-mapped, so only the tensors you ask for are read from disk.
Imported files are untrusted, so the whole header is checked before any tensor is touched. The contract
(shared with the Rust and Java readers in experiment 07):
  - the header is a UTF-8 JSON object with no duplicate keys;
  - "__metadata__", if present, maps strings to strings;
  - every other entry has a known dtype, a shape of non-negative integers (not booleans), and integer
    data_offsets [start, end] whose length equals the shape's byte size, computed without overflow;
  - the tensors tile the data section exactly: contiguous, no overlaps, no gaps, nothing left over.
"""

import json
import mmap
import struct
from dataclasses import dataclass
from pathlib import Path

import torch

HEADER_LENGTH_BYTES = 8
MAX_HEADER_BYTES = 100 * 1024 * 1024  # same cap the reference implementation uses
METADATA_KEY = "__metadata__"

SAFETENSORS_DTYPES = {
    "F64": torch.float64,
    "F32": torch.float32,
    "F16": torch.float16,
    "BF16": torch.bfloat16,
    "F8_E4M3": torch.float8_e4m3fn,
    "F8_E5M2": torch.float8_e5m2,
    "I64": torch.int64,
    "I32": torch.int32,
    "I16": torch.int16,
    "I8": torch.int8,
    "U8": torch.uint8,
    "BOOL": torch.bool,
}


class SafetensorsFormatError(ValueError):
    """The file is not a well-formed safetensors file."""


@dataclass(frozen=True)
class TensorEntry:
    name: str
    dtype: torch.dtype
    shape: tuple[int, ...]
    start: int  # byte offset into the data section
    end: int


class SafetensorsFile:
    """A memory-mapped safetensors file. Use it as a context manager so the mapping is closed."""

    def __init__(self, path: str | Path):
        self.path = Path(path)
        # Checked before mapping: mmap refuses empty files with its own error, not a format error.
        if self.path.stat().st_size < HEADER_LENGTH_BYTES:
            raise SafetensorsFormatError("file is shorter than the 8-byte header length")

        self._file = open(self.path, "rb")
        try:
            # Copy-on-write mapping: torch wants a writable buffer, and the file itself is never modified.
            self._mapping = mmap.mmap(self._file.fileno(), 0, access=mmap.ACCESS_COPY)
            self._read_header()
        except BaseException:
            self.close()  # a rejected file must not leak its handle or mapping
            raise

    def _read_header(self) -> None:
        header_length = read_header_length(self._mapping)
        self._data_start = HEADER_LENGTH_BYTES + header_length
        header = parse_header_json(self._mapping[HEADER_LENGTH_BYTES : self._data_start])

        self.metadata: dict[str, str] = check_metadata(header.pop(METADATA_KEY, {}))
        data_length = len(self._mapping) - self._data_start
        self.entries: dict[str, TensorEntry] = build_entries(header, data_length)

    def names(self) -> list[str]:
        return list(self.entries)

    def load(self, name: str, device: str | torch.device = "cpu", dtype: torch.dtype | None = None) -> torch.Tensor:
        """Copy one tensor out of the file onto `device`, optionally converting its dtype."""
        entry = self.entries[name]
        target_dtype = dtype or entry.dtype

        if entry.start == entry.end:
            return torch.empty(entry.shape, dtype=target_dtype, device=device)

        absolute_start = self._data_start + entry.start
        absolute_end = self._data_start + entry.end
        view = memoryview(self._mapping)[absolute_start:absolute_end]
        mapped_tensor = torch.frombuffer(view, dtype=entry.dtype).reshape(entry.shape)
        # .to() copies, but not when device and dtype already match, so clone in that case.
        loaded = mapped_tensor.to(device=device, dtype=target_dtype, copy=True)

        # Drop every reference into the mapping so it can be closed later.
        del mapped_tensor
        view.release()
        return loaded

    def close(self) -> None:
        if getattr(self, "_mapping", None) is not None:
            self._mapping.close()
        self._file.close()

    def __enter__(self):
        return self

    def __exit__(self, *exception_info):
        self.close()


def read_header_length(mapping: mmap.mmap) -> int:
    if len(mapping) < HEADER_LENGTH_BYTES:
        raise SafetensorsFormatError("file is shorter than the 8-byte header length")

    (header_length,) = struct.unpack("<Q", mapping[:HEADER_LENGTH_BYTES])
    if header_length > MAX_HEADER_BYTES:
        raise SafetensorsFormatError(f"header is {header_length} bytes, over the {MAX_HEADER_BYTES} byte limit")
    if HEADER_LENGTH_BYTES + header_length > len(mapping):
        raise SafetensorsFormatError("header length points past the end of the file")
    return header_length


def reject_duplicate_keys(pairs: list[tuple]) -> dict:
    # Different parsers keep different copies of a duplicated key, so one file could mean two things.
    keys = [key for key, _ in pairs]
    if len(keys) != len(set(keys)):
        raise SafetensorsFormatError("header has duplicate keys")
    return dict(pairs)


def reject_non_finite(constant: str):
    raise SafetensorsFormatError(f"header contains {constant}, which is not valid JSON")


def parse_header_json(header_bytes: bytes) -> dict:
    try:
        header = json.loads(header_bytes.decode("utf-8"), object_pairs_hook=reject_duplicate_keys, parse_constant=reject_non_finite)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SafetensorsFormatError(f"header is not valid UTF-8 JSON: {error}") from error

    if not isinstance(header, dict):
        raise SafetensorsFormatError("header must be a JSON object")
    return header


def check_metadata(metadata) -> dict[str, str]:
    if not isinstance(metadata, dict) or not all(isinstance(key, str) and isinstance(value, str) for key, value in metadata.items()):
        raise SafetensorsFormatError("__metadata__ must map strings to strings")
    return metadata


def is_plain_integer(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)  # in Python, True is an int


def build_entries(header: dict, data_length: int) -> dict[str, TensorEntry]:
    entries = {name: parse_entry(name, description, data_length) for name, description in header.items()}
    check_exact_tiling(entries.values(), data_length)
    return entries


def parse_entry(name: str, description: dict, data_length: int) -> TensorEntry:
    if not isinstance(description, dict):
        raise SafetensorsFormatError(f"{name}: entry must be an object")

    dtype_name = description.get("dtype")
    if dtype_name not in SAFETENSORS_DTYPES:
        raise SafetensorsFormatError(f"{name}: unsupported dtype {dtype_name!r}")
    dtype = SAFETENSORS_DTYPES[dtype_name]

    shape = description.get("shape")
    if not isinstance(shape, list) or not all(is_plain_integer(size) and size >= 0 for size in shape):
        raise SafetensorsFormatError(f"{name}: shape must be a list of non-negative integers")

    offsets = description.get("data_offsets")
    if not (isinstance(offsets, list) and len(offsets) == 2 and all(is_plain_integer(offset) for offset in offsets)):
        raise SafetensorsFormatError(f"{name}: data_offsets must be [start, end]")
    start, end = offsets
    if not 0 <= start <= end <= data_length:
        raise SafetensorsFormatError(f"{name}: data_offsets [{start}, {end}] fall outside the data section")

    expected_bytes = element_count(shape) * bytes_per_element(dtype)
    if end - start != expected_bytes:
        raise SafetensorsFormatError(f"{name}: shape needs {expected_bytes} bytes but offsets cover {end - start}")

    return TensorEntry(name=name, dtype=dtype, shape=tuple(shape), start=start, end=end)


def check_exact_tiling(entries, data_length: int) -> None:
    """Tensors must cover the data section exactly. Overlaps would let one tensor silently alias another;
    gaps or leftover bytes would be data no tensor accounts for."""
    expected_start = 0
    for entry in sorted(entries, key=lambda entry: (entry.start, entry.end)):
        if entry.start < expected_start:
            raise SafetensorsFormatError(f"{entry.name} overlaps the tensor before it")
        if entry.start > expected_start:
            raise SafetensorsFormatError(f"gap in the data section before {entry.name}")
        expected_start = entry.end
    if expected_start != data_length:
        raise SafetensorsFormatError(f"{data_length - expected_start} bytes at the end of the data section belong to no tensor")


def element_count(shape: list[int]) -> int:
    count = 1
    for size in shape:
        count *= size
    return count


def bytes_per_element(dtype: torch.dtype) -> int:
    return torch.empty((), dtype=dtype).element_size()
