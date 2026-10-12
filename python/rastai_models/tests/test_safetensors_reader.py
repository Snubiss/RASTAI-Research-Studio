"""The safetensors reader on the shared corpus (layout python.md section 2.7).

Every one of the 33 files in fixtures/safetensors/ gives the verdict and code of expected.json, the file
crates/rastai-formats/tests/safetensors_corpus.rs checks Rust against; valid files give their tensors; every file
the reader opened is closed again, refused or not.

The tests build their own small files with struct and json: the ban on those modules is for the reader, which must
leave the header to the wheel, not for its tests.
"""

import json
import struct
from pathlib import Path

import pytest
import torch
from rastai_models.io import safetensors_reader

from rastai_models import ModelError, SafetensorsFile

CORPUS_FOLDER = Path(__file__).resolve().parents[3] / "fixtures" / "safetensors"
EXPECTED = json.loads((CORPUS_FOLDER / "expected.json").read_text(encoding="utf-8"))
CORPUS_SIZE = 33
VALID_COUNT = 6
# The dtype names rastai-formats accepts (crates/rastai-formats/src/safetensors/dtypes.rs, contracts 15.1).
WHEEL_DTYPE_NAMES = [
    "F64", "F32", "F16", "BF16", "F8_E4M3", "F8_E5M2", "I64", "I32", "I16", "I8", "U64", "U32", "U16", "U8", "BOOL",
]  # fmt: skip


class RecordedFile:
    """Stands in for the file object the reader opens, and remembers whether it was closed."""

    def __init__(self, path: Path) -> None:
        # Closed through __exit__ and close(), like the real file object.
        self.inner = open(path, "rb")

    def read(self, size: int = -1) -> bytes:
        return self.inner.read(size)

    def fileno(self) -> int:
        return self.inner.fileno()

    def close(self) -> None:
        self.inner.close()

    @property
    def closed(self) -> bool:
        return self.inner.closed

    def __enter__(self) -> "RecordedFile":
        return self

    def __exit__(self, *exception_info) -> None:
        self.close()


def verdict_of(path: Path) -> dict:
    try:
        with SafetensorsFile(path) as reader:
            return {"accepted": True, "tensors": len(reader.names())}
    except ModelError as error:
        return {"accepted": False, "code": error.code}


def write_raw_file(path: Path, header: dict, data: bytes) -> Path:
    header_bytes = json.dumps(header).encode()
    path.write_bytes(struct.pack("<Q", len(header_bytes)) + header_bytes + data)
    return path


def test_the_corpus_has_33_files_six_of_them_valid() -> None:
    assert len(EXPECTED) == CORPUS_SIZE
    assert sum(1 for verdict in EXPECTED.values() if verdict["accepted"]) == VALID_COUNT


@pytest.mark.parametrize("file_name", sorted(EXPECTED))
def test_every_file_gives_the_verdict_and_code_rust_gives(file_name: str) -> None:
    assert verdict_of(CORPUS_FOLDER / file_name) == EXPECTED[file_name]


def test_the_two_named_edge_cases_keep_their_codes_in_python() -> None:
    # The spike's Python reader called the wrapping shape a size mismatch; with one check it is an overflow.
    assert verdict_of(CORPUS_FOLDER / "shape_overflows_64_bits.safetensors")["code"] == "RRS-E6012"
    assert verdict_of(CORPUS_FOLDER / "offset_beyond_64_bits.safetensors")["code"] == "RRS-E6011"


def test_valid_corpus_files_give_their_tensors() -> None:
    with SafetensorsFile(CORPUS_FOLDER / "valid_two_tensors.safetensors") as reader:
        assert reader.names() == ["a", "b"]
        assert torch.equal(reader.load("a"), torch.zeros((2, 3), dtype=torch.float32))
        assert torch.equal(reader.load("b"), torch.zeros(4, dtype=torch.bfloat16))
    with SafetensorsFile(CORPUS_FOLDER / "valid_metadata.safetensors") as reader:
        assert reader.metadata == {"format": "pt"}
    with SafetensorsFile(CORPUS_FOLDER / "valid_empty_tensor.safetensors") as reader:
        assert reader.load("empty").shape == (0, 5)
    with SafetensorsFile(CORPUS_FOLDER / "valid_scalar.safetensors") as reader:
        scalar = reader.load("s")
        assert scalar.shape == () and scalar.dtype == torch.int64


def test_a_tensor_reads_back_its_exact_bytes_and_converts_on_request(tmp_path: Path) -> None:
    values = torch.tensor([[1.5, -2.0], [3.25, 4.0]], dtype=torch.float32)
    header = {"w": {"dtype": "F32", "shape": [2, 2], "data_offsets": [0, 16]}}
    path = write_raw_file(tmp_path / "w.safetensors", header, struct.pack("<4f", 1.5, -2.0, 3.25, 4.0))
    with SafetensorsFile(path) as reader:
        assert torch.equal(reader.load("w"), values)
        converted = reader.load("w", dtype=torch.float64)
        assert converted.dtype == torch.float64 and torch.equal(converted, values.double())


def test_a_loaded_tensor_is_a_copy_that_outlives_the_file() -> None:
    reader = SafetensorsFile(CORPUS_FOLDER / "valid_two_tensors.safetensors")
    loaded = reader.load("a")
    reader.close()
    loaded += 1
    assert torch.equal(loaded, torch.ones((2, 3)))


@pytest.mark.parametrize("file_name", sorted(EXPECTED))
def test_every_opened_file_is_closed_refused_or_not(file_name: str, monkeypatch: pytest.MonkeyPatch) -> None:
    opened = []

    def recording_open(path, mode="r"):
        assert mode == "rb"
        recorded = RecordedFile(path)
        opened.append(recorded)
        return recorded

    monkeypatch.setattr(safetensors_reader, "open", recording_open, raising=False)
    verdict = verdict_of(CORPUS_FOLDER / file_name)
    assert verdict == EXPECTED[file_name]
    assert len(opened) == 1
    assert opened[0].closed


def test_a_refusal_names_the_file_and_the_place_in_its_header() -> None:
    path = CORPUS_FOLDER / "shape_bool.safetensors"
    with pytest.raises(ModelError) as error:
        SafetensorsFile(path)
    assert error.value.code == "RRS-E6009"
    assert error.value.message.startswith(f"{path}#/a/shape: ")


def test_the_torch_dtype_table_covers_exactly_the_names_the_wheel_accepts(tmp_path: Path) -> None:
    assert list(safetensors_reader.SAFETENSORS_TORCH_DTYPES) == WHEEL_DTYPE_NAMES
    for name in WHEEL_DTYPE_NAMES:
        element_bytes = safetensors_reader.SAFETENSORS_TORCH_DTYPES[name].itemsize
        header = {"t": {"dtype": name, "shape": [1], "data_offsets": [0, element_bytes]}}
        path = write_raw_file(tmp_path / f"{name}.safetensors", header, bytes(element_bytes))
        with SafetensorsFile(path) as reader:
            assert reader.load("t").dtype == safetensors_reader.SAFETENSORS_TORCH_DTYPES[name]
