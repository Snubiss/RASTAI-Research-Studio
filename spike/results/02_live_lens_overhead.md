# Experiment 02: live Psychology panel overhead

Gemma 3 1B-it in bf16, greedy decoding of 256 tokens after 8 warm-up tokens.
Panel per token: entropy + top-10, logit lens on all layers, top attended positions per head and layer, top-16 Gemma Scope 2 SAE features on layers [7, 13, 22] (16k wide, fp32).

## cuda:0 NVIDIA GeForce RTX 5080 (sm_120) (capped at 8 GiB)

| Context / execution / panel | tokens/s | ms/token | vs panel off | working VRAM GiB | peak VRAM GiB | packet bytes | tokens matching baseline |
|---|---|---|---|---|---|---|---|
| chat / eager / off | 67.5 | 14.82 | — | 0.01 | 2.37 | 4 | 64/64 |
| chat / eager / naive | 21.3 | 46.94 | +217% | 0.01 | 2.38 | 4 | 64/64 |
| chat / eager / batched | 49.1 | 20.37 | +37% | 0.10 | 2.46 | 8284 | 64/64 |
| chat / eager / fused | 52.9 | 18.90 | +28% | 0.10 | 2.46 | 8284 | 64/64 |
| chat / cuda graph / off | 155.0 | 6.45 | — | 0.07 | 2.44 | 4 | 64/64 |
| chat / cuda graph / batched | 114.3 | 8.75 | +36% | 0.16 | 2.56 | 8284 | 64/64 |
| chat / cuda graph / fused | 124.6 | 8.02 | +24% | 0.16 | 2.59 | 8284 | 64/64 |
| long / eager / off | 68.0 | 14.70 | — | 0.14 | 2.60 | 4 | 64/64 |
| long / eager / naive | 21.3 | 46.84 | +219% | 0.14 | 2.60 | 4 | 64/64 |
| long / eager / batched | 52.8 | 18.92 | +29% | 0.14 | 2.60 | 8284 | 64/64 |
| long / eager / fused | 56.2 | 17.80 | +21% | 0.14 | 2.60 | 8284 | 64/64 |
| long / cuda graph / off | 110.5 | 9.05 | — | 0.14 | 2.60 | 4 | 64/64 |
| long / cuda graph / batched | 88.0 | 11.37 | +26% | 0.20 | 2.69 | 8284 | 64/64 |
| long / cuda graph / fused | 95.0 | 10.53 | +16% | 0.20 | 2.72 | 8284 | 64/64 |

SAE sanity on the long prompt (does our residual stream look like what the SAEs were trained on?):

| Layer | Trained L0 | Measured L0 | Variance unexplained |
|---|---|---|---|
| 7 | 54 | 63.2 | 0.028 |
| 13 | 60 | 60.6 | 0.063 |
| 22 | 60 | 56.8 | 0.237 |

First generated tokens with the panel on (chat prompt, fused, CUDA graph):

- **' sky'**: entropy 0.002, top-3 [(' sky', 1.0), (' blue', 0.0), (' atmosphere', 0.0)], logit lens first agrees at layer 17, lens top-1 by layer {0: 'atrical', 5: ' aforementioned', 10: ' aforementioned', 15: ' entire', 20: ' sky', 25: ' sky'}, SAE active features [42, 52, 73]
- **' appears'**: entropy 0.082, top-3 [(' appears', 0.987), (' is', 0.008), ("'", 0.004)], logit lens first agrees at layer 20, lens top-1 by layer {0: ' became', 5: 'z', 10: ' naturally', 15: ' naturally', 20: ' appears', 25: ' appears'}, SAE active features [111, 69, 66]
- **' blue'**: entropy 0.0, top-3 [(' blue', 1.0), (' bluish', 0.0), (' Blue', 0.0)], logit lens first agrees at layer 16, lens top-1 by layer {0: 'of', 5: ' ска', 10: ' לו', 15: ' relatively', 20: ' blue', 25: ' blue'}, SAE active features [67, 67, 72]
- **' due'**: entropy 0.855, top-3 [(' due', 0.534), (' because', 0.416), (' primarily', 0.05)], logit lens first agrees at layer 24, lens top-1 by layer {0: "'", 5: ' čovjek', 10: ' في', 15: ' because', 20: ' because', 25: ' due'}, SAE active features [92, 77, 91]

## cuda:1 NVIDIA GeForce RTX 3090 (sm_86)

| Context / execution / panel | tokens/s | ms/token | vs panel off | working VRAM GiB | peak VRAM GiB | packet bytes | tokens matching baseline |
|---|---|---|---|---|---|---|---|
| chat / eager / off | 71.2 | 14.04 | — | 0.01 | 2.37 | 4 | 64/64 |
| chat / eager / naive | 22.7 | 44.06 | +214% | 0.01 | 2.38 | 4 | 64/64 |
| chat / eager / batched | 51.2 | 19.53 | +39% | 0.10 | 2.46 | 8284 | 64/64 |
| chat / eager / fused | 53.4 | 18.73 | +33% | 0.10 | 2.46 | 8284 | 64/64 |
| chat / cuda graph / off | 148.8 | 6.72 | — | 0.07 | 2.44 | 4 | 64/64 |
| chat / cuda graph / batched | 105.9 | 9.45 | +41% | 0.16 | 2.56 | 8284 | 64/64 |
| chat / cuda graph / fused | 114.0 | 8.77 | +31% | 0.16 | 2.59 | 8284 | 64/64 |
| long / eager / off | 70.2 | 14.25 | — | 0.14 | 2.60 | 4 | 64/64 |
| long / eager / naive | 22.9 | 43.75 | +207% | 0.14 | 2.60 | 4 | 64/64 |
| long / eager / batched | 51.3 | 19.50 | +37% | 0.14 | 2.60 | 8284 | 64/64 |
| long / eager / fused | 53.1 | 18.84 | +32% | 0.14 | 2.60 | 8284 | 64/64 |
| long / cuda graph / off | 102.9 | 9.72 | — | 0.14 | 2.60 | 4 | 64/64 |
| long / cuda graph / batched | 78.8 | 12.70 | +31% | 0.20 | 2.69 | 8284 | 64/64 |
| long / cuda graph / fused | 82.5 | 12.12 | +25% | 0.20 | 2.72 | 8284 | 64/64 |

SAE sanity on the long prompt (does our residual stream look like what the SAEs were trained on?):

| Layer | Trained L0 | Measured L0 | Variance unexplained |
|---|---|---|---|
| 7 | 54 | 63.2 | 0.027 |
| 13 | 60 | 60.5 | 0.063 |
| 22 | 60 | 56.8 | 0.237 |

First generated tokens with the panel on (chat prompt, fused, CUDA graph):

- **' sky'**: entropy 0.001, top-3 [(' sky', 1.0), (' blue', 0.0), (' atmosphere', 0.0)], logit lens first agrees at layer 17, lens top-1 by layer {0: 'atrical', 5: ' aforementioned', 10: ' aforementioned', 15: ' entire', 20: ' sky', 25: ' sky'}, SAE active features [42, 52, 74]
- **' appears'**: entropy 0.09, top-3 [(' appears', 0.985), (' is', 0.009), ("'", 0.005)], logit lens first agrees at layer 20, lens top-1 by layer {0: ' became', 5: 'z', 10: ' naturally', 15: ' naturally', 20: ' appears', 25: ' appears'}, SAE active features [109, 69, 70]
- **' blue'**: entropy 0.0, top-3 [(' blue', 1.0), (' bluish', 0.0), (' Blue', 0.0)], logit lens first agrees at layer 16, lens top-1 by layer {0: 'of', 5: ' ска', 10: ' לו', 15: ' relatively', 20: ' blue', 25: ' blue'}, SAE active features [66, 68, 74]
- **' due'**: entropy 0.842, top-3 [(' due', 0.537), (' because', 0.418), (' primarily', 0.044)], logit lens first agrees at layer 24, lens top-1 by layer {0: "'", 5: ' čovjek', 10: ' في', 15: ' because', 20: ' because', 25: ' due'}, SAE active features [92, 77, 89]

## Environment

```
recorded_at: 2026-10-10T08:07:04+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
