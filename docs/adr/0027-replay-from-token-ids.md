# ADR-0027: Replays recompute from token IDs; exact replays pin the attention kernel

**Status:** Accepted, 2026-10-10

## Context

- Psychology must show a past conversation's internals: rewind, branch, compare (§9). Storing every activation for every conversation would cost gigabytes.
- Shared replays must open with no model and no install, in the web viewer (§2: "useful before install").
- Spike 01b: on the 5080, one bf16 position flipped its top-1 token. Our bf16 eager path was bit-identical to HF's; the flip came only from PyTorch's efficient attention kernel summing in a different order, at a template-forced position with a 0.8-logit near-tie. The math and eager kernels didn't flip.
- Conversations come from four places with different guarantees: our gateway (exact token IDs), replay twins (different weights), text-only imports (retokenized) and API models (no weights at all).

## Decision

1. **What's stored per message:** token IDs, `model_hash`, `chat_template_hash`, sampling settings and the hardware fingerprint (in `vault.db` and in `.rrsreplay`). Activations are not stored by default.
2. **A replay is one forward pass from the token IDs** (`psychology.replay.open`), run on the node where the weights live. Only token IDs ship to remote nodes.
3. **Exact replays pin the attention kernel** to math or eager:
   - `GenerateRequest.pin_attention_kernel`;
   - setting `inference.pin_attention_kernel_for_replay`, default true;
   - the kernel is recorded in `HardwareFingerprint.attention_kernel`.
4. **Every replay carries a fidelity label:**

   | Fidelity | When |
   |---|---|
   | `exact` | same `model_hash`, token IDs from our gateway |
   | `approximate` | a replay twin with different weights (ADR-0025), or a text-only import that was retokenized |
   | `proxy` | an API-model chat replayed through a local observer model the user picks |

5. **`.rrsreplay` always contains a summary layer** (`summary/summary.safetensors` plus `summary/features.json`):
   - per token: entropy, the top-k final guesses, the top-k logit lens per layer and the top-k labeled features;
   - fp16 values and int32 ids; no attention;
   - target under 5 MB per 2,000 tokens, which is about 2.5 KB per token (smaller than the 8.3 KB live packet, because attention and the wider top-k are dropped);
   - `psychology.summary_layer_top_k` sets k.
   Full activations are optional (`activations/*.safetensors`).
6. **The viewer** (`apps/viewer`) opens a replay's summary layer with the WASM build of `rastai-formats`. It needs no daemon and no model.
7. **Honest reproducibility:** a replay on different hardware matches within a stated tolerance, not bit for bit. A deterministic mode is offered where the stack supports it.

**Checked by:** a test that replays a logged conversation and reproduces the logged greedy tokens with the kernel pinned; a size test that a 2,000-token replay's summary layer is under 5 MB; a viewer e2e test opening a fixture replay with no daemon running.

## Consequences

**Good**
- Replays cost kilobytes per conversation, not gigabytes, and recompute the full view on demand.
- Pinned kernels make "exact" mean exact on the same hardware.
- Shared replays are useful to people who haven't installed Rastai.

**Costs**
- Replays need the model weights (or a twin) for anything beyond the summary layer.
- Pinned math or eager attention is slower than SDPA's fast kernels, so replays of long contexts take longer.
- Live views (fast kernels, ADR-0026) and exact replays can differ at bf16 near-ties. The UI labels which is which.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Store activations for every conversation | Gigabytes per conversation, and still tied to one model version. |
| Replay with the fast kernels | Flips near-ties in bf16 (spike 01b), so "exact" wouldn't be exact. |
| Store logits only | Doesn't support lenses, attention or SAE features on recompute, and is still large. |
| Replays that need the app to open | Breaks "useful before install" and makes sharing pointless. |

## Links

- Spec: [§2](../spec/rastai_spec_v5.md#2-principles), [§8](../spec/rastai_spec_v5.md#8-inference) (replay twins), [§9](../spec/rastai_spec_v5.md#9-psychology-first-party-plugin) (replay), [§14](../spec/rastai_spec_v5.md#14-import--export), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new) (parity rules), [§26](../spec/rastai_spec_v5.md#26-code-and-reproducibility)
- Contracts: [§5.2 replay and conversation documents](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§4.4 GenerateRequest](../architecture/contracts.md#4-wire-contracts), [§13 fidelity](../architecture/contracts.md#13-glossary)
- Spike: [01b bf16 flip](../../spike/results/01b_bf16_flip_investigation.md), [02 packet size](../../spike/results/02_live_lens_overhead.md)
- Related: [ADR-0008](0008-gateway-owns-tokenization.md), [ADR-0025](0025-two-runtimes-per-model.md), [ADR-0026](0026-live-panel-in-decode-step.md)
