"""
Experiment 05: how long does the installer's Python environment take? (spec v5 §32, spike item 5)

The installer (spec §6, "0–3 min") creates a pinned Python environment per GPU backend with uv. For each
backend this measures, from nothing:
  - cold install: empty uv cache, so everything is downloaded (a first-time user);
  - warm install: a second environment from the same cache (a reinstall or a second project);
  - first `import torch` in the new environment, and whether it sees the GPU.

ROCm installs and imports here, and even reports a GPU: the Ryzen's integrated graphics (gfx1036). But
PyTorch's ROCm build has no kernels for that chip, and forcing it (HSA_OVERRIDE_GFX_VERSION=10.3.0) gives
silently wrong matmul results, so it can't stand in for a real AMD card. The install time is still the real
number a ROCm user would pay.

Everything is built under spike/.scratch/ (on disk; /tmp is RAM-backed here) and deleted afterwards.

Run:  uv run python experiments/05_env_install_timing.py
"""

import os
import shutil
import subprocess
import time
from pathlib import Path

from rastai_spike.reporting import environment_fingerprint, write_results

EXPERIMENT_NAME = "05_env_install_timing"
SPIKE_FOLDER = Path(__file__).resolve().parents[1]
SCRATCH_FOLDER = SPIKE_FOLDER / ".scratch" / EXPERIMENT_NAME
UV = Path.home() / ".local" / "bin" / "uv"
PYTHON_VERSION = "3.12"
PACKAGES = ["torch==2.14.1", "tokenizers==0.23.3"]
PYPI = "https://pypi.org/simple"
BACKEND_INDEXES = {  # None = PyPI only (its torch wheels are CUDA 13.0 builds)
    "cuda": None,
    "rocm": "https://download.pytorch.org/whl/rocm7.14",
    "cpu": "https://download.pytorch.org/whl/cpu",
}
IMPORT_CHECK = (
    "import time; start = time.perf_counter(); import torch; seconds = time.perf_counter() - start; "
    "print(torch.__version__, torch.version.cuda, torch.version.hip, torch.cuda.is_available(), f'{seconds:.2f}')"
)
GIB = 1024**3


def run_timed(command: list[str], environment: dict | None = None) -> tuple[float, str]:
    start = time.perf_counter()
    completed = subprocess.run(command, env=environment, capture_output=True, text=True, check=True)
    return time.perf_counter() - start, completed.stdout


def folder_size(folder: Path) -> int:
    # Count each file once: uv hard-links from its cache, so a linked file is not extra disk use.
    seen, total = set(), 0
    for path in folder.rglob("*"):
        if path.is_file() and not path.is_symlink():
            identity = (path.stat().st_dev, path.stat().st_ino)
            if identity not in seen:
                seen.add(identity)
                total += path.stat().st_size
    return total


def install_environment(backend: str, environment_folder: Path, cache_folder: Path) -> float:
    """Create a venv and install the pinned packages. Returns seconds."""
    uv_environment = dict(os.environ, UV_CACHE_DIR=str(cache_folder))  # everything else as normal
    subprocess.run([str(UV), "venv", "--quiet", "--python", PYTHON_VERSION, str(environment_folder)], env=uv_environment, check=True)

    command = [str(UV), "pip", "install", "--quiet", "--python", str(environment_folder / "bin" / "python"), *PACKAGES]
    index = BACKEND_INDEXES[backend]
    if index is not None:
        # uv gives --extra-index-url priority, so torch comes from the PyTorch index and the rest from PyPI.
        command += ["--index-url", PYPI, "--extra-index-url", index]
    seconds, _ = run_timed(command, uv_environment)
    return seconds


def measure_backend(backend: str) -> dict:
    print(f"[{EXPERIMENT_NAME}] {backend}", flush=True)
    backend_folder = SCRATCH_FOLDER / backend
    shutil.rmtree(backend_folder, ignore_errors=True)
    cache_folder = backend_folder / "uv-cache"

    cold_seconds = install_environment(backend, backend_folder / "cold-env", cache_folder)
    download_bytes = folder_size(cache_folder)
    warm_seconds = install_environment(backend, backend_folder / "warm-env", cache_folder)

    _, import_output = run_timed([str(backend_folder / "cold-env" / "bin" / "python"), "-c", IMPORT_CHECK])
    version, cuda_version, hip_version, gpu_visible, import_seconds = import_output.split()
    environment_bytes = folder_size(backend_folder / "cold-env")

    shutil.rmtree(backend_folder)
    return {
        "cold_install_seconds": cold_seconds,
        "warm_install_seconds": warm_seconds,
        "download_gib": download_bytes / GIB,
        "environment_gib": environment_bytes / GIB,
        "torch": version,
        "cuda": cuda_version,
        "hip": hip_version,
        "gpu_visible": gpu_visible == "True",
        "first_import_seconds": float(import_seconds),
    }


def build_markdown(results: dict) -> str:
    lines = [
        "# Experiment 05: Python environment install time per backend",
        "",
        f"uv {results['uv_version']}, Python {PYTHON_VERSION}, packages {PACKAGES}.",
        "",
        "| Backend | Cold install (empty cache) | Warm install | Downloaded | Environment on disk | torch build | GPU visible | First `import torch` |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for backend, row in results["backends"].items():
        build = row["torch"] + (f" (CUDA {row['cuda']})" if row["cuda"] != "None" else "") + (f" (HIP {row['hip']})" if row["hip"] != "None" else "")
        lines.append(
            f"| {backend} | {row['cold_install_seconds']:.0f} s | {row['warm_install_seconds']:.1f} s | {row['download_gib']:.2f} GiB | "
            f"{row['environment_gib']:.2f} GiB | {build} | {row['gpu_visible']} | {row['first_import_seconds']:.1f} s |"
        )
    lines += [
        "",
        "Cold-install time depends on this connection's bandwidth; the download size is the portable number.",
        "ROCm 'sees' the Ryzen's integrated graphics (gfx1036), but PyTorch's ROCm build has no kernels for it: computing fails, "
        "and forcing it with HSA_OVERRIDE_GFX_VERSION=10.3.0 returns silently wrong matmuls (up to 100% error at some sizes). "
        "A discrete RDNA3+ card is needed for real AMD testing.",
        "",
        "## Environment", "", "```", *[f"{key}: {value}" for key, value in results["environment"].items()], "```", "",
    ]
    return "\n".join(lines)


def main() -> None:
    SCRATCH_FOLDER.mkdir(parents=True, exist_ok=True)
    _, uv_version = run_timed([str(UV), "--version"])
    results = {
        "uv_version": uv_version.strip(),
        "backends": {backend: measure_backend(backend) for backend in BACKEND_INDEXES},
        "environment": environment_fingerprint(),
    }
    shutil.rmtree(SCRATCH_FOLDER, ignore_errors=True)

    markdown = build_markdown(results)
    json_path, markdown_path = write_results(EXPERIMENT_NAME, results, markdown)
    print(markdown)
    print(f"Wrote {json_path} and {markdown_path}")


if __name__ == "__main__":
    main()
