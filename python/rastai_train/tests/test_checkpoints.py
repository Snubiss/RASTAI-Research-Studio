"""Checkpoint folders (rastai_train/checkpoints.py): written whole under `.partial` then renamed, a `.partial` folder
is never returned, a full disk leaves nothing behind, the keep-last-3 + best + pinned rule, and the fallback past a
damaged checkpoint."""

import errno
import json
import re
from pathlib import Path

import pytest
import torch
from rastai_formats.generated.command_train_start import TrainStartArgs
from rastai_formats.generated.settings import SettingsCheckpoint
from rastai_plugin.catalog.generated import RRS_T2001, RRS_T2004, RRS_T2005
from rastai_train.checkpoints import new_generated_id, recorded_validation_loss

import rastai_formats
from rastai_plugin import TrainingContext
from rastai_train import (
    CheckpointPolicy,
    TrainError,
    TrainingSettings,
    build_training_context,
    checkpoint_folder,
    checkpoint_policy,
    checkpoints_to_keep,
    find_latest_checkpoint,
    list_complete_checkpoints,
    remove_unkept_checkpoints,
    resume_from_latest_checkpoint,
    write_checkpoint,
)
from rastai_train import checkpoints as checkpoints_module

TEXTCNN_TEMPLATE = Path("templates/designs/textcnn.rrsmodel")
TINY_DATASET = Path("fixtures/datasets/textcnn_tiny.jsonl")
RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2N"
OTHER_RUN_ID = "run_01JA2B3C4D5E6F7G8H9J0K1M2P"
CHECKPOINT_FILES = ["optimizer.safetensors", "resume.json", "rng.safetensors", "weights.safetensors"]
# contracts 8.6: a prefix, then 26 Crockford base32 characters whose first holds only 3 bits.
CHECKPOINT_ID_PATTERN = re.compile(r"ckpt_[0-7][0-9A-HJKMNP-TV-Z]{25}")
RECIPE_STATE = {"labels": ["cooking", "music", "sports", "weather"]}


@pytest.fixture(autouse=True)
def keep_torch_global_state():
    """build_training_context seeds the global generator and turns deterministic mode on; the other packages' tests
    run in the same process and must find both as they were."""
    deterministic = torch.are_deterministic_algorithms_enabled()
    generator_state = torch.get_rng_state()
    yield
    torch.use_deterministic_algorithms(deterministic)
    torch.set_rng_state(generator_state)


def new_context(repo_root: Path, run_folder: Path, run_id: str = RUN_ID) -> TrainingContext:
    arguments = TrainStartArgs(
        recipe="classification",
        design_path=str(repo_root / TEXTCNN_TEMPLATE),
        dataset_path=str(repo_root / TINY_DATASET),
    )
    settings = TrainingSettings(max_steps=100, checkpoint=checkpoint_policy(SettingsCheckpoint(interval_minutes=0)))
    return build_training_context(arguments, run_id, run_folder, settings)


def write_at_step(context: TrainingContext, step: int, validation_loss: float | None = None) -> Path:
    context.progress.step = step
    return write_checkpoint(context.run_folder, step, context, RECIPE_STATE, validation_loss)


# ---- Writing ----


def test_a_checkpoint_is_one_folder_of_four_files_and_nothing_partial(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    folder = write_at_step(context, 1200)

    assert folder == tmp_path / "run" / "checkpoints" / "step-00001200"
    assert sorted(path.name for path in folder.iterdir()) == CHECKPOINT_FILES
    assert [path.name for path in folder.parent.iterdir()] == ["step-00001200"]


def test_resume_json_is_a_valid_rastai_resume_document(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    folder = write_at_step(context, 7)

    result = json.loads(rastai_formats.read_document("resume.json", (folder / "resume.json").read_bytes()))
    assert result["diagnostics"] == []
    document = json.loads(result["document_json"])
    assert document["type"] == "rastai.resume"
    assert document["run_id"] == RUN_ID
    assert document["step"] == 7
    assert document["weights_file"] == "weights.safetensors"
    assert document["optimizer_file"] == "optimizer.safetensors"
    assert document["rng_file"] == "rng.safetensors"
    assert document["scheduler"] == {"name": "warmup_cosine", "last_step": 7}
    assert document["recipe_state"] == RECIPE_STATE
    assert document["dataloader"]["dataset_hash"] == context.dataset_hash
    assert document["dataloader"]["split"] == "train"
    assert document["content_hash"].startswith("blake3:")


def test_the_validation_loss_is_kept_in_the_weights_metadata(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    assert recorded_validation_loss(write_at_step(context, 1, validation_loss=0.625)) == 0.625
    assert recorded_validation_loss(write_at_step(context, 2)) is None


def test_a_full_disk_removes_the_partial_folder_and_keeps_the_older_checkpoint(
    repo_root: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    context = new_context(repo_root, tmp_path / "run")
    older = write_at_step(context, 10)

    def full_disk(*arguments) -> None:
        raise OSError(errno.ENOSPC, "No space left on device")

    monkeypatch.setattr(checkpoints_module, "write_resume_json", full_disk)
    with pytest.raises(TrainError) as refused:
        write_at_step(context, 20)
    assert refused.value.code == RRS_T2001
    assert [path.name for path in older.parent.iterdir()] == ["step-00000010"]
    assert find_latest_checkpoint(context.run_folder) == older


def test_a_job_killed_mid_write_leaves_a_partial_folder_nothing_returns(
    repo_root: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    context = new_context(repo_root, tmp_path / "run")
    older = write_at_step(context, 10)

    def killed(*arguments) -> None:
        # Not an OSError: nothing cleans up, as after a kill -9 between the file writes and the rename.
        raise KeyboardInterrupt

    monkeypatch.setattr(checkpoints_module, "write_resume_json", killed)
    with pytest.raises(KeyboardInterrupt):
        write_at_step(context, 20)
    partial = context.run_folder / "checkpoints" / "step-00000020.partial"
    assert sorted(path.name for path in partial.iterdir()) == [
        "optimizer.safetensors",
        "rng.safetensors",
        "weights.safetensors",
    ]

    assert find_latest_checkpoint(context.run_folder) == older
    assert list_complete_checkpoints(context.run_folder) == [(10, older)]
    assert resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder)).step == 10


def test_a_partial_folder_is_never_returned_even_when_it_holds_every_file(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    older = write_at_step(context, 10)
    newer = write_at_step(context, 30)
    newer.rename(newer.with_name("step-00000030.partial"))
    (older.parent / "step-0000004").mkdir()  # seven digits: not a checkpoint name
    (older.parent / "notes.txt").write_text("not a checkpoint")

    assert find_latest_checkpoint(context.run_folder) == older
    assert list_complete_checkpoints(context.run_folder) == [(10, older)]


def test_a_new_write_replaces_a_leftover_partial_folder_of_its_step(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    leftover = checkpoint_folder(context.run_folder, 20).with_name("step-00000020.partial")
    leftover.mkdir(parents=True)
    (leftover / "weights.safetensors").write_bytes(b"half a file")

    folder = write_at_step(context, 20)
    assert not leftover.exists()
    assert sorted(path.name for path in folder.iterdir()) == CHECKPOINT_FILES


def test_a_run_without_checkpoints_has_no_latest(tmp_path: Path) -> None:
    assert find_latest_checkpoint(tmp_path / "run") is None
    assert list_complete_checkpoints(tmp_path / "run") == []


# ---- Retention ----


def test_keep_last_three_plus_the_best_and_the_pinned() -> None:
    steps = [10, 20, 30, 40, 50, 60, 70, 80]
    assert checkpoints_to_keep(steps, 3, best_step=20, pinned_steps=(40,)) == {20, 40, 60, 70, 80}
    assert checkpoints_to_keep(steps, 3, best_step=None, pinned_steps=()) == {60, 70, 80}
    assert checkpoints_to_keep(steps, 3, best_step=70, pinned_steps=(80, 5)) == {60, 70, 80}
    assert checkpoints_to_keep([10, 20], 3, best_step=None, pinned_steps=()) == {10, 20}


def test_removal_deletes_only_what_the_rule_doesnt_keep(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    for step in (10, 20, 30, 40, 50, 60):
        write_at_step(context, step)

    removed = remove_unkept_checkpoints(context.run_folder, 3, best_step=10, pinned_steps=(30,))
    assert [path.name for path in removed] == ["step-00000020"]
    kept_steps = [step for step, _folder in list_complete_checkpoints(context.run_folder)]
    assert kept_steps == [10, 30, 40, 50, 60]


# ---- The policy ----


def test_the_policy_fills_what_the_checkpoint_settings_leave_out() -> None:
    policy = checkpoint_policy(SettingsCheckpoint(keep_last=5), every_steps=4, pinned_steps=(8,))
    assert policy.checkpoint_settings == SettingsCheckpoint(
        keep_last=5, interval_minutes=15, preemption_budget_seconds=25
    )
    assert policy.every_steps == 4
    assert policy.pinned_steps == (8,)
    assert checkpoint_policy().checkpoint_settings == SettingsCheckpoint(
        keep_last=3, interval_minutes=15, preemption_budget_seconds=25
    )


def test_a_policy_with_unfilled_settings_is_a_bug_in_the_caller(repo_root: Path, tmp_path: Path) -> None:
    arguments = TrainStartArgs(
        recipe="classification",
        design_path=str(repo_root / TEXTCNN_TEMPLATE),
        dataset_path=str(repo_root / TINY_DATASET),
    )
    settings = TrainingSettings(max_steps=10, checkpoint=CheckpointPolicy(SettingsCheckpoint(keep_last=3)))
    with pytest.raises(ValueError):
        build_training_context(arguments, RUN_ID, tmp_path / "run", settings)


# ---- Ids ----


def test_checkpoint_ids_are_generated_ids_that_sort_by_time() -> None:
    first = new_generated_id("ckpt")
    second = new_generated_id("ckpt")
    assert CHECKPOINT_ID_PATTERN.fullmatch(first)
    assert first != second
    assert new_generated_id("doc").startswith("doc_")
    # The first 10 characters are the millisecond time; ids made later never sort before.
    assert first[:15] <= second[:15]


# ---- Resuming from the newest sound checkpoint ----


def test_resume_skips_a_damaged_newer_checkpoint(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    write_at_step(context, 10)
    newest = write_at_step(context, 20)
    (newest / "resume.json").write_text("{ not json")

    point = resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder))
    assert point.step == 10
    assert len(point.damaged_checkpoints) == 1
    assert "step-00000020" in point.damaged_checkpoints[0]
    # The run goes on from step 10, so the damaged step 20 is removed: never the run's latest or best again.
    assert not newest.exists()
    assert list_complete_checkpoints(context.run_folder) == [(10, checkpoint_folder(context.run_folder, 10))]


def test_the_best_after_a_fallback_is_never_the_damaged_newer_checkpoint(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    write_at_step(context, 10, validation_loss=0.9)
    newest = write_at_step(context, 20, validation_loss=0.1)
    (newest / "rng.safetensors").write_bytes(b"damaged")

    resumed = new_context(repo_root, context.run_folder)
    assert resume_from_latest_checkpoint(context.run_folder, resumed).step == 10
    assert resumed.progress.best_step == 10
    assert resumed.progress.best_validation_loss == 0.9


def test_resume_skips_a_checkpoint_whose_safetensors_header_is_damaged(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    write_at_step(context, 10)
    newest = write_at_step(context, 20)
    optimizer_file = newest / "optimizer.safetensors"
    damaged = bytearray(optimizer_file.read_bytes())
    damaged[9] = ord("!")  # inside the JSON header, right after the 8-byte length
    optimizer_file.write_bytes(bytes(damaged))

    point = resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder))
    assert point.step == 10


def test_resume_without_a_sound_checkpoint_is_rrs_t2004(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    with pytest.raises(TrainError) as nothing:
        resume_from_latest_checkpoint(context.run_folder, context)
    assert nothing.value.code == RRS_T2004

    only = write_at_step(context, 10)
    (only / "rng.safetensors").unlink()
    with pytest.raises(TrainError) as all_damaged:
        resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder))
    assert all_damaged.value.code == RRS_T2004
    assert "damaged" in all_damaged.value.message


def test_another_runs_checkpoint_is_refused_not_skipped(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run", run_id=OTHER_RUN_ID)
    write_at_step(context, 10)

    with pytest.raises(TrainError) as refused:
        resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder))
    assert refused.value.code == RRS_T2005
    assert OTHER_RUN_ID in refused.value.message


def test_a_damaged_file_name_in_resume_json_is_refused(repo_root: Path, tmp_path: Path) -> None:
    context = new_context(repo_root, tmp_path / "run")
    folder = write_at_step(context, 10)
    document = json.loads((folder / "resume.json").read_text())
    document["weights_file"] = "../../../weights.safetensors"
    document["content_hash"] = ""
    (folder / "resume.json").write_bytes(rastai_formats.write_document(json.dumps(document)))

    with pytest.raises(TrainError) as refused:
        resume_from_latest_checkpoint(context.run_folder, new_context(repo_root, context.run_folder))
    assert refused.value.code == RRS_T2004
    assert "../../../weights.safetensors" in refused.value.message
