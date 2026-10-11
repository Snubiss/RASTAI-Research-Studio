# ADR-0014: Linux NVIDIA, AMD and CPU at tier 1; macOS and Windows at tier 2 with portability rules

**Status:** Accepted, 2026-10-10

## Context

- The owner has Linux with an RTX 5080 (Blackwell, sm_120) and an RTX 3090. There's no Mac and no discrete AMD GPU on hand.
- All three operating systems are the long-term goal (§0.4). Unsupported platforms rot unless the code is kept portable from day one (§28 risk 13).
- PyTorch's ROCm build uses the same `torch.cuda` API, and llama.cpp ships ROCm builds, so AMD is cheap to support in code.
- Spike 05: ROCm detected the Ryzen's integrated GPU (gfx1036) but had no kernels for it. Forcing it with `HSA_OVERRIDE_GFX_VERSION=10.3.0` returned **silently wrong matmuls**: up to 100% error at some sizes, different from run to run.

## Decision

1. **Tiers:**

   | Tier | Platform | Status |
   |---|---|---|
   | 1 | Linux + NVIDIA (CUDA 13) | built and tested in the prototype |
   | 1 | Linux + AMD (ROCm, discrete RDNA3 or newer) | built now; tested once the card is in |
   | 1 | Linux CPU-only | built and tested (CI, small models) |
   | 2 | macOS Apple Silicon (MPS) | designed for; untested until a Mac is available |
   | 2 | Windows (native or WSL2) | designed for; Phase 2+ |

2. **Portability rules, each with one owner:**
   - The UI is the web app; Tauri is only a shell (ADR-0006).
   - Local IPC goes only through `rastai-transport`: Unix sockets now, named pipes (`\\.\pipe\rastai-<user>-<name>`) on Windows.
   - Paths are built only in `rastai-core/src/paths.rs` (XDG on Linux; the macOS and Windows mappings are in contracts §8.1). `RASTAI_HOME` overrides everything.
   - Sandboxes go through any Docker-API engine (ADR-0031).
   - Every compute backend is chosen by hardware detection. No code hard-codes `cuda`. Code outside detection and environment selection never branches on the GPU vendor.
3. **CI from day one:** CPU tests on Linux, plus CPU tests on macOS and Windows runners. GPU tests run nightly on the owner's box (self-hosted runner), or by hand until it's set up. *(Note since acceptance, 2026-10-10: the runner is `nightly-gpu.yml`, scheduled and manual triggers on main only, run as a separate OS user `rastai-runner`, labels `gpu-nvidia` and, from M10, `gpu-amd`; a read-only HF token for gated weights once the repo is public (owner decision 12).)*
4. **`rastai doctor` checks numbers, not just kernels.** Its 20 s self-test runs matmuls at several sizes and a tiny forward pass, compares them with a CPU reference, and fails the GPU with `RRS-H1xxx` if they disagree beyond tolerance. A failed GPU is not offered as a compute target.
5. **Rastai never sets `HSA_OVERRIDE_GFX_VERSION`** or any similar override. Unsupported GPUs are reported plainly, with CPU and SSH pairing offered instead.
6. **AMD tier-1 testing waits for a discrete RDNA3+ card.** Its model decides the ROCm target architecture and whether 4-bit kernels are available. Until then QLoRA on ROCm is marked unavailable in the capability matrix. *(Note since acceptance, 2026-10-10: the owner buys the card before M10, which holds the AMD parity gate; M2 closes on NVIDIA alone (owner decision 11).)*
7. **Blackwell gaps** in external kernels are expected (§28 risk 7). The 3090 (sm_86) is the fallback test GPU.
8. **Agent-gym training runs on Linux only** for now (§0.6).

**Checked by:** macOS and Windows CPU jobs in CI on every PR; a check that no Rust file outside `paths.rs` reads `HOME`, `XDG_*` or `RASTAI_HOME` or contains a `~/` or `/home/` literal (in Python, only the standalone SDK's HF-cache module may; jobs get resolved paths from the node); a `doctor` test with a deliberately corrupted matmul (a fake backend) that must fail the self-test.

## Consequences

**Good**
- The prototype targets hardware the owner can test, without painting tier 2 into a corner.
- Silent numerical errors can't pass as a working GPU.
- AMD support is designed in, so adding the card is testing work, not porting work.

**Costs**
- CPU CI on three operating systems costs CI time from day one.
- Tier-2 code paths (named pipes, macOS paths) exist untested on real hardware until the hardware exists.
- The 20 s self-test adds time to first setup.

## Alternatives considered

| Alternative | Why not |
|---|---|
| All three operating systems at tier 1 now | No Mac on hand, and triple the test burden before anything works. |
| NVIDIA only | Excludes AMD users when the code cost is small. |
| A "kernels run" self-test only | The spike's override case runs and returns garbage. |
| Allow ROCm overrides behind a flag | The failure is silent and varies run to run. A flag would hide that. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) items 4, 6 and 16, [§4](../spec/rastai_spec_v5.md#4-audience-and-platforms), [§22](../spec/rastai_spec_v5.md#22-compute-nodes-and-data), [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup)
- Contracts: [§8.1 data directories](../architecture/contracts.md#8-storage), [§10 RRS-H codes](../architecture/contracts.md#10-error-catalog)
- Spike: [05 environment install timing](../../spike/results/05_env_install_timing.md)
- Related: [ADR-0006](0006-ui-transport-http-only.md), [ADR-0009](0009-python-environments-uv.md), [ADR-0031](0031-sandbox-manager.md)
