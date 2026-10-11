"""Seeded mutations of the CSV and JSONL fixtures: each reader gives rows or DataError, never anything else.

The readers parse untrusted files, and Python has no fuzzing dependency (ADR-0037, contracts 12.5), so this test
is the fuzz target: it damages the fixture files in fixed, repeatable ways and reads each damaged copy. A failure
names the seed of the mutation, so it can be replayed with mutate_file(fixture, seed).
"""

import random
import time
from pathlib import Path

import pytest

from rastai_data import DataError, read_csv_rows, read_jsonl_rows

MUTATIONS_PER_FIXTURE = 300
# A read of any of these small files takes milliseconds; seconds would mean the parser is stuck on its input.
MAX_SECONDS_PER_READ = 5.0
MUTATION_SEED_PREFIX = "rastai_data.reader_mutations"
# Each mutated copy gets between one and this many edits.
MAX_EDITS_PER_MUTATION = 4
MAX_EDIT_LENGTH = 64
# Bytes that mean something to a JSON or CSV parser, or to a UTF-8 decoder, so the edits hit the parsers' edge cases
# far more often than random bytes would.
INTERESTING_BYTES = [
    b'"',
    b"'",
    b",",
    b";",
    b"\t",
    b"|",
    b"\n",
    b"\r",
    b"\r\n",
    b"{",
    b"}",
    b"[",
    b"]",
    b":",
    b"\\",
    b"\\u",
    b"\\ud800",
    b"\x00",
    b"\xff",
    b"\xc3",
    b"\xef\xbb\xbf",
    b"NaN",
    b"-Infinity",
    b"1e999",
    b"null",
    b'"id": "x", "id": "y"',
    b"[" * 3000,
    b"9" * 5000,
    b"x" * 140_000,
]
FIXTURE_READERS = [
    ("textcnn_tiny.jsonl", read_jsonl_rows),
    ("textcnn_tiny.csv", read_csv_rows),
]


def mutate_bytes(content: bytes, generator: random.Random) -> bytes:
    """`content` with one to MAX_EDITS_PER_MUTATION random edits."""
    edit_count = generator.randint(1, MAX_EDITS_PER_MUTATION)
    for _ in range(edit_count):
        content = apply_one_edit(content, generator)
    return content


def apply_one_edit(content: bytes, generator: random.Random) -> bytes:
    place = generator.randint(0, len(content))
    length = generator.randint(1, MAX_EDIT_LENGTH)
    edit = generator.choice(["flip", "delete", "insert_interesting", "insert_random", "duplicate", "truncate"])
    if edit == "flip" and place < len(content):
        flipped = content[place] ^ (1 << generator.randint(0, 7))
        return content[:place] + bytes([flipped]) + content[place + 1 :]
    if edit == "delete":
        return content[:place] + content[place + length :]
    if edit == "insert_interesting":
        return content[:place] + generator.choice(INTERESTING_BYTES) + content[place:]
    if edit == "insert_random":
        return content[:place] + generator.randbytes(length) + content[place:]
    if edit == "duplicate":
        return content[:place] + content[place : place + length] * 2 + content[place + length :]
    return content[:place]


def mutate_file(fixture: Path, seed: str) -> bytes:
    return mutate_bytes(fixture.read_bytes(), random.Random(seed))


def read_rows_or_data_error(reader, path: Path, seed: str) -> list[dict] | DataError:
    """The rows the reader returns, or the DataError it raises. Any other exception fails the test with the seed."""
    try:
        return list(reader(path))
    except DataError as error:
        return error
    except Exception as error:
        pytest.fail(f"{seed}: {reader.__name__} raised {type(error).__name__}: {error}")


@pytest.mark.fuzz
@pytest.mark.parametrize("fixture_name, reader", FIXTURE_READERS)
def test_mutated_fixtures_give_rows_or_data_error(fixtures_folder, tmp_path, fixture_name, reader):
    fixture = fixtures_folder / "datasets" / fixture_name
    mutated_path = tmp_path / fixture_name
    outcomes = {"rows": 0, "data_error": 0}
    for mutation_index in range(MUTATIONS_PER_FIXTURE):
        seed = f"{MUTATION_SEED_PREFIX}:{fixture_name}:{mutation_index}"
        mutated_path.write_bytes(mutate_file(fixture, seed))
        started = time.monotonic()
        outcome = read_rows_or_data_error(reader, mutated_path, seed)
        elapsed = time.monotonic() - started
        assert elapsed < MAX_SECONDS_PER_READ, f"{seed} took {elapsed:.1f} s"
        if isinstance(outcome, DataError):
            assert outcome.code.startswith("RRS-D"), seed
            outcomes["data_error"] += 1
        else:
            assert all(isinstance(row, dict) for row in outcome), seed
            outcomes["rows"] += 1
    # Both outcomes happen, so the mutations neither break every file nor leave every file readable.
    assert outcomes["rows"] > 0 and outcomes["data_error"] > 0, outcomes


@pytest.mark.fuzz
@pytest.mark.parametrize(
    "fixture_name, reader", [("textcnn_tiny.csv", read_jsonl_rows), ("textcnn_tiny.jsonl", read_csv_rows)]
)
def test_each_reader_survives_the_other_format(fixtures_folder, fixture_name, reader):
    outcome = read_rows_or_data_error(reader, fixtures_folder / "datasets" / fixture_name, "unmutated")
    assert isinstance(outcome, (list, DataError))
