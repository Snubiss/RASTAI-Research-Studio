# ADR-0026: The live panel runs inside a CUDA-graph decode step, with a fused logit lens

**Status:** Accepted, 2026-10-10

## Context

- The live Psychology panel streams per-token readouts beside the chat: entropy, top guesses, a logit lens on every layer, top attended positions per head, and SAE features (§9).
- Live readouts slow generation (§28 risk 4). The question was how much.
- Spike 02, Gemma 3 1B in bf16:
  - Python's decode loop is launch-bound at ~67–71 tokens/s. Replaying one static-shape step as a CUDA graph gives **149–155 tokens/s**.
  - Panel cost: naive (per-layer hooks) **+207–219%**; batched (the CTO's rules) **+26–41%**; **fused +16–31% at CUDA-graph speed** (1.5–2.4 ms per token).
  - "Fused": the logit lens's last row *is* the model's output, so the 600 MB unembedding matrix is read once instead of twice.
  - Packet: **8.3 KB per token**, under the ~20 KB target. Peak VRAM of the whole stack: **2.7 GiB**.
  - The default CUDA-graph capture stream is created on whichever GPU was used first, which broke capture on the second GPU.
  - Sliding-window layers attended over the whole cache in static decode: 110 vs 155 tokens/s at long context.

## Decision

1. **Decode is a static-shape step, captured once as a CUDA graph** per loaded model and device during the `warming` model state, then replayed per token. Each device gets its own capture stream.
2. **Readouts are computed inside that step:**
   - one batched unembedding multiply for all layers, with the final row shared with the model's output (fused lens);
   - attention captured for the last token only, without copying shared key heads;
   - top-k computed on the GPU;
   - the whole readout packed into one tensor, with **one GPU→CPU copy per token**, so there is no extra sync per lens.
3. **Lens modes are separate captured graphs** (off, summary), so turning the panel on or off never re-captures mid-conversation.
4. **The packet** is the `LivePacket` message, streamed as ephemeral `psychology.packet` events. Under backpressure, packets are dropped, never decode.
5. **Sliding-window layers keep a ring-buffer cache** the size of their window (512 slots for Gemma 3).
6. **SDPA attention by default**; eager only where full attention maps are captured (replay zoom).
7. **Overhead budget:**
   - `psychology.overhead_budget_percent` = 35 at CUDA-graph speed (the proposal in §34);
   - the UI shows tokens/s with the panel on and off;
   - a nightly GPU test fails if the full panel exceeds the budget on Gemma 3 1B.
8. **Where readouts live:** `rastai_lens` stays in `python/` as a runtime feature exposed through the public API. The Psychology plugin holds the UI, the commands and the offline analyses.
9. **If graph capture fails** (an unsupported op, or a backend without graph support), decode falls back to the eager step with an `RRS-M4xxx` warning, and the UI shows the measured speed.

**Checked by:** a test that tokens with the panel on match tokens with it off (the spike: 64/64 in every configuration); the nightly overhead test; a two-GPU test capturing on GPU 1 after GPU 0 was used first.

## Consequences

**Good**
- The full live panel costs a measured +16–31%, inside the proposed 35% budget.
- Decode is twice as fast as the plain Python loop, with or without the panel.
- The same packed packet feeds local and remote panels (8.3 KB per token is small enough to ship).

**Costs**
- Static shapes constrain the code: no data-dependent branches in the step, fixed cache sizes, and MoE needs gather-by-index decode (ADR-0021).
- Capture takes time at load, and memory for each captured graph (one per lens mode and adapter state).
- Exact replays don't use these fast kernels (ADR-0027), so live and replay views can differ at bf16 near-ties.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Forward hooks with Python-side readouts | Measured at +207–219%. |
| A second forward pass for readouts | Doubles compute per token. |
| Readouts on the CPU | A GPU→CPU sync per lens per token. |
| Streaming full tensors to the UI | Megabytes per token. Summaries are 8.3 KB. |
| `torch.compile` instead of CUDA graphs | Kept as a later lever where measurement shows it helps. Graph replay alone already removes the launch-bound cost. |

## Links

- Spec: [§9](../spec/rastai_spec_v5.md#9-psychology-first-party-plugin) (live), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new) (performance), [§28](../spec/rastai_spec_v5.md#28-risk-register) risk 4, [§34](../spec/rastai_spec_v5.md#34-open-questions)
- Contracts: [§1 decision on `rastai_lens`](../architecture/contracts.md#1-repository-tree), [§4.4 LivePacket](../architecture/contracts.md#4-wire-contracts), [§9.3 model states](../architecture/contracts.md#9-state-machines), [§11.2 settings](../architecture/contracts.md#11-configuration)
- Spike: [02 live lens overhead](../../spike/results/02_live_lens_overhead.md)
- Related: [ADR-0013](0013-unified-model-runtime.md), [ADR-0021](0021-technique-coverage.md), [ADR-0025](0025-two-runtimes-per-model.md), [ADR-0027](0027-replay-from-token-ids.md)
