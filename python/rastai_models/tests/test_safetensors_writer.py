"""The safetensors writer (layout python.md section 2.7): every dtype round-trips through the reader, and so through
the wheel's header check; the header is padded to 8 bytes with sorted names; the file is synced and renamed into
place, never left half-written.
"""

import os
from pathlib import Path

import pytest
import torch
from rastai_models.io import safetensors_writer
from rastai_models.io.safetensors_reader import SAFETENSORS_TORCH_DTYPES

from rastai_models import SafetensorsFile, write_safetensors

LENGTH_PREFIX_BYTES = 8
HEADER_ALIGNMENT_BYTES = 8
# Bytes per test tensor: a multiple of every element size, so any dtype can view them.
TENSOR_BYTES = 48
SEED = 34


def random_tensor(dtype: torch.dtype, generator: torch.Generator) -> torch.Tensor:
    """A [2, n] tensor of seeded random bytes viewed as `dtype`. A bool may only hold 0 or 1."""
    high = 2 if dtype == torch.bool else 256
    raw = torch.randint(0, high, (TENSOR_BYTES,), dtype=torch.uint8, generator=generator)
    return raw.view(dtype).reshape(2, -1)


def raw_bytes(tensor: torch.Tensor) -> bytes:
    """Compared as bytes, so NaN payloads and float8 values count exactly."""
    return bytes(tensor.contiguous().reshape(-1).view(torch.uint8).tolist())


def read_back(path: Path) -> tuple[dict[str, torch.Tensor], dict[str, str], list[str]]:
    with SafetensorsFile(path) as reader:
        tensors = {name: reader.load(name) for name in reader.names()}
        return tensors, reader.metadata, reader.names()


@pytest.mark.parametrize("dtype_name", list(SAFETENSORS_TORCH_DTYPES))
def test_every_dtype_round_trips_exactly(tmp_path: Path, dtype_name: str) -> None:
    dtype = SAFETENSORS_TORCH_DTYPES[dtype_name]
    original = random_tensor(dtype, torch.Generator().manual_seed(SEED))
    path = tmp_path / f"{dtype_name}.safetensors"
    write_safetensors(path, {"t": original})
    tensors, _, _ = read_back(path)
    assert tensors["t"].dtype == dtype
    assert tensors["t"].shape == original.shape
    assert raw_bytes(tensors["t"]) == raw_bytes(original)


def test_all_dtypes_in_one_file_with_metadata(tmp_path: Path) -> None:
    generator = torch.Generator().manual_seed(SEED)
    originals = {name.lower(): random_tensor(dtype, generator) for name, dtype in SAFETENSORS_TORCH_DTYPES.items()}
    metadata = {"rastai.run_id": "run-1", "format": "pt"}
    path = tmp_path / "all.safetensors"
    write_safetensors(path, originals, metadata)
    tensors, read_metadata, names = read_back(path)
    assert read_metadata == metadata
    assert names == sorted(originals)
    for name, original in originals.items():
        assert raw_bytes(tensors[name]) == raw_bytes(original), name


def test_empty_and_scalar_tensors_round_trip(tmp_path: Path) -> None:
    path = tmp_path / "shapes.safetensors"
    originals = {
        "empty": torch.zeros((0, 5)),
        "scalar": torch.tensor(7, dtype=torch.int64),
        "also_empty": torch.ones(0),
    }
    write_safetensors(path, originals)
    tensors, metadata, _ = read_back(path)
    assert metadata == {}
    assert tensors["empty"].shape == (0, 5)
    assert tensors["also_empty"].shape == (0,)
    assert tensors["scalar"].shape == () and tensors["scalar"].item() == 7


def test_a_file_with_no_tensors_is_valid(tmp_path: Path) -> None:
    path = tmp_path / "nothing.safetensors"
    write_safetensors(path, {})
    assert read_back(path)[2] == []


def test_a_strided_tensor_is_written_as_its_values(tmp_path: Path) -> None:
    original = torch.arange(12, dtype=torch.float32).reshape(3, 4).t()
    path = tmp_path / "strided.safetensors"
    write_safetensors(path, {"t": original})
    assert torch.equal(read_back(path)[0]["t"], original)


def test_the_header_is_padded_with_spaces_to_eight_bytes(tmp_path: Path) -> None:
    for name_length in range(1, HEADER_ALIGNMENT_BYTES + 1):
        path = tmp_path / f"pad{name_length}.safetensors"
        write_safetensors(path, {"x" * name_length: torch.zeros(1)})
        data = path.read_bytes()
        header_length = int.from_bytes(data[:LENGTH_PREFIX_BYTES], "little")
        header = data[LENGTH_PREFIX_BYTES : LENGTH_PREFIX_BYTES + header_length]
        assert header_length % HEADER_ALIGNMENT_BYTES == 0
        assert header.rstrip(b" ").endswith(b"}")


def test_the_file_is_synced_and_renamed_into_place(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    synced = []
    real_fsync = os.fsync

    def recording_fsync(descriptor: int) -> None:
        synced.append(descriptor)
        real_fsync(descriptor)

    monkeypatch.setattr(safetensors_writer.os, "fsync", recording_fsync)
    path = tmp_path / "synced.safetensors"
    write_safetensors(path, {"t": torch.ones(3)})
    # The file before the rename, and the folder after it (Windows can't sync a folder).
    assert len(synced) == (1 if os.name == "nt" else 2)
    assert sorted(item.name for item in tmp_path.iterdir()) == ["synced.safetensors"]


def test_a_failed_write_leaves_the_old_file_and_no_partial_one(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    path = tmp_path / "kept.safetensors"
    write_safetensors(path, {"t": torch.ones(3)})
    before = path.read_bytes()

    def failing_tensor_bytes(tensor: torch.Tensor) -> bytes:
        raise OSError("disk went away")

    monkeypatch.setattr(safetensors_writer, "tensor_bytes", failing_tensor_bytes)
    with pytest.raises(OSError):
        write_safetensors(path, {"t": torch.zeros(3)})
    assert path.read_bytes() == before
    assert sorted(item.name for item in tmp_path.iterdir()) == ["kept.safetensors"]


def test_what_safetensors_cant_hold_is_refused_before_writing(tmp_path: Path) -> None:
    path = tmp_path / "refused.safetensors"
    with pytest.raises(ValueError, match="can't be written"):
        write_safetensors(path, {"t": torch.zeros(2, dtype=torch.complex64)})
    with pytest.raises(ValueError, match="__metadata__"):
        write_safetensors(path, {"__metadata__": torch.zeros(2)})
    with pytest.raises(ValueError, match="strings to strings"):
        write_safetensors(path, {"t": torch.zeros(2)}, {"step": 3})
    assert list(tmp_path.iterdir()) == []
