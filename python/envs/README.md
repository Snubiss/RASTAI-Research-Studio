# Locked Python environments

One uv project and one hash-pinned `uv.lock` per backend (ADR-0009; layout `docs/architecture/layout/python.md` §11). Nothing is resolved on a user's machine: the node installs an environment from its lock with `uv sync --frozen` into `<data>/envs/<environment_id>-<lock_hash12>/` (M2-16).

## Which backend gets which environment

| Environment | PyTorch build | Picked when | Status |
|---|---|---|---|
| `cpu` | CPU | no supported GPU; every CI runner; the M1 embedded node | M1 (this folder) |
| `cuda13` | CUDA 13.x | NVIDIA, driver 580 or newer (the default for NVIDIA) | Locked (M2-15); its CUDA runtime packages are NVIDIA's (below) |
| `cuda126` | CUDA 12.6 | NVIDIA Maxwell, Pascal and Volta | Locked (M2-15), the same NVIDIA packages |
| `rocm` | ROCm | supported AMD GPUs | Pending: the AMD license decision and the ROCm meta package (below); run from M10 |

`environments.json` holds the selection rules, the supported AMD gfx targets (RDNA3 and newer, from the device packages
of the ROCm 7.14 index) and the cold download sizes measured in spike 05. Each entry's `lock` is null while its lock is
pending.

## Pending lock: rocm (M2-15)

The rocm lock is not in the repository yet. Its pyproject and lock resolve (`uv lock` works), but two things need a decision,
not a change to this folder:

- **The AMD ROCm SDK licenses.** `rocm-sdk-core`, `rocm-sdk-libraries` and the `rocm-sdk-device-gfx*` packages come from the
  rocm7.14 index, which publishes no upload times and no license field in their METADATA (the core wheel bundles components
  under their own licenses, and the packages are not on PyPI). Spec §27 covers PyTorch's ROCm build, but names no license for
  these packages, and the owner approved none. `triton-rocm` is MIT. Until the owner names a license for the AMD packages,
  `check-deps` cannot pass a rocm lock, and `test_env_locks.py` requires a folder with a `pyproject.toml` to be listed in
  `environments.json` with its lock.
- **The ROCm meta package.** PyTorch's ROCm build requires `rocm[device-all]==7.14.*`, and the index serves that package only
  as an sdist (`rocm-7.14.1.tar.gz`). Its `setup.py` builds its own dependency list on the machine it is built on, so uv
  can't lock it with hashes. The rocm pyproject pins the device packages for the gfx targets `environments.json` lists
  directly and drops the sdist with `exclude-dependencies`, so the lock holds only wheels with hashes. The owner approved
  this approach on 2026-10-14.

## The cpu environment

- `cpu/pyproject.toml` pins torch (CPU build, from the PyTorch index), tokenizers, grpcio and protobuf exactly, with pytest, ruff, grpcio-tools (the generators `just codegen` runs) and maturin (which builds rastai-formats' PyO3 module) in the `dev` group. rastai-formats is built without build isolation (`no-build-isolation-package`), so it uses that locked maturin, and `just env-cpu` rebuilds it on every sync (`--reinstall-package rastai-formats`), so the module always matches the Rust sources. Workspace packages are installed editable from their folder through `[tool.uv.sources]`, with no version pin: `rastai-proto`, `rastai-data` and `rastai-plugin` so far. The packages later tasks need (pyarrow, torchvision, PyMySQL, onnx, onnxscript and the other workspace packages) join with those tasks.
- Platforms: the cpu lock has a `win_arm64` wheel for torch but none for torchvision 0.29.1, so a cpu sync on Windows on
  ARM fails. That platform is not one of ADR-0014's tiers, so it isn't supported until torchvision ships an arm64 wheel.
- `just env-cpu` syncs it into `.dev/envs/cpu` (git-ignored). Until M2-16 the embedded node uses that folder, and every `just check` runs pytest and ruff from it. After one online run it works offline from the uv cache.
- **numpy's 0BSD.** torchvision brings numpy, whose wheel declares `BSD-3-Clause AND 0BSD AND MIT AND Zlib AND CC0-1.0`.
  The allowlist's named entry for numpy names 0BSD (owner approval of 2026-10-14, spec §27), so `check-deps` passes this lock.
- `tests/test_cpu_environment.py` is the smoke test: Python 3.12, the CPU build of torch, the direct runtime packages (torch, tokenizers, grpcio and protobuf) importable, the installed versions equal to the pins, and the workspace packages installed.

## The cuda13 and cuda126 environments

- Both pin torch and torchvision from their own PyTorch index (`cu130` and `cu126`), as one build, with the same tokenizers,
  grpcio, protobuf and workspace packages as the cpu lock. torchvision 0.29.1 matches torch 2.14.1 on both.
- The CUDA runtime packages (`nvidia-*`, `cuda-*`) carry NVIDIA's proprietary license. The owner approved them on 2026-10-14 on
  one condition (spec §27, the NVIDIA license row): they are downloaded from PyPI by an explicit environment install
  (`system.env.install`), and never vendored, bundled or redistributed in Rastai's own packages. Each one has a named entry in
  `deps/allowlist-python.json` that cites that row. `nvidia-nvtx` is Apache-2.0 on PyPI and has no named entry.
- `cuda-toolkit` is left out of the graph (`exclude-dependencies`): it is a meta package with no code, and PyPI declares no
  license for it. The `nvidia-*` packages its extras brought are pinned in the pyproject instead, so the set of runtime packages
  is the same.
- `cuda-bindings` is 13.4.3 in the cuda13 lock and 12.9.9 in the cuda126 lock. One allowlist entry holds one version, so its
  `versions_by_lock` names the cuda126 lock's version (check-deps reads it).

## Updating a lock

1. Change the exact pin (`==`) in `<backend>/pyproject.toml`. A new package must be in spec §27 first.
2. Run `uv lock` in that folder. `exclude-newer` keeps out anything published in the last two weeks; move the date forward on purpose, in the same PR.
3. Update `deps/allowlist-python.json`: the new versions, and every transitive package the lock gained or lost.
4. Run `just env-cpu` and `just check`.

uv itself is pinned: `required-version` in each `pyproject.toml` refuses any uv but the one the justfile names.

## CI checks

- `just check` syncs the cpu environment and runs its tests, ruff format and ruff lint.
- From M1-05, `check-deps` fails a lock that holds a package or version missing from `deps/allowlist-python.json`.
- `python/envs/tests/test_env_locks.py` (in `just test-python`, offline) checks that each lock folder is listed in
  `environments.json`, that each lock's torchvision carries the same build tag as its torch and is the release
  `TORCHVISION_FOR_TORCH` names for it, that each proprietary NVIDIA package has its named entry, and that each
  `versions_by_lock` names a lock the allowlist reads.
- From M2, the `env-locks` job runs `just env-locks`: `uv lock --locked --refresh --directory <folder>` in every folder
  under `python/envs/`. It resolves each lock against its indexes and fails on a new package, a new version or a hash it
  didn't expect (§28 risk 11). A folder with a `pyproject.toml` and no `uv.lock` fails too. `--directory` is the form
  that passes on a clean lock with uv 0.13.0; `--project` from the repository root reports a stale lock that a fresh
  `uv lock` leaves unchanged. The job runs on every PR, since it is a required check (plan 8.4).
