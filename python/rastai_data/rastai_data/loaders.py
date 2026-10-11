"""Batches in a shuffled order that can be saved and restored exactly (layout python.md sections 4.1, 4.2, 5.4).

The order of an epoch is a pure function of (shuffle seed, epoch), so the loader has no random state to save: its
whole position is the epoch and the number of examples already handed out in it. The trainer writes that position
into resume.json's `dataloader` field (contracts 5.2, `rastai.resume/1`), beside the dataset hash and split it read
the examples from, and a loader made from it hands out exactly the batches an uninterrupted run would have.

A batch is the list of examples in epoch order, whatever their type. Turning it into tensors (right padding with
word_vocabulary.PADDING_ID, target masks) arrives with the first recipe that trains on it (M1-38).
"""

import random

# Mixed into every epoch's seed string, so the shuffle stream is never the same as another use of the same seed.
EPOCH_ORDER_SEED_PREFIX = "rastai_data.epoch_order"


def epoch_order(example_count: int, shuffle_seed: int, epoch: int) -> list[int]:
    """The example indices of one epoch, shuffled. The same seed and epoch give the same order on any machine.

    A private random.Random seeded with a string is used: string seeds are hashed with SHA-512, never with
    Python's salted hash(), and the shuffle algorithm doesn't change between Python versions.
    """
    shuffler = random.Random(f"{EPOCH_ORDER_SEED_PREFIX}:{shuffle_seed}:{epoch}")
    order = list(range(example_count))
    shuffler.shuffle(order)
    return order


class BatchLoader:
    """Hands out batches of one split, epoch after epoch. Build it with make_loader."""

    def __init__(
        self, examples: list, batch_size: int, shuffle_seed: int, start_epoch: int, start_position: int
    ) -> None:
        self.examples = examples
        self.batch_size = batch_size
        self.shuffle_seed = shuffle_seed
        self.epoch = start_epoch
        # The number of examples of this epoch already handed out.
        self.position_in_epoch = start_position
        self.order = epoch_order(len(examples), shuffle_seed, start_epoch)

    def next_batch(self) -> list | None:
        """The next batch of this epoch, or None once when the epoch is used up.

        None lets the trainer count epochs; the call after it starts the next epoch. The last batch of an epoch
        holds the examples that are left, so it may be smaller than batch_size.
        """
        if self.position_in_epoch >= len(self.examples):
            self.start_next_epoch()
            return None
        end = min(self.position_in_epoch + self.batch_size, len(self.examples))
        batch = []
        for example_index in self.order[self.position_in_epoch : end]:
            batch.append(self.examples[example_index])
        self.position_in_epoch = end
        return batch

    def position(self) -> dict:
        """Where the loader is: the loader's part of resume.json's `dataloader` field, under the same keys.

        make_loader(examples, batch_size, shuffle_seed, epoch, position) resumes it. The position counts examples,
        not batches, so a resume with another batch size still skips and repeats nothing. The dataset hash and split
        of that field come from the job, not from the loader, so the trainer adds them.
        """
        return {
            "shuffle_seed": self.shuffle_seed,
            "epoch": self.epoch,
            "position": self.position_in_epoch,
        }

    def start_next_epoch(self) -> None:
        self.epoch += 1
        self.position_in_epoch = 0
        self.order = epoch_order(len(self.examples), self.shuffle_seed, self.epoch)


def make_loader(
    examples: list, batch_size: int, shuffle_seed: int, start_epoch: int, start_position: int
) -> BatchLoader:
    """A loader at (start_epoch, start_position): 0 and 0 for a new run, the saved position() for a resume."""
    check_loader_arguments(len(examples), batch_size, start_epoch, start_position)
    return BatchLoader(examples, batch_size, shuffle_seed, start_epoch, start_position)


def check_loader_arguments(example_count: int, batch_size: int, start_epoch: int, start_position: int) -> None:
    if example_count == 0:
        raise ValueError("a loader needs at least one example")
    if batch_size < 1:
        raise ValueError(f"batch_size must be at least 1; got {batch_size}")
    if start_epoch < 0:
        raise ValueError(f"start_epoch can't be negative; got {start_epoch}")
    if not 0 <= start_position <= example_count:
        raise ValueError(f"start_position must be between 0 and {example_count}; got {start_position}")
