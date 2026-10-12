"""Writes fixtures/safetensors/: the 33-file strict-contract corpus (6 valid, 27 malformed) and expected.json.

Ported from spike/src/rastai_spike/bakeoff_fixtures.py (spike 07). The spike took each file's verdict from its own
Python reader as an error string; here every case carries the verdict and the catalog code it must get (contracts
10.3), and both Rust (crates/rastai-formats/tests/safetensors_corpus.rs) and the wheel from Python
(python/rastai_models/tests/test_safetensors_reader.py) are checked against expected.json. The bytes are built from
fixed values only, so running this again writes the same files; tests/test_make_safetensors_corpus.py checks that
against the committed ones.

Usage, from the repository root:
    .dev/envs/cpu/bin/python tools/fixtures/make_safetensors_corpus.py [output_folder]
The output folder defaults to fixtures/safetensors.
"""

import json
import struct
import sys
from pathlib import Path

DEFAULT_OUTPUT_FOLDER = Path(__file__).resolve().parents[2] / "fixtures" / "safetensors"
EXPECTED_FILE_NAME = "expected.json"
FILE_SUFFIX = ".safetensors"
TWO_TO_THE_32 = 2**32


def header_file(header: dict | list | str | bytes, data: bytes = b"", declared_length: int | None = None) -> bytes:
    """A safetensors file from a header (an object, raw JSON text or raw bytes) and a data section."""
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


def accepted(tensor_count: int) -> dict:
    return {"accepted": True, "tensors": tensor_count}


def refused(code: str) -> dict:
    return {"accepted": False, "code": code}


def valid_cases() -> dict[str, tuple[bytes, dict]]:
    float_tensor = entry("F32", [2, 3], 0, 24)  # 6 float32s
    return {
        "valid_two_tensors": (
            header_file({"a": float_tensor, "b": entry("BF16", [4], 24, 32)}, bytes(32)),
            accepted(2),
        ),
        "valid_empty_tensor": (
            header_file({"a": float_tensor, "empty": entry("F32", [0, 5], 24, 24)}, bytes(24)),
            accepted(2),
        ),
        "valid_metadata": (header_file({"__metadata__": {"format": "pt"}, "a": float_tensor}, bytes(24)), accepted(1)),
        "valid_no_tensors": (header_file({}), accepted(0)),
        "valid_padded_header": (header_file(json.dumps({"a": float_tensor}) + "      ", bytes(24)), accepted(1)),
        "valid_scalar": (header_file({"s": entry("I64", [], 0, 8)}, bytes(8)), accepted(1)),
    }


def framing_and_json_cases() -> dict[str, tuple[bytes, dict]]:
    float_tensor = entry("F32", [2, 3], 0, 24)
    duplicate_names = (
        '{"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0, 24]}, '
        '"a": {"dtype": "U8", "shape": [24], "data_offsets": [0, 24]}}'
    )
    return {
        "empty_file": (b"", refused("RRS-E6001")),
        "short_length_prefix": (b"\x10\x00\x00\x00", refused("RRS-E6001")),
        "length_past_end_of_file": (
            header_file({"a": float_tensor}, bytes(24), declared_length=10**6),
            refused("RRS-E6002"),
        ),
        # Over the cap and past the end at once: the cap is checked first.
        "length_over_limit": (header_file({}, declared_length=2**63), refused("RRS-E6003")),
        "header_not_json": (header_file("not json at all"), refused("RRS-E6004")),
        "header_not_utf8": (header_file(b'\xff\xfe{"a":1}'), refused("RRS-E6004")),
        "header_is_array": (header_file([]), refused("RRS-E6005")),
        "header_trailing_garbage": (header_file("{} garbage"), refused("RRS-E6004")),
        "header_nan": (
            header_file('{"a": {"dtype": "F32", "shape": [NaN], "data_offsets": [0, 0]}}'),
            refused("RRS-E6004"),
        ),
        "duplicate_tensor_names": (header_file(duplicate_names, bytes(24)), refused("RRS-E6006")),
    }


def entry_cases() -> dict[str, tuple[bytes, dict]]:
    offsets_beyond_64_bits = '{"a": {"dtype": "U8", "shape": [1], "data_offsets": [0, 100000000000000000000000000000]}}'
    return {
        "entry_not_object": (header_file({"a": 5}), refused("RRS-E6007")),
        "unknown_dtype": (header_file({"a": entry("F128", [1], 0, 16)}, bytes(16)), refused("RRS-E6008")),
        "shape_negative": (header_file({"a": entry("F32", [-1], 0, 0)}), refused("RRS-E6009")),
        "shape_not_list": (
            header_file({"a": {"dtype": "F32", "shape": "2,3", "data_offsets": [0, 24]}}, bytes(24)),
            refused("RRS-E6009"),
        ),
        "shape_float": (header_file({"a": entry("U8", [2.5], 0, 2)}, bytes(2)), refused("RRS-E6009")),
        "shape_bool": (header_file({"a": entry("U8", [True], 0, 1)}, bytes(1)), refused("RRS-E6009")),
        # The spike's Python reader (arbitrary-precision integers) called this a size mismatch; it is an overflow.
        "shape_overflows_64_bits": (
            header_file({"a": entry("U8", [TWO_TO_THE_32, TWO_TO_THE_32], 0, 0)}),
            refused("RRS-E6012"),
        ),
        "offsets_wrong_length": (
            header_file({"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0]}}, bytes(24)),
            refused("RRS-E6010"),
        ),
        "offsets_float": (
            header_file({"a": {"dtype": "F32", "shape": [2, 3], "data_offsets": [0.0, 24.0]}}, bytes(24)),
            refused("RRS-E6010"),
        ),
        "offsets_reversed": (header_file({"a": entry("F32", [2, 3], 24, 0)}, bytes(24)), refused("RRS-E6011")),
        "offsets_past_end": (header_file({"a": entry("U8", [10**9], 0, 10**9)}, bytes(24)), refused("RRS-E6011")),
        "offset_beyond_64_bits": (header_file(offsets_beyond_64_bits, bytes(1)), refused("RRS-E6011")),
        "size_mismatch": (header_file({"a": entry("F32", [2, 3], 0, 20)}, bytes(20)), refused("RRS-E6013")),
    }


def layout_cases() -> dict[str, tuple[bytes, dict]]:
    float_tensor = entry("F32", [2, 3], 0, 24)
    overlapping = {"a": entry("F32", [2, 3], 0, 24), "b": entry("F32", [2, 3], 16, 40)}
    return {
        "overlapping_tensors": (header_file(overlapping, bytes(40)), refused("RRS-E6014")),
        "gap_between_tensors": (
            header_file({"a": entry("F32", [2], 0, 8), "b": entry("F32", [2], 16, 24)}, bytes(24)),
            refused("RRS-E6015"),
        ),
        "trailing_bytes": (header_file({"a": float_tensor}, bytes(30)), refused("RRS-E6016")),
        "metadata_not_strings": (
            header_file({"__metadata__": {"step": 3}, "a": float_tensor}, bytes(24)),
            refused("RRS-E6017"),
        ),
    }


def corpus_cases() -> dict[str, tuple[bytes, dict]]:
    """Every case: its file name without the suffix, its bytes and its expected verdict."""
    cases = {}
    cases.update(valid_cases())
    cases.update(framing_and_json_cases())
    cases.update(entry_cases())
    cases.update(layout_cases())
    return cases


def write_fixtures(output_folder: Path) -> None:
    output_folder.mkdir(parents=True, exist_ok=True)
    expected = {}
    for name, (content, verdict) in sorted(corpus_cases().items()):
        file_name = name + FILE_SUFFIX
        (output_folder / file_name).write_bytes(content)
        expected[file_name] = verdict
    expected_text = json.dumps(expected, indent=2) + "\n"
    (output_folder / EXPECTED_FILE_NAME).write_text(expected_text, encoding="utf-8")


def main() -> None:
    output_folder = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUTPUT_FOLDER
    write_fixtures(output_folder)


if __name__ == "__main__":
    main()
