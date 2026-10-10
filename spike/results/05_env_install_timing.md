# Experiment 05: Python environment install time per backend

uv uv 0.13.0 (x86_64-unknown-linux-gnu), Python 3.12, packages ['torch==2.14.1', 'tokenizers==0.23.3'].

| Backend | Cold install (empty cache) | Warm install | Downloaded | Environment on disk | torch build | GPU visible | First `import torch` |
|---|---|---|---|---|---|---|---|
| cuda | 33 s | 0.2 s | 5.15 GiB | 5.17 GiB | 2.14.1+cu130 (CUDA 13.0) | True | 1.4 s |
| rocm | 91 s | 0.8 s | 16.47 GiB | 16.48 GiB | 2.14.1+rocm7.14 (HIP 7.14.60850) | True | 2.1 s |
| cpu | 3 s | 0.6 s | 0.74 GiB | 0.75 GiB | 2.14.1+cpu | False | 1.3 s |

Cold-install time depends on this connection's bandwidth; the download size is the portable number.
ROCm 'sees' the Ryzen's integrated graphics (gfx1036), but PyTorch's ROCm build has no kernels for it: computing fails, and forcing it with HSA_OVERRIDE_GFX_VERSION=10.3.0 returns silently wrong matmuls (up to 100% error at some sizes). A discrete RDNA3+ card is needed for real AMD testing.

## Environment

```
recorded_at: 2026-10-10T08:32:36+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
