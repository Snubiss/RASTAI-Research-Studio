"""rastai_train: the trainer and its recipes (layout python.md section 5). M1-38 lands the loop, the optimizer and
schedule, checkpoints, the resume contract and the classification recipe."""

from rastai_train.checkpoints import (
    checkpoint_folder,
    checkpoints_to_keep,
    find_latest_checkpoint,
    list_complete_checkpoints,
    remove_unkept_checkpoints,
    resume_from_latest_checkpoint,
    write_checkpoint,
)
from rastai_train.context import CheckpointPolicy, TrainingProgress, TrainingSettings, build_training_context
from rastai_train.errors import TrainError
from rastai_train.loop import TrainingOutcome, optimizer_step, preempt_at_step_boundary, train
from rastai_train.recipes import FIRST_PARTY_RECIPES, register_first_party
from rastai_train.recipes.classification import ClassificationRecipe
from rastai_train.resume import ResumePoint, load_resume_state

__all__ = [
    "FIRST_PARTY_RECIPES",
    "CheckpointPolicy",
    "ClassificationRecipe",
    "ResumePoint",
    "TrainError",
    "TrainingOutcome",
    "TrainingProgress",
    "TrainingSettings",
    "build_training_context",
    "checkpoint_folder",
    "checkpoints_to_keep",
    "find_latest_checkpoint",
    "list_complete_checkpoints",
    "load_resume_state",
    "optimizer_step",
    "preempt_at_step_boundary",
    "register_first_party",
    "remove_unkept_checkpoints",
    "resume_from_latest_checkpoint",
    "train",
    "write_checkpoint",
]
