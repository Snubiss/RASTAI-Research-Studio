# ADR-0025: Two runtimes per local model: Inspectable and Fast, with replay twins

**Status:** Accepted, 2026-10-10

## Context

- The Inspectable runtime (ADR-0013) sees every layer, but it is a PyTorch decode loop. llama.cpp is a tuned C++ engine with quantized kernels.
- Spike 04, Gemma 3 1B:

  | Runtime | RTX 5080 decode | RTX 3090 decode | Prefill (5080 / 3090) |
  |---|---|---|---|
  | llama.cpp f16 | 311 tok/s | 209 tok/s | 44k / 17k tok/s |
  | llama.cpp Q4_K_M | 496 tok/s | 267 tok/s | 36k / 16k tok/s |
  | Inspectable, bf16, panel off | 155 tok/s | 149 tok/s | ~36k / ~23k tok/s |

- From identical token IDs, the f16 GGUF matched our fp32 runtime on every token of all 4 prompts. Q4_K_M diverged after 3–9 tokens.
- Users want fast, quantized chat most of the time, and full inspection some of the time.

## Decision

1. **Every local model can run on two runtimes:**
   - **Inspectable** (`RuntimeKind.INSPECTABLE`): the compiled ModelSpec in a `serve_inspectable` Python job;
   - **Fast** (`RuntimeKind.FAST`): a managed `llama-server` in a node-native `serve_fast` job (ADR-0010). vLLM is an optional Fast engine on Linux serving nodes.
     *Note since acceptance (2026-10-10): a managed vLLM engine comes after the prototype (owner decision 8); the prototype has no vLLM job kind, environment or capability flag. An external vLLM server is reached now through `OpenAiAdapter` as an OpenAI-compatible endpoint (`RuntimeKind.API`), and its chats replay at `proxy` fidelity (ADR-0027). The two-runtime decision is unchanged.*
2. **One toggle in chat** switches between them (`chat.runtime.set`). The default is `inference.default_runtime = inspectable`.
3. **Both get token IDs from the gateway** (ADR-0008), so a conversation's token IDs are the same whichever runtime produced them.
4. **Psychology, interventions and branching with interventions run only on Inspectable.** A Fast-runtime conversation opens in Psychology through its **replay twin**: the full-precision weights mapped with `model.twin.set`, replayed from the logged token IDs (ADR-0027).
5. **Twin fidelity:**
   - `exact` only when the twin's weights are the same tensors the Fast runtime ran (an f16 or bf16 GGUF converted from the same safetensors);
   - `approximate` for any quantized GGUF, labeled with the quantization (for example "approximate: Q4 → bf16").
6. **Ollama is an external API adapter** (`RuntimeKind.API`), not a runtime (ADR-0010).
7. **Closing the gap is measured work.** `torch.compile` and fused kernels come in where measurement shows decode is launch-bound. An in-house fast runtime is a long-term option, not a prototype goal.

**Checked by:** a CI test that one conversation's token IDs are identical when generated through each runtime from the same prompt (greedy, f16 GGUF, CPU builds); a test that a Fast-runtime chat opened in Psychology is labeled with the twin's fidelity; the toggle e2e test from the §32 weeks 7–8 exit criterion.

## Consequences

**Good**
- Everyday chat runs at llama.cpp speed (on the 5080, 2× the Inspectable runtime in f16 and 3.2× in Q4), and inspection is one toggle away.
- Quantized models are served without in-house quantized kernels.
- Fidelity labels keep inspection honest when the weights differ.

**Costs**
- Two runtimes to keep in step: chat templates, stop tokens and sampling settings must mean the same thing on both. The gateway owns templates and tokens, and the sampling mapping is tested.
- Holding both a Fast and an Inspectable copy of a model costs VRAM. Leases arbitrate (ADR-0010).
- A pinned external engine build to track (`deps/engines/llama-server.json`).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Inspectable only | Half the decode speed of llama.cpp f16, no quantized serving, and worse on small GPUs. |
| llama.cpp only | No observation points, so Psychology would be impossible. |
| vLLM as the default Fast engine | CUDA-centric, heavy Python dependencies that conflict with the base environments, and aimed at serving, not a desktop. |
| Linking llama.cpp as a library | Couples crashes, adds ABI churn, and can't be swapped. A separate process is the §27 engine rule. |

## Links

- Spec: [§8](../spec/rastai_spec_v5.md#8-inference), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new) (Fast runtime), [§22](../spec/rastai_spec_v5.md#22-compute-nodes-and-data), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§4.1 RuntimeKind](../architecture/contracts.md#4-wire-contracts), [§6.4 inference commands](../architecture/contracts.md#6-commands), [§13 replay twin and fidelity](../architecture/contracts.md#13-glossary)
- Spike: [04 tokenizer and Fast runtime](../../spike/results/04_tokenizer_and_fast_runtime.md)
- Related: [ADR-0008](0008-gateway-owns-tokenization.md), [ADR-0010](0010-gpu-sharing-vram-leases.md), [ADR-0026](0026-live-panel-in-decode-step.md), [ADR-0027](0027-replay-from-token-ids.md)
