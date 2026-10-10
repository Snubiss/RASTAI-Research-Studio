"""
Find model files in the user's existing Hugging Face cache (spec v5 §19: weights stay there, never copied).

Cache layout:
    <cache>/models--<owner>--<name>/refs/main          -> the commit hash of the downloaded revision
    <cache>/models--<owner>--<name>/snapshots/<hash>/  -> the files, as symlinks into blobs/
"""

import os
from pathlib import Path

DEFAULT_CACHE_DIRECTORY = Path.home() / ".cache" / "huggingface" / "hub"
DEFAULT_REVISION = "main"


def cache_directory() -> Path:
    # Same environment variables the HF tools honour, so we find what they downloaded.
    if "HF_HUB_CACHE" in os.environ:
        return Path(os.environ["HF_HUB_CACHE"])
    if "HF_HOME" in os.environ:
        return Path(os.environ["HF_HOME"]) / "hub"
    return DEFAULT_CACHE_DIRECTORY


def snapshot_path(repository_id: str, revision: str = DEFAULT_REVISION) -> Path:
    """Return the local folder holding a downloaded model, or raise if it isn't downloaded."""
    owner, name = repository_id.split("/")
    model_folder = cache_directory() / f"models--{owner}--{name}"

    reference_file = model_folder / "refs" / revision
    commit_hash = reference_file.read_text().strip() if reference_file.exists() else revision

    snapshot = model_folder / "snapshots" / commit_hash
    if not snapshot.is_dir():
        raise FileNotFoundError(f"{repository_id}@{revision} is not in the HF cache at {model_folder}")
    return snapshot
