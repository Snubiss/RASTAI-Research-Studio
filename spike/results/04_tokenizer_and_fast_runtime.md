# Experiment 04: tokenizer parity and the Fast runtime

llama.cpp build b11537-8e2d31e0e. Gateway tokenizer: HF `tokenizers` 0.23.3.

## 1. Tokenizer parity: **PASS**

162 cases, 49,035 tokens (every spec paragraph, the whole spec, source code, 20 edge cases).

| Check | Mismatching cases |
|---|---|
| without begin-of-text | 0 |
| with begin-of-text | 0 |
| detokenize | 0 |


## 2. Generation parity (64 greedy tokens from identical token IDs)

**f16 GGUF on the RTX 5080**

| Prompt | Tokens agreeing with ours (bf16) | with ours (fp32) |
|---|---|---|
| short | 64/64 | 64/64 |
| chat | 53/55 | 55/55 |
| code | 64/64 | 64/64 |
| long | 64/64 | 64/64 |

**Q4_K_M GGUF on the RTX 5080**

| Prompt | Tokens agreeing with ours (bf16) | with ours (fp32) |
|---|---|---|
| short | 9/64 | 9/64 |
| chat | 4/48 | 4/48 |
| code | 5/64 | 5/64 |
| long | 3/64 | 3/64 |

## 3. Speed

| Runtime | GPU | Decode tokens/s | Prompt tokens/s |
|---|---|---|---|
| llama.cpp f16 | RTX 5080 | 311 | 44066 |
| llama.cpp f16 | RTX 3090 | 209 | 17438 |
| llama.cpp Q4_K_M | RTX 5080 | 496 | 35903 |
| llama.cpp Q4_K_M | RTX 3090 | 267 | 16204 |
| Rastai Inspectable bf16, panel off (exp. 02) | cuda:0 NVIDIA GeForce RTX 5080 (sm_120) | 155 | — |
| Rastai Inspectable bf16, full panel (exp. 02) | cuda:0 NVIDIA GeForce RTX 5080 (sm_120) | 125 | — |
| Rastai Inspectable bf16, panel off (exp. 02) | cuda:1 NVIDIA GeForce RTX 3090 (sm_86) | 149 | — |
| Rastai Inspectable bf16, full panel (exp. 02) | cuda:1 NVIDIA GeForce RTX 3090 (sm_86) | 114 | — |

## Environment

```
recorded_at: 2026-10-10T08:29:05+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
