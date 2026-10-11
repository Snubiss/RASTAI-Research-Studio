"""The resumable loader (M1-36): a restored loader yields exactly the batches an uninterrupted run would have."""

import json
import os
import subprocess
import sys

import pytest

from rastai_data import epoch_order, make_loader

EXAMPLE_COUNT = 10
BATCH_SIZE = 3
SHUFFLE_SEED = 42
# 10 examples in batches of 3 is four batches and the end-of-epoch None: five calls per epoch. 18 calls cross
# three epoch boundaries and stop in the middle of an epoch.
CALL_COUNT = 18
# Written down once from epoch_order(12, 42, 3). A change here reorders every resumed run.
GOLDEN_ORDER_12_42_3 = [0, 1, 8, 11, 7, 4, 2, 10, 6, 5, 3, 9]
HASH_SEEDS = ("1", "2")
ORDER_SCRIPT = "import json; from rastai_data import epoch_order; print(json.dumps(epoch_order(500, 11, 4)))"
# The loader's part of resume.json's dataloader field (contracts 5.2); the trainer adds dataset_hash and split.
LOADER_RESUME_FIELDS = {"shuffle_seed", "epoch", "position"}


def example_names(count: int = EXAMPLE_COUNT) -> list[str]:
    return [f"example-{index}" for index in range(count)]


def take_batches(loader, call_count: int) -> list:
    return [loader.next_batch() for _ in range(call_count)]


def resume_from(position: dict, examples: list, batch_size: int = BATCH_SIZE):
    return make_loader(examples, batch_size, position["shuffle_seed"], position["epoch"], position["position"])


def test_epoch_order_matches_the_golden():
    assert epoch_order(12, 42, 3) == GOLDEN_ORDER_12_42_3


def test_epoch_order_is_a_permutation_that_changes_with_seed_and_epoch():
    order = epoch_order(100, 1, 0)
    assert sorted(order) == list(range(100))
    assert order != list(range(100))
    assert epoch_order(100, 1, 1) != order
    assert epoch_order(100, 2, 0) != order


def test_epoch_order_is_the_same_under_two_hash_seeds():
    outputs = []
    for hash_seed in HASH_SEEDS:
        environment = dict(os.environ, PYTHONHASHSEED=hash_seed)
        completed = subprocess.run(
            [sys.executable, "-c", ORDER_SCRIPT], env=environment, capture_output=True, text=True, check=True
        )
        outputs.append(json.loads(completed.stdout))
    assert outputs[0] == outputs[1] == epoch_order(500, 11, 4)


def test_a_restored_loader_yields_exactly_the_next_batch():
    examples = example_names()
    uninterrupted = take_batches(make_loader(examples, BATCH_SIZE, SHUFFLE_SEED, 0, 0), CALL_COUNT)
    for stop_after in range(CALL_COUNT):
        interrupted = make_loader(examples, BATCH_SIZE, SHUFFLE_SEED, 0, 0)
        take_batches(interrupted, stop_after)
        # The position goes through JSON, as it does through resume.json.
        saved = json.loads(json.dumps(interrupted.position()))
        resumed = resume_from(saved, examples)
        assert take_batches(resumed, CALL_COUNT - stop_after) == uninterrupted[stop_after:], stop_after


def test_each_epoch_hands_out_every_example_once_then_none():
    examples = example_names()
    loader = make_loader(examples, BATCH_SIZE, SHUFFLE_SEED, 0, 0)
    for epoch in range(3):
        batches = take_batches(loader, 4)
        handed_out = [example for batch in batches for example in batch]
        assert sorted(handed_out) == sorted(examples)
        assert [len(batch) for batch in batches] == [3, 3, 3, 1]
        assert handed_out == [examples[index] for index in epoch_order(EXAMPLE_COUNT, SHUFFLE_SEED, epoch)]
        assert loader.next_batch() is None


def test_position_has_the_loader_resume_fields():
    loader = make_loader(example_names(), BATCH_SIZE, SHUFFLE_SEED, 2, 0)
    loader.next_batch()
    position = loader.position()
    assert set(position) == LOADER_RESUME_FIELDS
    assert position == {"shuffle_seed": SHUFFLE_SEED, "epoch": 2, "position": 3}


def test_the_end_of_epoch_none_moves_the_position_to_the_next_epoch():
    loader = make_loader(example_names(), BATCH_SIZE, SHUFFLE_SEED, 0, EXAMPLE_COUNT)
    assert loader.position()["epoch"] == 0
    assert loader.next_batch() is None
    assert (loader.position()["epoch"], loader.position()["position"]) == (1, 0)


def test_a_resume_with_another_batch_size_skips_and_repeats_nothing():
    examples = example_names()
    loader = make_loader(examples, BATCH_SIZE, SHUFFLE_SEED, 0, 0)
    first_part = take_batches(loader, 2)
    resumed = resume_from(loader.position(), examples, batch_size=4)
    second_part = take_batches(resumed, 1)
    handed_out = [example for batch in first_part + second_part for example in batch]
    assert sorted(handed_out) == sorted(examples)


@pytest.mark.parametrize(
    "examples, batch_size, start_epoch, start_position",
    [
        ([], 3, 0, 0),
        (["a"], 0, 0, 0),
        (["a"], 1, -1, 0),
        (["a"], 1, 0, 2),
        (["a"], 1, 0, -1),
    ],
)
def test_impossible_loader_arguments_are_refused(examples, batch_size, start_epoch, start_position):
    with pytest.raises(ValueError):
        make_loader(examples, batch_size, SHUFFLE_SEED, start_epoch, start_position)
