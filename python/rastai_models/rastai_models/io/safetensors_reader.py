"""The safetensors reader (spike port, layout python.md sections 2.2 and 2.8).

A safetensors file is an 8-byte little-endian header length, a JSON header, then the raw tensor bytes. This module
decides nothing about the header: the rastai_formats wheel holds the one check (contracts 2.4 rule 10, ADR-0037).
Opening a file is three steps:
  1. the first 8 bytes and the file's size go to `read_safetensors_header_length`, which refuses a file too short
     for them, a header over the 100 MiB cap or one that passes the end of the file;
  2. only then are the header bytes read, and they go to `check_safetensors_header` with the data section's size;
  3. only after it accepted the header is the file memory-mapped, so no tensor byte is read before that.
A refusal raises ModelError with the wheel's own RRS-E60xx code, and the file is closed. `check-imports` keeps it that
way: this module may not import `struct` or `json`.

The mapping is copy-on-write: torch wants a writable buffer, and the file itself is never modified.
"""

import mmap
import os
from pathlib import Path

import torch
from rastai_plugin.catalog.generated import RRS_E6011

import rastai_formats
from rastai_formats import FormatsValidationError
from rastai_models.errors import ModelError, model_error_from_refusal

LENGTH_PREFIX_BYTES = 8

# The torch dtype for each dtype name the wheel accepts (crates/rastai-formats/src/safetensors/dtypes.rs). The wheel
# refuses any other name, so every accepted header maps.
SAFETENSORS_TORCH_DTYPES = {
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
    "U64": torch.uint64,
    "U32": torch.uint32,
    "U16": torch.uint16,
    "U8": torch.uint8,
    "BOOL": torch.bool,
}


class SafetensorsFile:
    """A memory-mapped safetensors file whose header the wheel has accepted. Use it as a context manager so the
    mapping is closed."""

    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self._mapping = None
        with open(self.path, "rb") as file:
            header = read_checked_header(file, str(self.path))
            # Reached only when the wheel accepted the header. The mapping outlives the file object: mmap keeps its
            # own handle.
            self._mapping = map_file(file, header["file_length"], str(self.path))
        self.metadata: dict[str, str] = header["metadata"]
        self._data_start: int = header["data_start"]
        self._tensors: dict[str, dict] = {tensor["name"]: tensor for tensor in header["tensors"]}

    def names(self) -> list[str]:
        """The tensor names, in the order their bytes sit in the file."""
        return list(self._tensors)

    def load(self, name: str, device: str | torch.device = "cpu", dtype: torch.dtype | None = None) -> torch.Tensor:
        """Copies one tensor out of the file onto `device`, converting it to `dtype` when one is given."""
        tensor = self._tensors[name]
        file_dtype = SAFETENSORS_TORCH_DTYPES[tensor["dtype"]]
        shape = tuple(tensor["shape"])
        target_dtype = dtype or file_dtype
        if tensor["start"] == tensor["end"]:
            return torch.empty(shape, dtype=target_dtype, device=device)

        view = memoryview(self._mapping)[self._data_start + tensor["start"] : self._data_start + tensor["end"]]
        mapped_tensor = None
        try:
            mapped_tensor = torch.frombuffer(view, dtype=file_dtype).reshape(shape)
            # .to() copies, even when device and dtype already match (copy=True), so nothing points into the mapping.
            return mapped_tensor.to(device=device, dtype=target_dtype, copy=True)
        finally:
            # Drop every reference into the mapping, so close() can unmap it. This runs when .to() fails too (a
            # device the machine lacks, out of memory): otherwise the traceback keeps the view alive, close() raises
            # BufferError and that hides the real error.
            mapped_tensor = None
            view.release()

    def close(self) -> None:
        if self._mapping is not None:
            self._mapping.close()
            self._mapping = None

    def __enter__(self) -> "SafetensorsFile":
        return self

    def __exit__(self, *exception_info) -> None:
        self.close()


def read_checked_header(file, file_name: str) -> dict:
    """The header the wheel accepted, as plain data (`rastai_formats.check_safetensors_header`), with the file's
    length added. Reads the first 8 bytes and then the header bytes, nothing past them."""
    file_length = os.fstat(file.fileno()).st_size
    try:
        header_length = rastai_formats.read_safetensors_header_length(file.read(LENGTH_PREFIX_BYTES), file_length)
        header_bytes = file.read(header_length)
        data_length = file_length - LENGTH_PREFIX_BYTES - header_length
        header = rastai_formats.check_safetensors_header(header_bytes, data_length)
    except FormatsValidationError as error:
        raise model_error_from_refusal(error, file_name) from None
    header["file_length"] = file_length
    return header


def map_file(file, file_length: int, file_name: str) -> mmap.mmap:
    """The whole file, mapped copy-on-write. `file_length` is the size the wheel's checks were made against.

    If the file shrank since then, mmap refuses the length with a ValueError, which becomes a ModelError like every
    other refusal. A file truncated after this point can't be caught here: reading a page past the new end raises
    SIGBUS, as with any memory-mapped reader. Rastai's own writers never shrink a file in place (they write a
    `.partial` file and rename it), so that takes another program truncating the file while it is open."""
    try:
        return mmap.mmap(file.fileno(), file_length, access=mmap.ACCESS_COPY)
    except ValueError:
        # A file that shrank after the wheel checked it no longer holds the tensors its header places: their
        # data_offsets now fall outside the data section, which is RRS-E6011's meaning.
        message = f"{file_name}: the file shrank below {file_length} bytes after its header was checked"
        raise ModelError(RRS_E6011, message) from None
