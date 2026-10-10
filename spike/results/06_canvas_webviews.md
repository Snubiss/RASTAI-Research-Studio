# Experiment 06: in-house WebGL2 canvas in Chrome and WebKitGTK

Window 1600×1000, 3000 ms per configuration after a warm-up. Target: 1,000 visible nodes at 60 fps. Frame rates are capped by the display's refresh rate.

## chrome

- ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 5080/PCIe/SSE2, OpenGL ES 3.2) (Google Inc. (NVIDIA Corporation)); WebGL 2.0 (OpenGL ES 3.0 Chromium)
- Refresh 145 Hz, canvas 1600×944, device pixel ratio 1
- **Target met: True**; largest graph still at 60 fps: 50000 nodes
- Headroom: the combined scene (1,000 nodes + 26×512 flow) still runs at the refresh cap when drawn **16×** per frame (1×: 145 fps, 2×: 145 fps, 4×: 145 fps, 8×: 145 fps, 16×: 145 fps, 32×: 75 fps)

| Scene | Instances | fps | Frame ms p50 / p95 / p99 | Script ms p50 / p95 | Pixels drawn |
|---|---|---|---|---|---|
| graph 1000 nodes | 1,000 nodes, 1,167 edges, 10,515 glyphs | 144 | 6.9 / 7.0 / 7.1 | 0.00 / 0.10 | 28.5% |
| graph 2000 nodes | 2,000 nodes, 2,342 edges, 22,140 glyphs | 144 | 6.9 / 7.0 / 7.2 | 0.10 / 0.10 | 20.9% |
| graph 5000 nodes | 5,000 nodes, 5,867 edges, 57,015 glyphs | 144 | 6.9 / 7.0 / 7.2 | 0.10 / 0.20 | 8.6% |
| graph 10000 nodes | 10,000 nodes, 11,742 edges, 115,140 glyphs | 144 | 6.9 / 7.0 / 7.1 | 0.20 / 0.30 | 6.6% |
| graph 20000 nodes | 20,000 nodes, 23,492 edges, 241,390 glyphs | 144 | 6.9 / 7.0 / 7.1 | 0.40 / 0.50 | 2.8% |
| graph 50000 nodes | 50,000 nodes, 58,742 edges, 620,140 glyphs | 144 | 6.9 / 7.0 / 7.1 | 0.90 / 1.10 | 1.6% |
| flow 26×512 | 13,312 cubes | 144 | 6.9 / 7.0 / 7.1 | 0.40 / 0.50 | 6.4% |
| flow 26×2048 | 53,248 cubes | 144 | 6.9 / 7.0 / 7.1 | 1.60 / 1.90 | 1.9% |
| flow 26×8192 | 212,992 cubes | 144 | 6.9 / 7.0 / 7.1 | 6.50 / 7.00 | 0.5% |
| combined 1000 nodes + flow 26×512 | 1,000 nodes, 1,167 edges, 10,515 glyphs, 13,312 cubes | 144 | 6.9 / 7.0 / 7.1 | 0.50 / 0.60 | 12.4% |

## webkitgtk

- Apple GPU (Apple Inc.); WebGL 2.0
- Refresh 62 Hz, canvas 1600×1000, device pixel ratio 1
- **Target met: True**; largest graph still at 60 fps: 50000 nodes
- Headroom: the combined scene (1,000 nodes + 26×512 flow) still runs at the refresh cap when drawn **32×** per frame (1×: 63 fps, 2×: 63 fps, 4×: 63 fps, 8×: 63 fps, 16×: 63 fps, 32×: 63 fps, 64×: 45 fps)

| Scene | Instances | fps | Frame ms p50 / p95 / p99 | Script ms p50 / p95 | Pixels drawn |
|---|---|---|---|---|---|
| graph 1000 nodes | 1,000 nodes, 1,167 edges, 10,515 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 27.3% |
| graph 2000 nodes | 2,000 nodes, 2,342 edges, 22,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 19.6% |
| graph 5000 nodes | 5,000 nodes, 5,867 edges, 57,015 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 8.1% |
| graph 10000 nodes | 10,000 nodes, 11,742 edges, 115,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 6.3% |
| graph 20000 nodes | 20,000 nodes, 23,492 edges, 241,390 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 2.6% |
| graph 50000 nodes | 50,000 nodes, 58,742 edges, 620,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 1.00 / 1.00 | 1.5% |
| flow 26×512 | 13,312 cubes | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 6.0% |
| flow 26×2048 | 53,248 cubes | 63 | 16.0 / 17.0 / 17.0 | 1.00 / 2.00 | 1.8% |
| flow 26×8192 | 212,992 cubes | 63 | 16.0 / 17.0 / 17.0 | 3.00 / 4.00 | 0.5% |
| combined 1000 nodes + flow 26×512 | 1,000 nodes, 1,167 edges, 10,515 glyphs, 13,312 cubes | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 11.7% |

## webkitgtk, DMABUF renderer off

- Apple GPU (Apple Inc.); WebGL 2.0
- Refresh 62 Hz, canvas 1600×1000, device pixel ratio 1
- **Target met: True**; largest graph still at 60 fps: 50000 nodes
- Headroom: the combined scene (1,000 nodes + 26×512 flow) still runs at the refresh cap when drawn **32×** per frame (1×: 63 fps, 2×: 63 fps, 4×: 63 fps, 8×: 63 fps, 16×: 63 fps, 32×: 63 fps, 64×: 45 fps)

| Scene | Instances | fps | Frame ms p50 / p95 / p99 | Script ms p50 / p95 | Pixels drawn |
|---|---|---|---|---|---|
| graph 1000 nodes | 1,000 nodes, 1,167 edges, 10,515 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 27.3% |
| graph 2000 nodes | 2,000 nodes, 2,342 edges, 22,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 19.7% |
| graph 5000 nodes | 5,000 nodes, 5,867 edges, 57,015 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 8.1% |
| graph 10000 nodes | 10,000 nodes, 11,742 edges, 115,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 6.3% |
| graph 20000 nodes | 20,000 nodes, 23,492 edges, 241,390 glyphs | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 2.6% |
| graph 50000 nodes | 50,000 nodes, 58,742 edges, 620,140 glyphs | 63 | 16.0 / 17.0 / 17.0 | 1.00 / 1.00 | 1.5% |
| flow 26×512 | 13,312 cubes | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 6.0% |
| flow 26×2048 | 53,248 cubes | 63 | 16.0 / 17.0 / 17.0 | 1.00 / 1.00 | 1.7% |
| flow 26×8192 | 212,992 cubes | 63 | 16.0 / 17.0 / 17.0 | 3.00 / 4.00 | 0.5% |
| combined 1000 nodes + flow 26×512 | 1,000 nodes, 1,167 edges, 10,515 glyphs, 13,312 cubes | 63 | 16.0 / 17.0 / 17.0 | 0.00 / 1.00 | 11.7% |

## Environment

```
recorded_at: 2026-10-10T09:55:22+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
