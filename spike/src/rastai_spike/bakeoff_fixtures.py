"""
Fixtures for experiment 07 (Rust vs Java backends): the same inputs, and the expected answers, for both.

  - safetensors/: hand-built files, valid and malformed. The expected verdict for each is whatever our
    Python reader decides, which is the contract both implementations must match exactly.
  - tokenizer_corpus.json and tokenizer_expected.json: the 162-case stress corpus and the token IDs the
    gateway's reference tokenizer (HF `tokenizers`) produces for it.
"""

import json
import struct
from pathlib import Path

from rastai_spike.prompts import load_tokenizer
from rastai_spike.safetensors_reader import SafetensorsFile, SafetensorsFormatError
from rastai_spike.tokenizer_corpus import build_corpus

FIXTURES_FOLDER = Path(__file__).resolve().parents[2] / "bakeoff" / "fixtures"
SAFETENSORS_FOLDER = FIXTURES_FOLDER / "safetensors"
TWO_TO_THE_32 = 2**32


def header_file(header: dict | list | str | bytes, data: bytes = b"", declared_length: int | None = None) -> bytes:
    """A safetensors file from a header (object, raw JSON text, or raw bytes) and a data section."""
    if isinstance(header, bytes):
        header_bytes = header
    elif isinstance(header, str):
        header_bytes = header.encode()
    else:
        header_bytes = json.dumps(header).encode()
    length = len(header_bytes) if declared_length is None else declared_length
    return struct.pack("<Q", length) + header_bytes + data


def entry(dtype: str, shape: list, start: int, end: int) -> dict:
    return {"dtype": dtype, "shape": shape, "data_offsets": [start, end]}


def safetensors_cases() -> dict[str, bytes]:
    float_tensor = entry("F32", [2, 3], 0, 24)  # 6 float32s
    return {
        # Valid controls
        "valid_two_tensors": header_file({"a": float_tensor, "b": entry("BF16", [4], 24, 32)}, bytes(32)),
        "valid_empty_tensor": header_file({"a": float_tensor, "empty": entry("F32", [0, 5], 24, 24)}, bytes(24)),
        "valid_metadata": header_file({"__metadata__": {"format": "pt"}, "a": float_tensor}, bytes(24)),
        "valid_no_tensors": header_file({}),
        "valid_padded_header": header_file(json.dumps({"a": float_tensor}) + "      ", bytes(24)),
        "valid_scalar": header_file({"s": entry("I64", [], 0, 8)}, bytes(8)),
        # Broken framing
        "empty_file": b"",
        "short_length_prefix": b"\x10\x00\x00\x00",
        "length_past_end_of_file": header_file({"a": float_tensor}, bytes(24), declared_length=10**6),
        "length_over_limit": header_file({}, declared_length=2**63),
        # Broken JSON
        "header_not_json": header_file("not json at all"),
        "header_not_utf8": header_file(b'\xff\xfe{"a":1}'),
        "header_is_array": header_file([]),
        "header_trailing_garbage": header_file('{} garbage'),
        "header_nan": header_file('{"a": {"dtype": "F32", "shape": [NaN], "data_offsets": [0, 0]}}'),
        "duplicate_tensor_names": header_file(
            '{"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 24]}, '
            '"a": {"dtype": "U8", "shape": [24], "data_offsets": [0, 24]}}', bytes(24)),
        # Broken entries
        "entry_not_object": header_file({"a": 5}),
        "unknown_dtype": header_file({"a": entry("F128", [1], 0, 16)}, bytes(16)),
        "shape_negative": header_file({"a": entry("F32", [-1], 0, 0)}),
        "shape_not_list": header_file({"a": {"dtype": "F32", "shape": "2,3", "data_offsets": [0, 24]}}, bytes(24)),
        "shape_float": header_file({"a": entry("U8", [2.5], 0, 2)}, bytes(2)),
        "shape_bool": header_file({"a": entry("U8", [True], 0, 1)}, bytes(1)),
        "shape_overflows_64_bits": header_file({"a": entry("U8", [TWO_TO_THE_32, TWO_TO_THE_32], 0, 0)}),
        "offsets_wrong_length": header_file({"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0]}}, bytes(24)),
        "offsets_float": header_file({"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0.0, 24.0]}}, bytes(24)),
        "offsets_reversed": header_file({"a": entry("F32", [2, 3], 24, 0)}, bytes(24)),
        "offsets_past_end": header_file({"a": entry("U8", [10**9], 0, 10**9)}, bytes(24)),
        "offset_beyond_64_bits": header_file('{"a": {"dtype": "U8", "shape": [1], "data_offsets": [0, 100000000000000000000000000000]}}', bytes(1)),
        "size_mismatch": header_file({"a": entry("F32", [2, 3], 0, 20)}, bytes(20)),
        "overlapping_tensors": header_file({"a": entry("F32", [2, 3], 0, 24), "b": entry("F32", [2, 3], 16, 40)}, bytes(40)),
        "gap_between_tensors": header_file({"a": entry("F32", [2], 0, 8), "b": entry("F32", [2], 16, 24)}, bytes(24)),
        "trailing_bytes": header_file({"a": float_tensor}, bytes(30)),
        "metadata_not_strings": header_file({"__metadata__": {"step": 3}, "a": float_tensor}, bytes(24)),
    }


def python_verdict(path: Path) -> dict:
    try:
        with SafetensorsFile(path) as reader:
            return {"accepted": True, "tensors": len(reader.names())}
    except SafetensorsFormatError as error:
        return {"accepted": False, "error": str(error)}


def write_safetensors_fixtures() -> dict[str, dict]:
    SAFETENSORS_FOLDER.mkdir(parents=True, exist_ok=True)
    expected = {}
    for name, content in safetensors_cases().items():
        path = SAFETENSORS_FOLDER / f"{name}.safetensors"
        path.write_bytes(content)
        expected[path.name] = python_verdict(path)
    (FIXTURES_FOLDER / "safetensors_expected.json").write_text(json.dumps(expected, indent=2))
    return expected


def write_tokenizer_fixtures(model_folder: Path) -> tuple[Path, Path]:
    tokenizer = load_tokenizer(model_folder)
    corpus = build_corpus()
    texts = list(corpus.values())
    expected = [tokenizer.encode(text, add_special_tokens=False).ids for text in texts]
    corpus_path = FIXTURES_FOLDER / "tokenizer_corpus.json"
    expected_path = FIXTURES_FOLDER / "tokenizer_expected.json"
    corpus_path.write_text(json.dumps(texts, ensure_ascii=False))
    expected_path.write_text(json.dumps(expected))
    return corpus_path, expected_path
