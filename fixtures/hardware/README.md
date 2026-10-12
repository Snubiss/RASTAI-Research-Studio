# Hardware fixtures

What real machines showed `rastai-node`'s hardware detection and telemetry (node layout 7.7 and 7.9; flows/platform.md 1.2). Each case is one machine. `expected.json` holds the `HardwareProfile` that detection must build from it, in protobuf field names, plus the codes of the notes it must give (`RRS-H0001`, `RRS-H0002`).

| Case | Machine | Expected |
|---|---|---|
| `rtx5080_rtx3090_gfx1036/` | the owner's box: RTX 5080 and RTX 3090 (driver 595.91.07), and the Ryzen 9 7900X's gfx1036 iGPU, without ROCm installed | three GPUs: NVIDIA 0 (`sm_120`) and 1 (`sm_86`), AMD 0 (`gfx1036`, 512 MiB); no notes |
| `rtx5080/` | the 5080 alone | one GPU, `sm_120` |
| `rtx3090/` | the 3090 alone | one GPU, `sm_86` |
| `gfx1036_igpu/` | the iGPU alone, without ROCm | one AMD GPU, `gfx1036`, named by its PCI ids, because no rocm-smi is there to name it; no notes |
| `no_gpu/` | no display device | no GPU, backend `cpu`, `RRS-H0001` |

Choosing the environment from the profile is `environments.rs` (M2-16), so `backend` is empty whenever a GPU is listed. That is also where `gfx1036` becomes "listed but not supported" (`RRS-H4004`). Free disk is 0 here: detection doesn't know the node's data folder, and the node fills it in from `rastai_store::free_bytes`.

## What each case holds

| Path | Recorded from | Becomes, in a test |
|---|---|---|
| `proc/cpuinfo`, `proc/meminfo` | `/proc` | `<test folder>/proc/` |
| `pci/<address>/class`, `vendor`, `device`, `mem_info_vram_total` (AMD only) | `/sys/bus/pci/devices/<address>/` of each display device | `<test folder>/sys/bus/pci/devices/<address>/` |
| `kfd/<node>/properties` | `/sys/class/kfd/kfd/topology/nodes/<node>/properties` | `<test folder>/sys/class/kfd/kfd/topology/nodes/<node>/` |
| `tools/nvidia-smi-gpus.csv` | `nvidia-smi --query-gpu=index,pci.bus_id,name,driver_version,memory.total,compute_cap --format=csv,noheader,nounits` | the stand-in `nvidia-smi`'s answer to that query |
| `tools/nvidia-smi-telemetry-stream.csv` (full box only) | 11 seconds of `nvidia-smi --query-gpu=index,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw --format=csv,noheader,nounits -l 5` | the stand-in's telemetry stream |
| `tools/nvidia-smi-compute-apps.csv` (full box only) | `nvidia-smi --query-compute-apps=pid,gpu_bus_id,used_memory --format=csv,noheader,nounits` | the stand-in's per-process answer |

PCI addresses are stored with `-` for `:` (`0000-01-00.0`), because Windows can't check out a file name with `:`. `crates/rastai-node/tests/common/mod.rs` lays each case out as `/sys` and `/proc` with the `:` back, and writes a stand-in `nvidia-smi` script that prints these files. A case without `tools/nvidia-smi-gpus.csv` has no `nvidia-smi`. No case has `rocm-smi`: no box with ROCm installed was available. The rocm-smi parsers' unit tests use JSON written from rocm-smi's documented output and say so. Replace them with a recording when M10's AMD runner exists.

## Which files are cut from a recording

The full box is recorded as it was (2026-10-11). The four single-device cases are cut from that recording:

- `proc/cpuinfo` keeps the first two processor blocks, so the files stay small; their expected `cpu_count` is 2.
- `pci/` keeps only that case's device.
- `rtx5080` keeps nvidia-smi's first line as it is.
- `rtx3090` keeps the second line with its index changed from 1 to 0, which is what nvidia-smi prints for a box with one GPU. Its bus id `0D:00.0` is kept.
- `gfx1036_igpu` keeps both kfd nodes (the CPU node is skipped by detection).

## Who reads them

| Test | Checks |
|---|---|
| `crates/rastai-node/tests/hardware.rs` | each case gives its `expected.json`; a hung `nvidia-smi` times out with `RRS-H0002` and a partial profile, and doesn't block the node's start |
| `crates/rastai-node/tests/handshake.rs` | GetHardware returns the full box's profile and the stream's last samples |
| unit tests in `hardware.rs` and `telemetry.rs` | the parsers on the recorded lines |
