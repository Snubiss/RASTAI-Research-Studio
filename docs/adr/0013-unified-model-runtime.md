# ADR-0013: ModelSpec is the Inspectable runtime

**Status:** Accepted, 2026-10-10

## Context

- The Designer needs a compiler from a design to PyTorch modules. Psychology needs observation points on imported HF models. Training needs both.
- Doing this with HF `transformers` plus a hooking library would mean two runtimes: one for designed models and one for imported ones. Psychology would behave differently on each, and hooks on someone else's modules break when upstream changes.
- The bet: an in-house implementation can match Hugging Face exactly. The spike tested it on Gemma 3 1B:
  - fp32 with eager attention: max |Δlogit| = **0** on all 4 prompts on both GPUs;
  - fp32 with SDPA: within 6×10⁻⁴, KL ~10⁻⁹;
  - 64-token greedy generations identical; KV-cache decode matches one full pass;
  - bf16 drift from fp32 is 0.88–1.05× the reference's own;
  - the one bf16 top-1 flip came from PyTorch's efficient attention kernel at a template-forced position with a 0.8-logit near-tie (01b).
  The from-scratch model is about 500 lines including comments and the static decode path.

## Decision

1. **Every model Rastai inspects or trains runs as a compiled ModelSpec** (`rastai_models`).
   - A designed model is a ModelSpec.
   - An HF model is mapped by its family (`config.json` plus weight names → ModelSpec blocks), compiled to PyTorch modules by the in-house compiler, and loaded from safetensors in the HF cache.
2. **Every graph node is an observation point**, named `<node path>.<output>` (for example `blocks.3.residual_out`). Psychology, steering, ablation and LoRA attach to nodes. No hooking library.
3. **Family order:** Gemma 3 and Llama first (the First Run and the SAEs need them); then Qwen, Mistral, GPT-2, BERT and ViT; then the MoE, MLA and ALiBi families (ADR-0021).
4. **A family is supported only when it passes the parity suite** in `tests/parity/`, against HF `transformers` as a test-time reference. Criteria (from the spike):
   - fp32: max |Δlogit| ≤ 0.005, mean KL ≤ 10⁻⁶, top-1 agreement 100%;
   - bf16: our drift from fp32 ≤ 1.25× the reference's own bf16 drift;
   - KV-cache decode matches one full pass;
   - 64 greedy tokens identical in fp32;
   - template-forced positions (where the chat template forces the next token) are skipped.
   CI runs tiny random-weight configs per family (`fixtures/models/`); real-weight parity runs nightly on the GPU runner.
5. **Unmapped architectures import as opaque nodes:** shapes known, internals sealed, observable at their edges. Import never fails outright.
6. **The compat runtime** (`RuntimeKind.COMPAT`) runs opaque models through HF `transformers` so they can still chat and train, with Psychology limited to the node's edges. `trust_remote_code` is always off. Otherwise `transformers` is only a test-time reference and is never imported by shipped code outside the compat wrapper.
7. **The trainer runs on the same compiled modules.** Freeze, LoRA and train are graph-node attributes (`train_mode`).

**Checked by:** the parity suite per family, in fp32 and bf16, on NVIDIA and (once the card is in) AMD; the §32 weeks 5–6 exit criterion (Gemma 3 and Llama pass parity on both vendors); an import test that an unknown architecture yields an opaque node, not an error.

*Note since acceptance (2026-10-10):*
- *M2 closes on NVIDIA. The AMD parity gate moves to M10, and the owner buys a discrete RDNA3+ card before M10 (owner decision 11). The parity criteria are unchanged.*
- *Real-weight parity runs on a self-hosted GPU runner (`nightly-gpu.yml`: schedule and manual triggers on main only, a separate OS user, a read-only `HF_READ_TOKEN` for gated weights) once the repo is public (owner decision 12).*
- *One owner per algorithm: each ModelSpec op has one golden file, `fixtures/ops/<op>.json` (shapes, parameter count, FLOPs), that both the Rust live stats and this compiler must match; layer placement is planned only in Rust and the compiler applies the `placement{}` field (contracts §2.4 rule 10).*

## Consequences

**Good**
- One runtime for designed and imported models. Psychology works the same everywhere.
- Observation points come for free and can't break on an upstream release.
- Parity tests turn "is our implementation right?" into a number checked in CI.

**Costs**
- Every family is real work, and new architectures lag behind upstream (§28 risk 2). The opaque node and the compat runtime are the fallback.
- Inspectable decode is slower than llama.cpp (155 vs 311 tokens/s for the same model, spike 04). ADR-0025 and ADR-0026 deal with this.
- The parity suite needs the HF reference installed in the test environment, and GPU time.

## Alternatives considered

| Alternative | Why not |
|---|---|
| HF `transformers` plus a hooking library (nnsight, TransformerLens) | Two runtimes. Hooks depend on upstream module internals. `trust_remote_code` pressure. |
| `torch.fx` tracing of HF models into a graph | Tracing breaks on dynamic control flow, and the traced graph has no meaningful names. |
| Native Rust compute (Burn, tch-rs) | Long-term option only. PyTorch is the foundation for kernels and autograd today. |

## Links

- Spec: [§11](../spec/rastai_spec_v5.md#11-model-designer), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new), [§28](../spec/rastai_spec_v5.md#28-risk-register) risk 2, [§32](../spec/rastai_spec_v5.md#32-first-90-days)
- Contracts: [§5.2–5.3 ModelSpec and ops](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§13 glossary](../architecture/contracts.md#13-glossary)
- Spike: [01 Gemma parity](../../spike/results/01_gemma_parity.md), [01b bf16 flip](../../spike/results/01b_bf16_flip_investigation.md)
- Related: [ADR-0021](0021-technique-coverage.md), [ADR-0025](0025-two-runtimes-per-model.md), [ADR-0027](0027-replay-from-token-ids.md)
