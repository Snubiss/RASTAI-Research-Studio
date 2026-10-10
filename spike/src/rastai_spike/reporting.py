"""Hardware fingerprints and result files shared by the spike experiments (spec v5 §15: every run records its hardware)."""

import json
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path

import torch

RESULTS_FOLDER = Path(__file__).resolve().parents[2] / "results"
NVIDIA_QUERY = ["nvidia-smi", "--query-gpu=index,name,driver_version,memory.total", "--format=csv,noheader"]


def gpu_inventory() -> list[str]:
    try:
        return subprocess.run(NVIDIA_QUERY, capture_output=True, text=True, check=True).stdout.strip().splitlines()
    except (FileNotFoundError, subprocess.CalledProcessError):
        return []


def environment_fingerprint() -> dict:
    return {
        "recorded_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "python": platform.python_version(),
        "torch": torch.__version__,
        "cuda_runtime": torch.version.cuda,
        "gpus": gpu_inventory(),
        "kernel": platform.release(),
    }


def device_label(device: str) -> str:
    properties = torch.cuda.get_device_properties(device)
    return f"{device} {properties.name} (sm_{properties.major}{properties.minor})"


def write_results(experiment_name: str, data: dict, markdown: str) -> tuple[Path, Path]:
    RESULTS_FOLDER.mkdir(exist_ok=True)
    json_path = RESULTS_FOLDER / f"{experiment_name}.json"
    markdown_path = RESULTS_FOLDER / f"{experiment_name}.md"
    json_path.write_text(json.dumps(data, indent=2) + "\n")
    markdown_path.write_text(markdown)
    return json_path, markdown_path
