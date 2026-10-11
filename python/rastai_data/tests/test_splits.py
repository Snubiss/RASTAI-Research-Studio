"""Splits are stable across runs and machines (M1-36): zlib.crc32 of the row key, never Python's hash()."""

import json
import os
import subprocess
import sys
from collections import Counter

import pytest

from rastai_data import assign_split, read_jsonl_rows

RATIOS = {"train": 0.8, "validation": 0.1, "test": 0.1}
# Written down once from assign_split(key, RATIOS, seed=1). A change here means every existing dataset's rows move
# between splits, so it needs a new seed scheme, not an updated golden.
GOLDEN_SPLITS_SEED_1 = {
    "tiny-0000": "train",
    "tiny-0001": "test",
    "tiny-0002": "train",
    "tiny-0005": "test",
    "tiny-0013": "validation",
    "tiny-0017": "validation",
    "tiny-0019": "train",
}
# Two different string-hash salts: if anything used hash(), the two runs would disagree.
HASH_SEEDS = ("1", "2")
# 2,000 rows at 10 % each: a binomial standard deviation of about 13 rows, so 50 is far outside chance.
SPLIT_COUNT_TOLERANCE = 50

# Prints every fixture row's split as JSON, so two interpreters can be compared.
SPLITS_SCRIPT = """
import json, sys
from pathlib import Path
from rastai_data import assign_split, read_jsonl_rows
rows = read_jsonl_rows(Path(sys.argv[1]))
ratios = {"train": 0.8, "validation": 0.1, "test": 0.1}
print(json.dumps({row["id"]: assign_split(row["id"], ratios, 7) for row in rows}))
"""


def fixture_rows(fixtures_folder) -> list[dict]:
    return list(read_jsonl_rows(fixtures_folder / "datasets" / "textcnn_tiny.jsonl"))


def test_assignments_match_the_golden():
    for row_key, split in GOLDEN_SPLITS_SEED_1.items():
        assert assign_split(row_key, RATIOS, seed=1) == split


def test_assignments_are_the_same_under_two_hash_seeds(fixtures_folder):
    fixture_path = str(fixtures_folder / "datasets" / "textcnn_tiny.jsonl")
    outputs = []
    for hash_seed in HASH_SEEDS:
        environment = dict(os.environ, PYTHONHASHSEED=hash_seed)
        completed = subprocess.run(
            [sys.executable, "-c", SPLITS_SCRIPT, fixture_path],
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        outputs.append(json.loads(completed.stdout))
    assert outputs[0] == outputs[1]
    in_this_process = {row["id"]: assign_split(row["id"], RATIOS, 7) for row in fixture_rows(fixtures_folder)}
    assert outputs[0] == in_this_process


def test_split_sizes_follow_the_ratios(fixtures_folder):
    rows = fixture_rows(fixtures_folder)
    counts = Counter(assign_split(row["id"], RATIOS, seed=0) for row in rows)
    for name, ratio in RATIOS.items():
        assert abs(counts[name] - ratio * len(rows)) < SPLIT_COUNT_TOLERANCE


def test_the_order_of_the_ratios_does_not_matter(fixtures_folder):
    reordered = {"test": 0.1, "train": 0.8, "validation": 0.1}
    for row in fixture_rows(fixtures_folder):
        assert assign_split(row["id"], RATIOS, 3) == assign_split(row["id"], reordered, 3)


def test_another_seed_moves_rows(fixtures_folder):
    rows = fixture_rows(fixtures_folder)
    moved = [row for row in rows if assign_split(row["id"], RATIOS, 1) != assign_split(row["id"], RATIOS, 2)]
    assert moved


def test_a_split_with_ratio_zero_gets_no_rows(fixtures_folder):
    ratios = {"train": 0.9, "validation": 0.0, "test": 0.1}
    splits = {assign_split(row["id"], ratios, 0) for row in fixture_rows(fixtures_folder)}
    assert splits == {"train", "test"}


def test_one_split_takes_every_row():
    assert assign_split("any row", {"train": 1.0}, 0) == "train"


def test_ratios_a_rounding_error_short_of_one_are_accepted():
    ratios = {"a": 0.7, "b": 0.2, "c": 0.0999999999}
    assert assign_split("row", ratios, 0) in ratios


@pytest.mark.parametrize(
    "ratios",
    [
        {},
        {"train": 0.5, "test": 0.4},
        {"train": 1.2, "test": -0.2},
        {"train": 0.9, "test": 0.2},
    ],
)
def test_ratios_that_are_not_a_split_are_refused(ratios):
    with pytest.raises(ValueError):
        assign_split("row", ratios, 0)
