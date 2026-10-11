"""Deterministic splits: each row's split comes from a stable hash of its key (layout python.md sections 4.1, 4.5).

The hash is zlib.crc32, never Python's hash(), which is salted per process: the same row lands in the same split in
every run, on every machine, so a held-out row never leaks into training after a restart.
"""

import zlib

# zlib.crc32 returns an unsigned 32-bit integer; dividing by this maps it into [0, 1).
CRC32_RANGE = 2**32
# Ratios written by hand ("0.8, 0.1, 0.1") don't add up to exactly 1.0 in floating point.
RATIO_SUM_TOLERANCE = 1e-6


def assign_split(row_key: str, ratios: dict[str, float], seed: int) -> str:
    """The split name for one row, with probability `ratios[name]`. Changing `seed` reshuffles every row.

    Splits are taken in name order, not in the dict's order, so ratios read from a file in any key order give the
    same answer.
    """
    check_ratios(ratios)
    point = split_point(row_key, seed)
    cumulative = 0.0
    for name in sorted(ratios):
        cumulative += ratios[name]
        if point < cumulative:
            return name
    # The sum can be a rounding error short of 1.0; a point in that sliver goes to the last split that takes rows.
    return last_split_with_rows(ratios)


def split_point(row_key: str, seed: int) -> float:
    """A number in [0, 1) that depends only on the seed and the row key."""
    hashed = zlib.crc32(f"{seed}:{row_key}".encode())
    return hashed / CRC32_RANGE


def check_ratios(ratios: dict[str, float]) -> None:
    if not ratios:
        raise ValueError("ratios must name at least one split")
    for name, ratio in ratios.items():
        if not 0.0 <= ratio <= 1.0:
            raise ValueError(f"the ratio of split {name!r} must be between 0 and 1; got {ratio}")
    total = sum(ratios.values())
    if abs(total - 1.0) > RATIO_SUM_TOLERANCE:
        raise ValueError(f"split ratios must add up to 1; they add up to {total}")


def last_split_with_rows(ratios: dict[str, float]) -> str:
    names_with_rows = [name for name in sorted(ratios) if ratios[name] > 0.0]
    return names_with_rows[-1]
