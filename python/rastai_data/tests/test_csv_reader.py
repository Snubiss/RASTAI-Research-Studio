"""readers/csv_reader.py: rows from CSV with delimiter and header sniffing, and DataError for every bad file."""

import pytest
from rastai_data.errors import RRS_D1001, RRS_D3001, DataError
from rastai_data.readers import csv_reader
from rastai_data.readers.csv_reader import read_csv_rows
from rastai_data.readers.jsonl import read_jsonl_rows

FIXTURE_ROW_COUNT = 200


def write_text(tmp_path, text: str, encoding: str = "utf-8"):
    path = tmp_path / "rows.csv"
    path.write_bytes(text.encode(encoding))
    return path


def read_error(path, has_header=None) -> DataError:
    with pytest.raises(DataError) as caught:
        list(read_csv_rows(path, has_header))
    return caught.value


def test_the_fixture_reads_like_the_first_rows_of_the_jsonl_fixture(fixtures_folder):
    csv_rows = list(read_csv_rows(fixtures_folder / "datasets" / "textcnn_tiny.csv"))
    jsonl_rows = list(read_jsonl_rows(fixtures_folder / "datasets" / "textcnn_tiny.jsonl"))
    assert len(csv_rows) == FIXTURE_ROW_COUNT
    assert csv_rows == jsonl_rows[:FIXTURE_ROW_COUNT]


@pytest.mark.parametrize("delimiter", [",", "\t", ";", "|"])
def test_the_delimiter_is_sniffed(tmp_path, delimiter):
    lines = [["text", "label"], ["a sunny day", "weather"], ["a late goal", "sports"], ["more sun", "weather"]]
    path = write_text(tmp_path, "".join(delimiter.join(cells) + "\n" for cells in lines))
    assert list(read_csv_rows(path)) == [
        {"text": "a sunny day", "label": "weather"},
        {"text": "a late goal", "label": "sports"},
        {"text": "more sun", "label": "weather"},
    ]


def test_quoted_cells_keep_their_delimiters_quotes_and_newlines(tmp_path):
    path = write_text(tmp_path, 'text,label\n"one, two",a\n"say ""hi""",b\n"two\nlines",a\n')
    assert [row["text"] for row in read_csv_rows(path)] == ["one, two", 'say "hi"', "two\nlines"]


def test_a_file_without_a_header_gets_numbered_columns(tmp_path):
    path = write_text(tmp_path, "rain all day,weather\nlate goal,sports\nfoggy morning,weather\n")
    assert list(read_csv_rows(path))[0] == {"column_1": "rain all day", "column_2": "weather"}


def test_a_first_row_of_numbers_is_data(tmp_path):
    path = write_text(tmp_path, "1,2\n3,4\n")
    assert list(read_csv_rows(path)) == [{"column_1": "1", "column_2": "2"}, {"column_1": "3", "column_2": "4"}]


def test_has_header_overrides_the_sniffing(tmp_path):
    path = write_text(tmp_path, "rain all day,weather\nlate goal,sports\nfoggy morning,weather\n")
    rows = list(read_csv_rows(path, has_header=True))
    assert rows[0] == {"rain all day": "late goal", "weather": "sports"}
    header_path = write_text(tmp_path, "text,label\nsun,weather\n")
    assert list(read_csv_rows(header_path, has_header=False))[0] == {"column_1": "text", "column_2": "label"}


def test_a_one_column_file_with_commas_in_its_text_stays_one_column(tmp_path):
    path = write_text(tmp_path, "text\nhello, world\nno commas here\none, two, three\n")
    assert [row["text"] for row in read_csv_rows(path)] == ["hello, world", "no commas here", "one, two, three"]


def test_blank_lines_crlf_and_a_byte_order_mark_are_accepted(tmp_path):
    path = write_text(tmp_path, "﻿text,label\r\nsun,weather\r\n\r\ngoal,sports\r\n")
    assert list(read_csv_rows(path)) == [{"text": "sun", "label": "weather"}, {"text": "goal", "label": "sports"}]


def test_an_empty_file_has_no_rows(tmp_path):
    assert list(read_csv_rows(write_text(tmp_path, ""))) == []


def test_a_row_with_the_wrong_cell_count_is_refused_with_its_line(tmp_path):
    error = read_error(write_text(tmp_path, "text,label\nsun,weather\ngoal,sports,extra\nrain,weather\n"))
    assert error.code == RRS_D1001
    assert "line 3" in error.message


def test_a_repeated_header_name_is_refused(tmp_path):
    error = read_error(write_text(tmp_path, "text,text\na,b\n"), has_header=True)
    assert error.code == RRS_D1001
    assert "twice" in error.message


def test_an_empty_header_name_is_refused(tmp_path):
    error = read_error(write_text(tmp_path, "text,\na,b\n"), has_header=True)
    assert error.code == RRS_D1001


def test_bytes_that_are_not_utf8_are_refused(tmp_path):
    error = read_error(write_text(tmp_path, "text,label\ncafé,a\n", encoding="latin-1"))
    assert error.code == RRS_D1001
    assert "UTF-8" in error.message


def test_bytes_that_are_not_utf8_after_the_sample_are_refused(tmp_path, monkeypatch):
    monkeypatch.setattr(csv_reader, "SNIFF_SAMPLE_CHARACTERS", 16)
    path = tmp_path / "rows.csv"
    path.write_bytes(b"text,label\n" + b"sun,weather\n" * 1000 + b"caf\xe9,a\n")
    error = read_error(path)
    assert error.code == RRS_D1001


def test_a_cell_over_the_csv_field_limit_is_refused(tmp_path):
    error = read_error(write_text(tmp_path, "text,label\n" + "x" * 200_000 + ",a\n"))
    assert error.code == RRS_D1001


def test_a_missing_file_is_a_source_error(tmp_path):
    error = read_error(tmp_path / "missing.csv")
    assert error.code == RRS_D3001
