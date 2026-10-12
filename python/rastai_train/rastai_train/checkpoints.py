"""Checkpoint folders: written whole or not at all, found newest first, kept by the retention rule (layout python.md
sections 5.1 and 5.4; contracts 8.4; ADR-0024).

A checkpoint is `<run folder>/checkpoints/step-NNNNNNNN/` holding weights.safetensors, optimizer.safetensors,
rng.safetensors and resume.json (resume.py). It is written into `step-NNNNNNNN.partial/`, every file synced, then
renamed: the rename is the moment it exists. A job killed during the write leaves only the `.partial` folder, which
nothing here ever returns and the next write at that step replaces (the daemon's GC removes the others).

A resume that falls back past newer checkpoints it can't load leaves them on disk for inspection (flow 5.2): the
error may be a passing one (a file it can't open for the moment) and nothing here can tell, so nothing is deleted on
a guess. The run goes on from the older step and its progress names the skipped steps
(`TrainingProgress.skipped_checkpoint_steps`): its latest, its best and keep_last leave them out until the run writes
that step again, which replaces the folder. The daemon's GC removes the others on its usual rules.

Retention keeps the newest `keep_last` complete checkpoints, the best (lowest validation.loss) and every pinned one;
the loop removes the rest after each save, so a run's disk use stays at keep_last + best + the one being written
(platform flow 8.1's estimate).
"""

import errno
import math
import os
import re
import shutil
import time
from pathlib import Path

from rastai_plugin.catalog.generated import RRS_T2001, RRS_T2002, RRS_T2004

import rastai_formats
from rastai_models import ModelError, SafetensorsFile, sync_folder, write_safetensors
from rastai_plugin import TrainingContext
from rastai_train.errors import TrainError
from rastai_train.optimizer import optimizer_state_tensors, trainable_parameters
from rastai_train.resume import (
    OPTIMIZER_FILE,
    RNG_FILE,
    WEIGHTS_FILE,
    ResumePoint,
    load_resume_state,
    rng_state_tensors,
    write_resume_json,
)

CHECKPOINTS_FOLDER_NAME = "checkpoints"
STEP_FOLDER_PREFIX = "step-"
# Eight digits, so folder names sort by step as plain text (contracts 8.4: step-00001200).
STEP_DIGITS = 8
PARTIAL_SUFFIX = ".partial"
# A complete checkpoint folder's name; `.partial` folders and anything else don't match.
COMPLETE_FOLDER_PATTERN = re.compile(r"step-(\d{8})")
# weights.safetensors' metadata keeps the checkpoint's validation.loss, so a resumed run knows its best checkpoint
# without a file the contract doesn't list.
VALIDATION_LOSS_KEY = "validation_loss"
# Generated ids (contracts 8.6): `<prefix>_<26 Crockford base32>` of a 48-bit Unix millisecond time and 80 random
# bits, the same layout as rastai-core's new_id, so they sort by time like every other id.
CHECKPOINT_ID_PREFIX = "ckpt"
DOCUMENT_ID_PREFIX = "doc"
CROCKFORD_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
ID_RANDOM_BYTES = 10
ID_RANDOM_BITS = ID_RANDOM_BYTES * 8
ID_CHARACTERS = 26
BITS_PER_CHARACTER = 5
CHARACTER_MASK = 0b11111


def checkpoint_folder(run_folder: Path, step: int) -> Path:
    """Where the complete checkpoint of `step` lives."""
    return run_folder / CHECKPOINTS_FOLDER_NAME / f"{STEP_FOLDER_PREFIX}{step:0{STEP_DIGITS}d}"


def write_checkpoint(
    run_folder: Path, step: int, context: TrainingContext, recipe_state: dict, validation_loss: float | None
) -> Path:
    """Writes the run as it stands as the checkpoint of `step` and returns its folder. A full disk removes the
    `.partial` folder and raises RRS-T2001; the previous checkpoints are untouched."""
    final_folder = checkpoint_folder(run_folder, step)
    partial_folder = final_folder.with_name(final_folder.name + PARTIAL_SUFFIX)
    # A leftover of a job killed while it wrote this step.
    remove_folder(partial_folder)
    try:
        partial_folder.mkdir(parents=True)
        write_checkpoint_files(partial_folder, context, recipe_state, validation_loss)
        sync_folder(partial_folder)
    except OSError as error:
        remove_folder(partial_folder)
        if error.errno == errno.ENOSPC:
            raise TrainError(RRS_T2001, f"The checkpoint of step {step} didn't fit on the disk.") from error
        raise
    # A complete folder of this step is one a resume skipped (or one copied in by hand): the step's new checkpoint
    # replaces it, since a rename can't.
    remove_folder(final_folder)
    os.rename(partial_folder, final_folder)
    sync_folder(final_folder.parent)
    return final_folder


def write_checkpoint_files(
    folder: Path, context: TrainingContext, recipe_state: dict, validation_loss: float | None
) -> None:
    weights_metadata = {}
    if validation_loss is not None:
        weights_metadata[VALIDATION_LOSS_KEY] = repr(validation_loss)
    write_safetensors(folder / WEIGHTS_FILE, context.model.graph_parameters(), weights_metadata)
    optimizer_tensors = optimizer_state_tensors(context.optimizer, trainable_parameters(context.model))
    write_safetensors(folder / OPTIMIZER_FILE, optimizer_tensors)
    write_safetensors(folder / RNG_FILE, rng_state_tensors())
    write_resume_json(folder, context, recipe_state, new_generated_id(DOCUMENT_ID_PREFIX))


def remove_folder(folder: Path) -> None:
    if folder.exists():
        shutil.rmtree(folder)


def new_generated_id(prefix: str) -> str:
    """A new `<prefix>_<26 characters>` id: the time in milliseconds, then random bits, in Crockford base32."""
    unix_ms = time.time_ns() // 1_000_000
    value = (unix_ms << ID_RANDOM_BITS) | int.from_bytes(os.urandom(ID_RANDOM_BYTES), "big")
    characters = []
    for index in range(ID_CHARACTERS):
        shift = BITS_PER_CHARACTER * (ID_CHARACTERS - 1 - index)
        characters.append(CROCKFORD_ALPHABET[(value >> shift) & CHARACTER_MASK])
    return f"{prefix}_{''.join(characters)}"


def checkpoint_file_hashes(folder: Path) -> dict[str, str]:
    """Each file of a checkpoint by name with its BLAKE3 hash from the wheel, for checkpoint.saved: the daemon checks
    them before it resumes from the folder (flow 5.2)."""
    hashes = {}
    for path in sorted(folder.iterdir()):
        hashes[path.name] = rastai_formats.hash_bytes(path.read_bytes())
    return hashes


# ---- Finding ----


def list_complete_checkpoints(run_folder: Path, skipped_steps: tuple[int, ...] = ()) -> list[tuple[int, Path]]:
    """Every complete checkpoint of the run as (step, folder), oldest first, but the `skipped_steps` a resume fell
    back past. `.partial` folders are never listed."""
    checkpoints_folder = run_folder / CHECKPOINTS_FOLDER_NAME
    if not checkpoints_folder.is_dir():
        return []
    checkpoints = []
    for entry in checkpoints_folder.iterdir():
        matched = COMPLETE_FOLDER_PATTERN.fullmatch(entry.name)
        if not matched or not entry.is_dir():
            continue
        step = int(matched.group(1))
        if step not in skipped_steps:
            checkpoints.append((step, entry))
    return sorted(checkpoints)


def find_latest_checkpoint(run_folder: Path, skipped_steps: tuple[int, ...] = ()) -> Path | None:
    """The newest complete checkpoint but the `skipped_steps`, or None when the run has none."""
    checkpoints = list_complete_checkpoints(run_folder, skipped_steps)
    if not checkpoints:
        return None
    return checkpoints[-1][1]


def resume_from_latest_checkpoint(run_folder: Path, context: TrainingContext) -> ResumePoint:
    """Loads the newest complete checkpoint whose files load into the run, trying older ones past files it can't
    read or the wheel refuses (RRS-T2002). The ones it passed stay on disk, each named in `damaged_checkpoints` for
    the job to report, and their steps go into the run's `skipped_checkpoint_steps`, so its latest, best and
    keep_last leave them out. None at all is RRS-T2004; one of another run stops at once (RRS-T2005)."""
    damaged = []
    skipped_steps = []
    for step, folder in reversed(list_complete_checkpoints(run_folder)):
        try:
            point = load_resume_state(folder, context)
        except TrainError as error:
            if error.code != RRS_T2002:
                raise
            damaged.append(error.message)
            skipped_steps.append(step)
            continue
        point.damaged_checkpoints = damaged
        context.progress.skipped_checkpoint_steps = tuple(sorted(skipped_steps))
        restore_best_checkpoint(run_folder, context)
        return point
    reason = f" ({len(damaged)} damaged: {damaged[0]})" if damaged else ""
    raise TrainError(RRS_T2004, f"No complete checkpoint to resume from in {run_folder}{reason}.")


def restore_best_checkpoint(run_folder: Path, context: TrainingContext) -> None:
    """Sets the run's best checkpoint from the validation losses its checkpoints recorded, leaving out the newer
    ones the resume skipped: the run hasn't reached them."""
    skipped_steps = context.progress.skipped_checkpoint_steps
    for step, folder in list_complete_checkpoints(run_folder, skipped_steps):
        loss = recorded_validation_loss(folder)
        best_loss = context.progress.best_validation_loss
        if loss is not None and (best_loss is None or loss < best_loss):
            context.progress.best_step = step
            context.progress.best_validation_loss = loss


def recorded_validation_loss(folder: Path) -> float | None:
    """The validation.loss in a checkpoint's weights metadata; None when it has none, the file is damaged or the
    value isn't finite. A NaN written before the loop left non-finite losses out would otherwise become a best that
    nothing can beat (NaN compares false with everything)."""
    try:
        with SafetensorsFile(folder / WEIGHTS_FILE) as weights_file:
            text = weights_file.metadata.get(VALIDATION_LOSS_KEY)
    except (OSError, ModelError):
        return None
    if text is None:
        return None
    try:
        loss = float(text)
    except ValueError:
        return None
    if not math.isfinite(loss):
        return None
    return loss


# ---- Retention ----


def checkpoints_to_keep(
    steps: list[int], keep_last: int, best_step: int | None, pinned_steps: tuple[int, ...]
) -> set[int]:
    """The steps the retention rule keeps: the newest `keep_last`, the best and the pinned ones."""
    newest_first = sorted(steps, reverse=True)
    kept = set(newest_first[:keep_last])
    for step in (best_step, *pinned_steps):
        if step in steps:
            kept.add(step)
    return kept


def remove_unkept_checkpoints(
    run_folder: Path,
    keep_last: int,
    best_step: int | None,
    pinned_steps: tuple[int, ...],
    skipped_steps: tuple[int, ...] = (),
) -> list[Path]:
    """Removes the complete checkpoints the retention rule doesn't keep and returns their folders. The
    `skipped_steps` a resume fell back past are neither counted nor removed: they stay for inspection."""
    checkpoints = list_complete_checkpoints(run_folder, skipped_steps)
    kept = checkpoints_to_keep([step for step, _folder in checkpoints], keep_last, best_step, pinned_steps)
    removed = []
    for step, folder in checkpoints:
        if step not in kept:
            shutil.rmtree(folder)
            removed.append(folder)
    return removed
