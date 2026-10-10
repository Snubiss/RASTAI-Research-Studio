# Experiment 01: in-house Gemma 3 1B vs Hugging Face

**Overall: FAIL**

Model: `unsloth/gemma-3-1b-it`. Reference: transformers 5.19.0 (eager attention).

Pass criteria (fixed before the first run):
- fp32: max |Δlogit| ≤ 0.005, mean KL ≤ 1e-06, top-1 agreement 100%.
- bf16: our drift from fp32 ≤ 1.25× the reference's own bf16 drift; top-1 within 1%.
- KV cache: decoding token by token matches one full pass, max |Δlogit| ≤ 0.005.
- Generation: 64 greedy tokens in fp32 are identical.

## cuda:0 NVIDIA GeForce RTX 5080 (sm_120): FAIL

Load time, fp32: ours 0.5s, reference 0.7s.

| Prompt | Tokens | fp32 SDPA max Δ | fp32 eager max Δ | fp32 KL | KV cache max Δ | Generation | bf16 drift ratio | bf16 top-1 (ours / ref) |
|---|---|---|---|---|---|---|---|---|
| short | 6 | 1.81e-05 | 0.00e+00 | -2.64e-09 | 2.19e-05 | identical (64 tok) | 0.891 | 100.0% / 100.0% |
| chat | 19 | 1.42e-04 | 0.00e+00 | -1.86e-08 | 9.92e-05 | identical (55 tok) | 1.048 | 94.7% / 100.0% |
| code | 17 | 9.35e-05 | 0.00e+00 | -5.76e-09 | 8.96e-05 | identical (64 tok) | 0.917 | 100.0% / 100.0% |
| long | 1600 | 4.62e-04 | 0.00e+00 | 1.02e-09 | 5.34e-05 | identical (64 tok) | 0.878 | 97.7% / 98.1% |

Failed checks: {'chat': ['bf16_drift']}

## cuda:1 NVIDIA GeForce RTX 3090 (sm_86): PASS

Load time, fp32: ours 5.3s, reference 5.5s.

| Prompt | Tokens | fp32 SDPA max Δ | fp32 eager max Δ | fp32 KL | KV cache max Δ | Generation | bf16 drift ratio | bf16 top-1 (ours / ref) |
|---|---|---|---|---|---|---|---|---|
| short | 6 | 2.10e-05 | 0.00e+00 | -1.43e-08 | 2.15e-05 | identical (64 tok) | 0.992 | 100.0% / 100.0% |
| chat | 19 | 1.67e-04 | 0.00e+00 | 8.67e-09 | 1.11e-04 | identical (55 tok) | 1.009 | 100.0% / 100.0% |
| code | 17 | 1.15e-04 | 0.00e+00 | -1.92e-09 | 6.60e-05 | identical (64 tok) | 0.916 | 100.0% / 100.0% |
| long | 1600 | 5.89e-04 | 0.00e+00 | -1.73e-09 | 6.48e-05 | identical (64 tok) | 0.892 | 97.8% / 97.6% |

Failed checks: none

## Environment

```
recorded_at: 2026-10-10T07:55:53+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
