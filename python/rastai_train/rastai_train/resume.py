"""The resume contract: resume.json (`rastai.resume/1`), rng.safetensors and loading a checkpoint back into a run
(layout python.md sections 5.1 and 5.4; training-evaluation flow 5.2; ADR-0024).

| Part | File | Restored |
|---|---|---|
| Weights | weights.safetensors, tensors named by graph path | strictly, by name (bind_weights) |
| Optimizer | optimizer.safetensors, `<parameter>.<state>` | by parameter name (restore_optimizer_state) |
| RNG | rng.safetensors: `torch_cpu` and `cuda_<index>`, byte tensors | after recipe.prepare, before the first batch |
| Dataloader | resume.json dataloader{dataset_hash, split, shuffle_seed, epoch, position} | make_loader there |
| Schedule | resume.json scheduler{name, last_step} | recomputed from the step |
| Recipe | resume.json recipe_state{} | Recipe.restore_state, after prepare |

Nothing is pickled: tensors are safetensors and the rest is JSON written and read through the rastai_formats wheel,
the one validation of a document. Every file of a checkpoint is read and its header checked before the model, the
optimizer or the loader changes, so a damaged checkpoint (RRS-T2002) leaves the run as it was for the next older one.
"""

import json
import os
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path

import torch
from rastai_formats.generated.resume import (
    ResumeDataloader,
    ResumeState,
    resume_state_from_json,
    resume_state_to_json,
)
from rastai_plugin.catalog.generated import RRS_T2002, RRS_T2005

import rastai_formats
from rastai_data import BatchLoader, make_loader
from rastai_models import ModelError, SafetensorsFile, bind_weights
from rastai_plugin import TrainingContext
from rastai_train.errors import TrainError
from rastai_train.optimizer import (
    check_scheduler_state,
    restore_optimizer_state,
    scheduler_state,
    trainable_parameters,
)

RESUME_FILE = "resume.json"
WEIGHTS_FILE = "weights.safetensors"
OPTIMIZER_FILE = "optimizer.safetensors"
RNG_FILE = "rng.safetensors"
RESUME_SCHEMA_URL = "https://schemas.rastai.dev/rastai.resume/1.json"
RESUME_TYPE = "rastai.resume"
RESUME_SCHEMA_VERSION = 1
# rng.safetensors' tensor names (contracts 8.4).
CPU_RNG_NAME = "torch_cpu"
CUDA_RNG_PREFIX = "cuda_"
# A refused file's code starts with this when the wheel found the bytes damaged; the compiler's RRS-M codes mean
# the file is sound but belongs to another design.
FORMATS_CODE_PREFIX = "RRS-E"
CREATED_AT_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
ERROR_SEVERITY = "error"


@dataclass
class ResumePoint:
    """A checkpoint loaded into a run. The loop hands `recipe_state` to Recipe.restore_state and puts the random
    generators back after recipe.prepare, so the first batch draws exactly what it would have drawn."""

    checkpoint_folder: Path
    step: int
    recipe_state: dict
    rng_tensors: dict[str, torch.Tensor]
    # Newer complete checkpoints skipped because their files were damaged (RRS-T2002), for the job to report.
    damaged_checkpoints: list[str] = field(default_factory=list)


# ---- Writing ----


def write_resume_json(folder: Path, context: TrainingContext, recipe_state: dict, document_id: str) -> None:
    """resume.json for the run as it stands, validated and content-hashed by the wheel, synced to disk."""
    path = folder / RESUME_FILE
    with open(path, "wb") as file:
        file.write(resume_json_bytes(context, recipe_state, document_id))
        file.flush()
        os.fsync(file.fileno())


def resume_json_bytes(context: TrainingContext, recipe_state: dict, document_id: str) -> bytes:
    progress = context.progress
    loader_position = context.loader.position()
    state = ResumeState(
        schema=RESUME_SCHEMA_URL,
        type=RESUME_TYPE,
        schema_version=RESUME_SCHEMA_VERSION,
        id=document_id,
        run_id=context.run_id,
        step=progress.step,
        epoch=loader_position["epoch"],
        samples_seen=progress.samples_seen,
        dataloader=ResumeDataloader(
            dataset_hash=context.dataset_hash,
            split=context.split,
            shuffle_seed=loader_position["shuffle_seed"],
            epoch=loader_position["epoch"],
            position=loader_position["position"],
        ),
        rng_file=RNG_FILE,
        weights_file=WEIGHTS_FILE,
        optimizer_file=OPTIMIZER_FILE,
        scheduler=scheduler_state(progress.step),
        recipe_state=recipe_state,
        created_at=datetime.now(UTC).strftime(CREATED_AT_FORMAT),
    )
    return rastai_formats.write_document(json.dumps(resume_state_to_json(state)))


def rng_state_tensors() -> dict[str, torch.Tensor]:
    """The torch CPU generator and each GPU's, as the byte tensors torch gives (rng.safetensors)."""
    tensors = {CPU_RNG_NAME: torch.get_rng_state()}
    if torch.cuda.is_available():
        for device_index in range(torch.cuda.device_count()):
            tensors[f"{CUDA_RNG_PREFIX}{device_index}"] = torch.cuda.get_rng_state(device_index)
    return tensors


def restore_rng_state(tensors: dict[str, torch.Tensor]) -> None:
    """Puts the generators back. A GPU the checkpoint had and this machine lacks has nothing to restore."""
    torch.set_rng_state(tensors[CPU_RNG_NAME])
    for name, tensor in tensors.items():
        if not name.startswith(CUDA_RNG_PREFIX):
            continue
        device_index = int(name.removeprefix(CUDA_RNG_PREFIX))
        if torch.cuda.is_available() and device_index < torch.cuda.device_count():
            torch.cuda.set_rng_state(tensor, device_index)


# ---- Reading ----


def load_resume_state(checkpoint_folder: Path, context: TrainingContext) -> ResumePoint:
    """Loads one complete checkpoint into the run: weights, optimizer state, loader position and progress. The
    random generators and the recipe state come back in the loop (ResumePoint). Damaged files are RRS-T2002 (the
    caller may try an older checkpoint); a checkpoint of another run, dataset or design is RRS-T2005."""
    state = read_resume_json(checkpoint_folder)
    check_same_run(state, context)
    check_scheduler_state(state.scheduler or {}, state.step)
    weight_files = [open_checkpoint_file(checkpoint_folder, state.weights_file)]
    optimizer_tensors = read_all_tensors(checkpoint_folder, state.optimizer_file or OPTIMIZER_FILE)
    rng_tensors = read_all_tensors(checkpoint_folder, state.rng_file)
    if CPU_RNG_NAME not in rng_tensors:
        raise TrainError(RRS_T2002, f"{checkpoint_folder / state.rng_file} has no {CPU_RNG_NAME} generator state.")
    try:
        bind_weights(context.model, weight_files)
    except ModelError as error:
        raise checkpoint_error(error, checkpoint_folder) from None
    finally:
        weight_files[0].close()
    restore_optimizer_state(context.optimizer, trainable_parameters(context.model), optimizer_tensors)
    context.loader = loader_at(context, state.dataloader)
    point = ResumePoint(checkpoint_folder, state.step, dict(state.recipe_state or {}), rng_tensors)
    set_progress(context, state, point)
    return point


def read_resume_json(checkpoint_folder: Path) -> ResumeState:
    """resume.json through the wheel. Anything it refuses, or a document of another type, is RRS-T2002."""
    path = checkpoint_folder / RESUME_FILE
    try:
        result = json.loads(rastai_formats.read_document(RESUME_FILE, path.read_bytes()))
    except OSError as error:
        raise TrainError(RRS_T2002, f"{path} can't be read ({error.strerror}).") from None
    errors = [problem for problem in result["diagnostics"] if problem["severity"] == ERROR_SEVERITY]
    if errors or not result["document_json"]:
        reason = errors[0]["message"] if errors else "the file holds no document"
        raise TrainError(RRS_T2002, f"{path}: {reason}")
    document = json.loads(result["document_json"])
    if document["type"] != RESUME_TYPE:
        raise TrainError(RRS_T2002, f"{path} is a {document['type']} document, not {RESUME_TYPE}.")
    return resume_state_from_json(document)


def check_same_run(state: ResumeState, context: TrainingContext) -> None:
    """A checkpoint resumes only the run and the data it was written by (RRS-T2005)."""
    loader = state.dataloader
    if state.run_id != context.run_id:
        reason = f"it belongs to run {state.run_id}, not {context.run_id}"
    elif loader.dataset_hash != context.dataset_hash:
        reason = f"it was trained on dataset {loader.dataset_hash}, not {context.dataset_hash}"
    elif loader.split != context.split:
        reason = f"it read the {loader.split} split, not {context.split}"
    else:
        return
    raise TrainError(RRS_T2005, f"The checkpoint at step {state.step} can't resume this run: {reason}.")


def open_checkpoint_file(checkpoint_folder: Path, file_name: str) -> SafetensorsFile:
    """One safetensors file of the checkpoint, its header checked by the wheel. resume.json names the file, so a
    name that isn't a plain file name in this folder is refused before anything is opened."""
    if Path(file_name).name != file_name or file_name in ("", ".", ".."):
        raise TrainError(RRS_T2002, f"{checkpoint_folder / RESUME_FILE} names the file {file_name!r}.")
    try:
        return SafetensorsFile(checkpoint_folder / file_name)
    except OSError as error:
        raise TrainError(RRS_T2002, f"{checkpoint_folder / file_name} can't be read ({error.strerror}).") from None
    except ModelError as error:
        raise checkpoint_error(error, checkpoint_folder) from None


def read_all_tensors(checkpoint_folder: Path, file_name: str) -> dict[str, torch.Tensor]:
    """Every tensor of one checkpoint file, on the CPU."""
    tensor_file = open_checkpoint_file(checkpoint_folder, file_name)
    with tensor_file:
        tensors = {}
        for name in tensor_file.names():
            tensors[name] = tensor_file.load(name)
    return tensors


def checkpoint_error(error: ModelError, checkpoint_folder: Path) -> TrainError:
    """A refused weight file as the trainer's code: damaged bytes (the wheel's RRS-E codes) are RRS-T2002, a sound
    file that doesn't fit this design is RRS-T2005."""
    code = RRS_T2002 if error.code.startswith(FORMATS_CODE_PREFIX) else RRS_T2005
    return TrainError(code, f"{checkpoint_folder}: {error.message}")


def loader_at(context: TrainingContext, position: ResumeDataloader) -> BatchLoader:
    """The training loader at the checkpoint's epoch and position, over the same examples."""
    examples = context.loader.examples
    if position.position > len(examples):
        message = f"The checkpoint read {position.position} examples of an epoch; this split has {len(examples)}."
        raise TrainError(RRS_T2005, message)
    batch_size = context.settings.micro_batch
    return make_loader(examples, batch_size, position.shuffle_seed, position.epoch, position.position)


def set_progress(context: TrainingContext, state: ResumeState, point: ResumePoint) -> None:
    progress = context.progress
    progress.step = state.step
    progress.samples_seen = state.samples_seen
    progress.last_checkpoint_step = state.step
    progress.resume_point = point
