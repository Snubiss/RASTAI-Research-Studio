# Experiment 01b: the single bf16 top-1 flip on the RTX 5080

- Our bf16 **eager** path bit-identical to the reference's bf16 eager path: **True** (max |Δ| 0).
- Median fp32 gap between the top two tokens on this prompt: 1.411 logits.

Positions where any bf16 run disagrees with fp32:

| Run | Disagreeing positions |
|---|---|
| reference_bf16_eager | none |
| rastai_bf16_eager | none |
| rastai_bf16_sdpa_default | [16] |
| rastai_bf16_sdpa_math | none |
| rastai_bf16_sdpa_efficient | [16] |

- Position 16 (context `' sky is blue.\n'`): fp32 top two [' सामाजिक', 'override'] separated by **0.7958** logits; chosen per run: {'reference_bf16_eager': ' सामाजिक', 'rastai_bf16_eager': ' सामाजिक', 'rastai_bf16_sdpa_default': 'override', 'rastai_bf16_sdpa_math': ' सामाजिक', 'rastai_bf16_sdpa_efficient': 'override'}
