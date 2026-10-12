"""JSON Lines rows: one JSON object per line (layout python.md section 4.1).

The file is untrusted, so the reader returns rows or raises DataError, never another exception
(tests/test_reader_mutations.py). It is stricter than json.loads alone, with the same rules the Rust JSON core
keeps (contracts 5.1): every row is an object, a key appears once per object, NaN, Infinity and numbers too big for
a 64-bit float are refused, and so is a lone UTF-16 surrogate in any string or key.
"""

import json
import math
from collections.abc import Iterator
from pathlib import Path
from typing import BinaryIO

from rastai_plugin.catalog.generated import RRS_D1001, RRS_D3001

from rastai_data.errors import DataError

# A file with no newline would otherwise be read into memory whole as one line. 16 MiB is far above any row of
# training text and still small next to a job's memory.
MAX_LINE_BYTES = 16 * 1024 * 1024
# The start of a file saved as "UTF-8 with BOM" by some editors; it isn't part of the first row.
UTF8_BOM = "\ufeff"
# The characters JSON allows around a value, so a line holding only these is blank.
JSON_WHITESPACE = " \t\r\n"
# Error messages quote at most this much of a parser's own explanation.
MAX_REASON_CHARACTERS = 200


def read_jsonl_rows(path: Path) -> Iterator[dict]:
    """Each non-blank line of the file as a dict, in file order."""
    try:
        jsonl_file = open(path, "rb")
    except OSError as error:
        raise DataError(RRS_D3001, f"{path}: the file can't be opened ({error.strerror})") from None
    with jsonl_file:
        line_number = 0
        while True:
            line_bytes = read_line(jsonl_file, path, line_number + 1)
            if not line_bytes:
                return
            line_number += 1
            text = decode_line(line_bytes, path, line_number)
            if text.strip(JSON_WHITESPACE) == "":
                continue
            yield parse_row(text, path, line_number)


def read_line(jsonl_file: BinaryIO, path: Path, line_number: int) -> bytes:
    """The next line with its newline, or b"" at the end of the file. A line over MAX_LINE_BYTES is refused."""
    try:
        line_bytes = jsonl_file.readline(MAX_LINE_BYTES + 1)
    except OSError as error:
        raise DataError(RRS_D3001, f"{path}: the file can't be read ({error.strerror})") from None
    if len(line_bytes) > MAX_LINE_BYTES and not line_bytes.endswith(b"\n"):
        raise DataError(RRS_D1001, f"{path}, line {line_number}: the line is longer than {MAX_LINE_BYTES} bytes")
    return line_bytes


def decode_line(line_bytes: bytes, path: Path, line_number: int) -> str:
    try:
        text = line_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        raise DataError(RRS_D1001, f"{path}, line {line_number}: byte {error.start} isn't UTF-8 text") from None
    if line_number == 1:
        text = text.removeprefix(UTF8_BOM)
    return text


def parse_row(text: str, path: Path, line_number: int) -> dict:
    try:
        value = json.loads(
            text,
            object_pairs_hook=object_without_repeated_keys,
            parse_constant=refuse_constant,
            parse_float=finite_float,
            parse_int=int_that_fits_a_float,
        )
    except ValueError as error:
        # JSONDecodeError is a ValueError, and so are the refusals below and the integer-digits limit.
        raise DataError(RRS_D1001, f"{path}, line {line_number}: not valid JSON ({short_reason(error)})") from None
    except RecursionError:
        raise DataError(RRS_D1001, f"{path}, line {line_number}: the JSON is nested too deeply") from None
    if not isinstance(value, dict):
        raise DataError(RRS_D1001, f"{path}, line {line_number}: each line must be a JSON object")
    if holds_a_lone_surrogate(value):
        raise DataError(
            RRS_D1001, f"{path}, line {line_number}: a string holds a lone UTF-16 surrogate escape, which isn't text"
        )
    return value


def object_without_repeated_keys(pairs: list[tuple[str, object]]) -> dict:
    """json.loads keeps the last of two equal keys without a word; a row that says two things is refused."""
    row = {}
    for key, value in pairs:
        if key in row:
            raise ValueError(f"the key {key!r} appears twice in one object")
        row[key] = value
    return row


def refuse_constant(name: str) -> float:
    raise ValueError(f"{name} isn't a JSON number")


def finite_float(number_text: str) -> float:
    """json.loads quietly turns 1e999 into inf; a number a 64-bit float can't hold is refused instead."""
    number = float(number_text)
    if not math.isfinite(number):
        raise ValueError(f"{number_text[:MAX_REASON_CHARACTERS]} is too big for a 64-bit float")
    return number


def int_that_fits_a_float(number_text: str) -> int:
    """An integer stays a Python int, but one beyond a 64-bit float's range is refused, as the Rust core does.

    int() itself refuses more digits than sys.get_int_max_str_digits() with a ValueError, so a huge number can't
    make this slow.
    """
    number = int(number_text)
    try:
        float(number)
    except OverflowError:
        raise ValueError(f"an integer of {len(number_text)} characters is too big for a 64-bit float") from None
    return number


def holds_a_lone_surrogate(value: object) -> bool:
    """Whether any string or key in `value` holds a lone UTF-16 surrogate.

    json.loads turns an escape like "\\ud800" without its partner into a lone surrogate character. That isn't text:
    it can't be written as UTF-8, so a later step (assign_split encodes the row key) would fail with an error that
    isn't a DataError. A proper pair such as "\\ud83d\\ude00" becomes one character and is fine. The walk keeps its
    own list of values to visit instead of recursing, so a deeply nested row can't hit the recursion limit here.
    """
    values_to_visit = [value]
    while values_to_visit:
        current = values_to_visit.pop()
        if isinstance(current, dict):
            values_to_visit.extend(current.keys())
            values_to_visit.extend(current.values())
        elif isinstance(current, list):
            values_to_visit.extend(current)
        elif isinstance(current, str) and not is_utf8_text(current):
            return True
    return False


def is_utf8_text(text: str) -> bool:
    try:
        text.encode("utf-8")
    except UnicodeEncodeError:
        return False
    return True


def short_reason(error: ValueError) -> str:
    if isinstance(error, json.JSONDecodeError):
        reason = f"{error.msg} at column {error.colno}"
    else:
        reason = str(error)
    return reason[:MAX_REASON_CHARACTERS]
