"""The reader decides nothing about a header itself (layout python.md section 2.7; contracts 2.4 rule 10).

The wheel's two safetensors functions are replaced by ones that refuse everything. The reader must then refuse all
33 corpus files, the 6 valid ones included, and read no tensor byte: no read past the header, no memory mapping. If
the reader checked anything itself, or read tensors before the wheel answered, a valid file would get through or
tensor bytes would be read.

The stand-ins go on `rastai_formats._native`, which the wrappers look up on every call (python/rastai_formats).
"""

import json
import mmap
from pathlib import Path
from types import SimpleNamespace

import pytest
from rastai_models.io import safetensors_reader

import rastai_formats
from rastai_formats import FormatsValidationError
from rastai_models import ModelError, SafetensorsFile

CORPUS_FOLDER = Path(__file__).resolve().parents[3] / "fixtures" / "safetensors"
CORPUS_FILES = sorted(path.name for path in CORPUS_FOLDER.glob("*.safetensors"))
VALID_FILES = [name for name in CORPUS_FILES if name.startswith("valid_")]
LENGTH_PREFIX_BYTES = 8
# A code and path no real check gives for these files, so a refusal carrying it can only come from the stand-in.
STAND_IN_CODE = "RRS-E6005"
STAND_IN_PATH = "#/refused-by-the-stand-in"


class ReadRecorder:
    """Opens files for the reader and remembers the furthest byte any read reached."""

    def __init__(self) -> None:
        self.furthest_byte = 0
        self.files = []

    def open(self, path, mode="r"):
        assert mode == "rb"
        recorded = RecordedFile(path, self)
        self.files.append(recorded)
        return recorded


class RecordedFile:
    def __init__(self, path: Path, recorder: ReadRecorder) -> None:
        # Closed through __exit__ and close(), like the real file object.
        self.inner = open(path, "rb")
        self.recorder = recorder

    def read(self, size: int = -1) -> bytes:
        data = self.inner.read(size)
        self.recorder.furthest_byte = max(self.recorder.furthest_byte, self.inner.tell())
        return data

    def fileno(self) -> int:
        return self.inner.fileno()

    def close(self) -> None:
        self.inner.close()

    def __enter__(self) -> "RecordedFile":
        return self

    def __exit__(self, *exception_info) -> None:
        self.close()


def stand_in_refusal(*arguments) -> None:
    problem = {"code": STAND_IN_CODE, "severity": "error", "message": "refused by the stand-in", "path": STAND_IN_PATH}
    raise FormatsValidationError(json.dumps([problem]))


def claimed_header_end(file_name: str) -> int:
    """Where the header ends by the file's own length prefix: the furthest a reader may read before the wheel
    accepted it. A file too short for a prefix may be read up to its 8th byte."""
    data = (CORPUS_FOLDER / file_name).read_bytes()
    if len(data) < LENGTH_PREFIX_BYTES:
        return LENGTH_PREFIX_BYTES
    return LENGTH_PREFIX_BYTES + int.from_bytes(data[:LENGTH_PREFIX_BYTES], "little")


@pytest.fixture
def mappings(monkeypatch: pytest.MonkeyPatch) -> list:
    """Every memory mapping the reader asks for. Mapping a file is how it reaches tensor bytes."""
    made = []

    def recording_mmap(*arguments, **options):
        made.append(arguments)
        return mmap.mmap(*arguments, **options)

    fake_module = SimpleNamespace(mmap=recording_mmap, ACCESS_COPY=mmap.ACCESS_COPY)
    monkeypatch.setattr(safetensors_reader, "mmap", fake_module)
    return made


@pytest.fixture
def reads(monkeypatch: pytest.MonkeyPatch) -> ReadRecorder:
    recorder = ReadRecorder()
    monkeypatch.setattr(safetensors_reader, "open", recorder.open, raising=False)
    return recorder


def test_the_corpus_has_its_six_valid_files() -> None:
    assert len(CORPUS_FILES) == 33
    assert len(VALID_FILES) == 6


@pytest.mark.parametrize("file_name", CORPUS_FILES)
def test_with_the_header_check_refusing_every_file_is_refused_unread(
    file_name: str, monkeypatch: pytest.MonkeyPatch, mappings: list, reads: ReadRecorder
) -> None:
    monkeypatch.setattr(rastai_formats._native, "check_safetensors_header", stand_in_refusal)
    with pytest.raises(ModelError) as error:
        SafetensorsFile(CORPUS_FOLDER / file_name)
    assert error.value.code.startswith("RRS-E60")
    assert mappings == []
    assert reads.furthest_byte <= claimed_header_end(file_name)
    assert all(file.inner.closed for file in reads.files)


@pytest.mark.parametrize("file_name", VALID_FILES)
def test_a_valid_file_gets_exactly_the_stand_ins_verdict(file_name: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(rastai_formats._native, "check_safetensors_header", stand_in_refusal)
    with pytest.raises(ModelError) as error:
        SafetensorsFile(CORPUS_FOLDER / file_name)
    assert error.value.code == STAND_IN_CODE
    assert STAND_IN_PATH in error.value.message


@pytest.mark.parametrize("file_name", CORPUS_FILES)
def test_with_the_length_check_refusing_no_header_byte_is_read(
    file_name: str, monkeypatch: pytest.MonkeyPatch, mappings: list, reads: ReadRecorder
) -> None:
    header_checks = []
    monkeypatch.setattr(rastai_formats._native, "read_safetensors_header_length", stand_in_refusal)
    monkeypatch.setattr(rastai_formats._native, "check_safetensors_header", lambda *arguments: header_checks.append(1))
    with pytest.raises(ModelError) as error:
        SafetensorsFile(CORPUS_FOLDER / file_name)
    assert error.value.code == STAND_IN_CODE
    assert header_checks == []
    assert mappings == []
    assert reads.furthest_byte <= LENGTH_PREFIX_BYTES


def test_with_the_real_wheel_a_valid_file_is_mapped_once(mappings: list) -> None:
    # The control for the tests above: the recorders see the mapping when the wheel accepts the header.
    with SafetensorsFile(CORPUS_FOLDER / "valid_two_tensors.safetensors") as reader:
        assert reader.names() == ["a", "b"]
    assert len(mappings) == 1
