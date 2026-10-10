# ADR-0009: uv, Python 3.12, one lockfile per backend, no runtime installs

**Status:** Accepted, 2026-10-10

## Context

- PyTorch ships a different wheel per backend (CUDA 13, legacy CUDA 12.6, ROCm, CPU). They can't live in one environment.
- Installing packages while the app runs is a supply-chain hole (LiteLLM was compromised on PyPI in March 2026) and a reproducibility hole.
- Runs must be reproducible. The run record holds an environment lock hash (§26).
- Spike 05, uv 0.13, torch 2.14.1:

  | Backend | Cold install | Download | Warm reinstall | First `import torch` |
  |---|---|---|---|---|
  | CUDA | 33 s | 5.2 GB | under 1 s | 1.4 s |
  | ROCm | 91 s | 16.5 GB | under 1 s | 2.1 s |
  | CPU | 3 s | 0.75 GB | under 1 s | 1.3 s |

## Decision

1. **Python 3.12 only.** uv creates environments and resolves lockfiles. It is a §27 foundation.
2. **One uv project and one `uv.lock` per backend** under `python/envs/`: `cpu/`, `cuda13/`, `cuda126/`, `rocm/` (`mps/` later). Environment ids match the folder names.
3. **The node installs environments** at `<data>/envs/<environment_id>-<lock_hash12>/` through `NodeService.EnsureEnvironment`, which streams `EnvironmentProgress` (command `system.env.install`).
   - A changed lockfile means a new folder. The old one is garbage-collected when no job uses it.
   - Installs come only from the hash-pinned lockfile (`uv sync --frozen` semantics).
4. **The backend is picked by hardware detection** (ADR-0014):
   - `cuda13` by default (needs driver 580+);
   - `cuda126` for Maxwell, Pascal and Volta;
   - `rocm` for supported AMD GPUs (one `rocm` environment while there is one card; per-architecture ids only if the wheels differ, contracts §14 item 10);
   - `cpu` as the fallback.
5. **No installs at runtime.** No running process calls `pip`, `uv pip` or `uv add`. CI greps for them. Only `EnsureEnvironment` and plugin install (below) run uv, and only from a lockfile.
6. **Plugins** bring a per-plugin uv lockfile with hashes. At install time (never later) it is synced into `<data>/plugins/<plugin_id>/<version>/env/` and layered on the base environment with a `.pth` file. A plugin that conflicts with the base lock fails install with `RRS-P3xxx`.
7. **The installer shows the download size and an estimated time** per backend before it starts.
8. **Every run record** carries `environment_lock_hash`, and every `HardwareFingerprint` carries `environment_hash`.

**Checked by:** CI resolves every backend lockfile on every PR; a grep check fails on `pip install` or `uv pip` in shipped code; a test that a job refuses to start in an environment whose lock hash doesn't match its `JobSpec.environment_id` folder.

## Consequences

**Good**
- Fast, cacheable, hash-verified environments. A warm reinstall takes under a second.
- Reproducible runs: the lock hash is in the run record.
- No code reaches the machine after install time without a lockfile change.

**Costs**
- ROCm is heavy: 16.5 GB and 91 s cold on the owner's connection.
- Four lockfiles to keep resolving. Version pins can conflict across backends (§28 risk 11). Fewer dependencies is the main mitigation.
- Plugins can only use packages compatible with the base lock.
- Users can't `pip install` something into a job to try it. That's deliberate, and a plugin with its own lockfile is the path.

## Alternatives considered

| Alternative | Why not |
|---|---|
| conda / mamba | Slower, heavier, a second package index, and not on the allowlist. |
| pip + venv | No hash-locked resolution at uv's speed. |
| One environment for all backends | Impossible: the torch wheels conflict. |
| System Python | Version drift across distros, and it breaks the pin. |
| Docker images for jobs | GPU passthrough setup, very large ROCm images, slow cold start, and Docker becomes a hard requirement. |

## Links

- Spec: [§13](../spec/rastai_spec_v5.md#13-plugins-and-ecosystem) (dependencies), [§22](../spec/rastai_spec_v5.md#22-compute-nodes-and-data), [§26](../spec/rastai_spec_v5.md#26-code-and-reproducibility), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§1 repository tree](../architecture/contracts.md#1-repository-tree) (`python/envs/`), [§8.1 paths](../architecture/contracts.md#8-storage)
- Spike: [05 environment install timing](../../spike/results/05_env_install_timing.md)
- Related: [ADR-0012](0012-build-in-house-policy.md), [ADR-0014](0014-platform-tiers.md), [ADR-0015](0015-plugins-and-licensing.md)
