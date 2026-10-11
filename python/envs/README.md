# Locked Python environments

One uv project and one hash-pinned `uv.lock` per backend (ADR-0009; layout `docs/architecture/layout/python.md` §11). Nothing is resolved on a user's machine: the node installs an environment from its lock with `uv sync --frozen` into `<data>/envs/<environment_id>-<lock_hash12>/` (M2-16).

## Which backend gets which environment

| Environment | PyTorch build | Picked when | Status |
|---|---|---|---|
| `cpu` | CPU | no supported GPU; every CI runner; the M1 embedded node | M1 (this folder) |
| `cuda13` | CUDA 13.x | NVIDIA, driver 580 or newer (the default for NVIDIA) | M2 |
| `cuda126` | CUDA 12.6 | NVIDIA Maxwell, Pascal and Volta | M2 |
| `rocm` | ROCm | supported AMD GPUs | M2 (resolved in CI, run from M10) |

`environments.json` (M2) holds the selection rules and download sizes.

## The cpu environment

- `cpu/pyproject.toml` pins torch (CPU build, from the PyTorch index), tokenizers, grpcio and protobuf exactly, with pytest, ruff and grpcio-tools (the generators `just codegen` runs) in the `dev` group. Workspace packages are installed editable from their folder through `[tool.uv.sources]`, with no version pin: `rastai-proto`, `rastai-data` and `rastai-plugin` so far. The packages later tasks need (pyarrow, torchvision, PyMySQL, onnx, onnxscript and the other workspace packages) join with those tasks.
- `just env-cpu` syncs it into `.dev/envs/cpu` (git-ignored). Until M2-16 the embedded node uses that folder, and every `just check` runs pytest and ruff from it. After one online run it works offline from the uv cache.
- `tests/test_cpu_environment.py` is the smoke test: Python 3.12, the CPU build of torch, the direct runtime packages (torch, tokenizers, grpcio and protobuf) importable, the installed versions equal to the pins, and the workspace packages installed.

## Updating a lock

1. Change the exact pin (`==`) in `<backend>/pyproject.toml`. A new package must be in spec §27 first.
2. Run `uv lock` in that folder. `exclude-newer` keeps out anything published in the last two weeks; move the date forward on purpose, in the same PR.
3. Update `deps/allowlist-python.json`: the new versions, and every transitive package the lock gained or lost.
4. Run `just env-cpu` and `just check`.

uv itself is pinned: `required-version` in each `pyproject.toml` refuses any uv but the one the justfile names.

## CI checks

- `just check` syncs the cpu environment and runs its tests, ruff format and ruff lint.
- From M1-05, `check-deps` fails a lock that holds a package or version missing from `deps/allowlist-python.json`.
- From M2, the `env-locks` job resolves every lock on every change and fails on a hash change it didn't expect (§28 risk 11).
