"""CSV rows with the standard library's csv module, plus delimiter and header sniffing (layout python.md 4.1).

The file is untrusted, so the reader returns rows or raises DataError, never another exception
(tests/test_reader_mutations.py). Sniffing is written here instead of using csv.Sniffer: Sniffer's quote guess runs
a backtracking regular expression over the sample, which can take a very long time on a hostile file, and its
header guess calls a column of short words "not a header". The rules below are simple and run in linear time.
"""

import csv
import io
from collections.abc import Iterator
from pathlib import Path
from typing import TextIO

from rastai_plugin.catalog.generated import RRS_D1001, RRS_D3001

from rastai_data.errors import DataError

# Delimiters tried when sniffing, in order of preference when two fit equally well (an empty file gets the first).
CANDIDATE_DELIMITERS = (",", "\t", ";", "|")
# How much of the file sniffing looks at: enough rows to see a pattern, little enough to stay fast.
SNIFF_SAMPLE_CHARACTERS = 64 * 1024
SNIFF_ROW_COUNT = 50
# A header names columns; a first-row cell longer than this is data.
MAX_HEADER_NAME_CHARACTERS = 100
# Columns of a file without a header are named column_1, column_2, ...
GENERATED_COLUMN_PREFIX = "column_"
# "utf-8-sig" reads plain UTF-8 and also drops the byte-order mark some spreadsheet programs write first.
CSV_ENCODING = "utf-8-sig"
# Error messages quote at most this much of a parser's own explanation.
MAX_REASON_CHARACTERS = 200


def read_csv_rows(path: Path, has_header: bool | None = None) -> Iterator[dict]:
    """Each row as a dict of strings, in file order. `has_header` None means sniff it from the first rows.

    Blank lines are skipped. A row with a different number of cells than the first row is refused.
    """
    sample_rows, delimiter = sniff_sample(path)
    if has_header is None:
        has_header = looks_like_header(sample_rows)
    with open_csv_file(path) as csv_file:
        row_reader = csv.reader(csv_file, delimiter=delimiter)
        column_names = None
        for cells in read_cell_rows(row_reader, path):
            if column_names is None:
                column_names = first_row_column_names(cells, has_header, path, row_reader.line_num)
                if has_header:
                    continue
            yield row_from_cells(cells, column_names, path, row_reader.line_num)


def open_csv_file(path: Path) -> TextIO:
    try:
        # newline="" hands line endings to the csv module, which needs them for newlines inside quoted cells.
        return open(path, encoding=CSV_ENCODING, newline="")
    except OSError as error:
        raise DataError(RRS_D3001, f"{path}: the file can't be opened ({error.strerror})") from None


def read_cell_rows(row_reader: Iterator[list[str]], path: Path) -> Iterator[list[str]]:
    """The non-blank rows of a csv.reader, with every way reading can fail turned into DataError."""
    while True:
        try:
            cells = next(row_reader)
        except StopIteration:
            return
        except UnicodeDecodeError as error:
            raise DataError(RRS_D1001, f"{path}: byte {error.start} of a block isn't UTF-8 text") from None
        except csv.Error as error:
            reason = str(error)[:MAX_REASON_CHARACTERS]
            raise DataError(RRS_D1001, f"{path}: not valid CSV ({reason})") from None
        except OSError as error:
            raise DataError(RRS_D3001, f"{path}: the file can't be read ({error.strerror})") from None
        if cells:
            yield cells


def sniff_sample(path: Path) -> tuple[list[list[str]], str]:
    """The first rows of the file, split with the delimiter that fits them best, and that delimiter."""
    sample_text = read_sample_text(path)
    best_rows = []
    best_delimiter = CANDIDATE_DELIMITERS[0]
    best_score = None
    for delimiter in CANDIDATE_DELIMITERS:
        rows = sample_rows_with(sample_text, delimiter)
        score = delimiter_score(rows)
        if best_score is None or score > best_score:
            best_rows, best_delimiter, best_score = rows, delimiter, score
    return best_rows, best_delimiter


def read_sample_text(path: Path) -> str:
    with open_csv_file(path) as csv_file:
        try:
            # One character more than the sample tells whether the sample is the whole file.
            sample_text = csv_file.read(SNIFF_SAMPLE_CHARACTERS + 1)
        except UnicodeDecodeError as error:
            raise DataError(RRS_D1001, f"{path}: byte {error.start} of the start isn't UTF-8 text") from None
        except OSError as error:
            raise DataError(RRS_D3001, f"{path}: the file can't be read ({error.strerror})") from None
    if len(sample_text) > SNIFF_SAMPLE_CHARACTERS:
        # The last line of a cut sample is usually cut too; drop it so it can't look like a short row.
        sample_text = sample_text[:SNIFF_SAMPLE_CHARACTERS].rpartition("\n")[0]
    return sample_text


def sample_rows_with(sample_text: str, delimiter: str) -> list[list[str]]:
    """Up to SNIFF_ROW_COUNT non-blank rows of the sample. A sample the csv module refuses gives no rows."""
    rows = []
    try:
        for cells in csv.reader(io.StringIO(sample_text, newline=""), delimiter=delimiter):
            if cells:
                rows.append(cells)
            if len(rows) == SNIFF_ROW_COUNT:
                break
    except csv.Error:
        return []
    return rows


def delimiter_score(rows: list[list[str]]) -> tuple[bool, bool, int]:
    """How well a delimiter fits the sample rows; a larger tuple fits better. In order of weight:

    1. It splits the first row into columns. A file with a few bad rows still gets its real delimiter, and the
       bad rows are refused with their line numbers instead of the file quietly becoming one column.
    2. Every row has the same number of cells. A one-column file whose text holds commas splits unevenly on ",",
       so it stays one column.
    3. More columns.
    """
    if not rows:
        return (False, False, 0)
    first_row_count = len(rows[0])
    rows_agree = all(len(cells) == first_row_count for cells in rows)
    return (first_row_count > 1, rows_agree, first_row_count)


def looks_like_header(sample_rows: list[list[str]]) -> bool:
    """The first row is a header when its cells read like column names.

    Names are non-empty, short, unique and not numbers, and none of them shows up again further down its own
    column (a label column repeats its values; a header's "label" doesn't reappear as a label).
    A file of one such row is a header with no data rows: an exported empty table ("id,text,label") is far more
    common than a dataset of one row, and has_header=False reads it as data when it is one.
    """
    if not sample_rows:
        return False
    first_row = sample_rows[0]
    if len(set(first_row)) != len(first_row):
        return False
    for column_index, cell in enumerate(first_row):
        if not is_column_name(cell):
            return False
        if value_repeats_in_column(cell, column_index, sample_rows[1:]):
            return False
    return True


def is_column_name(cell: str) -> bool:
    name = cell.strip()
    return name != "" and len(name) <= MAX_HEADER_NAME_CHARACTERS and not is_number(name)


def is_number(text: str) -> bool:
    try:
        float(text)
    except ValueError:
        return False
    return True


def value_repeats_in_column(value: str, column_index: int, rows: list[list[str]]) -> bool:
    for cells in rows:
        if column_index < len(cells) and cells[column_index] == value:
            return True
    return False


def first_row_column_names(cells: list[str], has_header: bool, path: Path, line_number: int) -> list[str]:
    """The column names: the header's cells, or column_1 ... column_n for a file without one."""
    if not has_header:
        return [f"{GENERATED_COLUMN_PREFIX}{index + 1}" for index in range(len(cells))]
    names = [cell.strip() for cell in cells]
    if "" in names:
        raise DataError(RRS_D1001, f"{path}, line {line_number}: a header cell is empty")
    if len(set(names)) != len(names):
        raise DataError(RRS_D1001, f"{path}, line {line_number}: the header names a column twice")
    return names


def row_from_cells(cells: list[str], column_names: list[str], path: Path, line_number: int) -> dict:
    if len(cells) != len(column_names):
        raise DataError(
            RRS_D1001,
            f"{path}, line {line_number}: {len(cells)} cells where the first row has {len(column_names)}",
        )
    return dict(zip(column_names, cells, strict=True))
