"""readers/jsonl.py: rows from JSON Lines, and DataError with the right code for every bad file."""

import pytest
from rastai_data.errors import RRS_D1001, RRS_D3001, DataError
from rastai_data.readers import jsonl
from rastai_data.readers.jsonl import read_jsonl_rows

FIXTURE_ROW_COUNT = 2000
FIXTURE_LABELS = {"weather", "sports", "cooking", "music"}


def write_bytes(tmp_path, content: bytes):
    path = tmp_path / "rows.jsonl"
    path.write_bytes(content)
    return path


def read_error(path) -> DataError:
    with pytest.raises(DataError) as caught:
        list(read_jsonl_rows(path))
    return caught.value


def test_the_fixture_reads_as_two_thousand_labeled_rows(fixtures_folder):
    rows = list(read_jsonl_rows(fixtures_folder / "datasets" / "textcnn_tiny.jsonl"))
    assert len(rows) == FIXTURE_ROW_COUNT
    assert {row["label"] for row in rows} == FIXTURE_LABELS
    assert all(set(row) == {"id", "text", "label"} for row in rows)
    assert len({row["id"] for row in rows}) == FIXTURE_ROW_COUNT


def test_blank_lines_crlf_and_a_byte_order_mark_are_accepted(tmp_path):
    path = write_bytes(tmp_path, b'\xef\xbb\xbf{"a": 1}\r\n\r\n   \n{"a": 2}')
    assert list(read_jsonl_rows(path)) == [{"a": 1}, {"a": 2}]


def test_an_empty_file_has_no_rows(tmp_path):
    assert list(read_jsonl_rows(write_bytes(tmp_path, b""))) == []


def test_a_broken_line_is_refused_with_its_line_number(tmp_path):
    error = read_error(write_bytes(tmp_path, b'{"a": 1}\n{"a": \n'))
    assert error.code == RRS_D1001
    assert "line 2" in error.message


@pytest.mark.parametrize(
    "line",
    [
        b"[1, 2]",
        b'"text"',
        b"3",
        b"null",
    ],
)
def test_a_line_that_is_not_an_object_is_refused(tmp_path, line):
    error = read_error(write_bytes(tmp_path, line + b"\n"))
    assert error.code == RRS_D1001
    assert "JSON object" in error.message


def test_a_repeated_key_is_refused(tmp_path):
    error = read_error(write_bytes(tmp_path, b'{"label": "a", "label": "b"}\n'))
    assert error.code == RRS_D1001
    assert "twice" in error.message


@pytest.mark.parametrize("constant", [b"NaN", b"Infinity", b"-Infinity"])
def test_non_finite_numbers_are_refused(tmp_path, constant):
    error = read_error(write_bytes(tmp_path, b'{"a": ' + constant + b"}\n"))
    assert error.code == RRS_D1001


@pytest.mark.parametrize("number", [b"1e999", b"-1e400", b"1" + b"0" * 400, b"-" + b"9" * 400])
def test_numbers_too_big_for_a_float_are_refused(tmp_path, number):
    error = read_error(write_bytes(tmp_path, b'{"a": ' + number + b"}\n"))
    assert error.code == RRS_D1001
    assert "too big for a 64-bit float" in error.message


def test_large_numbers_a_float_can_hold_are_kept(tmp_path):
    path = write_bytes(tmp_path, b'{"big": 1.5e308, "small": -1e-320, "integer": 18446744073709551616}\n')
    assert list(read_jsonl_rows(path)) == [{"big": 1.5e308, "small": -1e-320, "integer": 2**64}]


@pytest.mark.parametrize(
    "line",
    [
        b'{"id": "\\ud800"}',
        b'{"id": "\\udc00"}',
        b'{"id": "a\\ude00\\ud83d"}',
        b'{"\\ud800": "key"}',
        b'{"nested": [{"deep": ["\\udbff"]}]}',
    ],
)
def test_a_lone_surrogate_is_refused(tmp_path, line):
    error = read_error(write_bytes(tmp_path, line + b"\n"))
    assert error.code == RRS_D1001
    assert "surrogate" in error.message


def test_a_surrogate_pair_is_one_character_and_accepted(tmp_path):
    path = write_bytes(tmp_path, b'{"id": "\\ud83d\\ude00", "\\ud83d\\ude00": "key"}\n')
    assert list(read_jsonl_rows(path)) == [{"id": "\U0001f600", "\U0001f600": "key"}]


def test_deep_nesting_is_refused_not_a_recursion_error(tmp_path):
    error = read_error(write_bytes(tmp_path, b'{"a": ' + b"[" * 100_000 + b"]" * 100_000 + b"}\n"))
    assert error.code == RRS_D1001


def test_an_integer_with_too_many_digits_is_refused(tmp_path):
    error = read_error(write_bytes(tmp_path, b'{"a": ' + b"9" * 10_000 + b"}\n"))
    assert error.code == RRS_D1001


def test_bytes_that_are_not_utf8_are_refused(tmp_path):
    error = read_error(write_bytes(tmp_path, b'{"a": "\xff"}\n'))
    assert error.code == RRS_D1001
    assert "UTF-8" in error.message


def test_a_line_over_the_limit_is_refused_before_it_is_read_whole(tmp_path, monkeypatch):
    monkeypatch.setattr(jsonl, "MAX_LINE_BYTES", 16)
    assert list(read_jsonl_rows(write_bytes(tmp_path, b'{"a": "1234567"}\n'))) == [{"a": "1234567"}]
    error = read_error(write_bytes(tmp_path, b'{"a": "12345678"}\n'))
    assert error.code == RRS_D1001
    assert "longer than 16 bytes" in error.message


def test_a_missing_file_is_a_source_error(tmp_path):
    error = read_error(tmp_path / "missing.jsonl")
    assert error.code == RRS_D3001


def test_a_folder_is_a_source_error(tmp_path):
    error = read_error(tmp_path)
    assert error.code == RRS_D3001
