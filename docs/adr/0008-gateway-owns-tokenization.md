# ADR-0008: The gateway owns chat templates and tokenization; runtimes get token IDs

**Status:** Accepted, 2026-10-10

## Context

- Replay, Psychology and training data all depend on exact token IDs. If each runtime tokenizes its own text, the logged IDs and the IDs the model actually saw can differ, and nobody can tell.
- Two runtimes serve the same model (Inspectable and `llama-server`, ADR-0025). Their tokenizers are different code.
- Tokenizers and Jinja chat templates are byte-exact compatibility surfaces (§27 category 3). Rebuilding them in-house is a correctness risk with no payoff.
- Spike 04: HF `tokenizers` 0.23.3 and llama.cpp b11537 gave **0 mismatches** on 162 cases and 49,035 tokens (every spec paragraph, the whole spec, source code and 20 edge cases), with and without begin-of-text, and on detokenization. From identical token IDs, an f16 GGUF matched our fp32 runtime on every token of all 4 prompts.
- Spike 07: the Rust tokenizer ran at 2.83 M tokens/s. About 250 MiB of each gateway's memory was the Gemma tokenizer itself (262k-entry vocabulary). Java's practical tokenizer (DJL) silently truncated 7 of 162 cases by default.

## Decision

1. **`rastai-gateway` renders chat templates and tokenizes for every local runtime.**
   - Tokenizer: the HF `tokenizers` crate, imported only in `rastai-gateway/src/tokenizer.rs`.
   - Templates: `minijinja`, imported only in the gateway's template module.
   - Truncation and padding are always off; the gateway checks lengths itself and returns `RRS-G1xxx` when a prompt is too long.
2. **Runtimes receive token IDs, not text.**
   - The Inspectable worker gets `GenerateRequest.prompt_token_ids`.
   - `llama-server` gets token IDs on `/completion`. Its `/tokenize` is used only in parity checks.
3. **All logged token IDs come from the gateway**: `gateway_log.prompt_ids` and `generated_ids`, and `messages.token_ids`, with `model_hash` and `chat_template_hash`.
4. **Third-party API adapters** (OpenAI, Anthropic, OpenAI-compatible) log text plus any logprobs returned. Their replays are labeled `proxy` (ADR-0027).
5. **The same crate runs in two places:**
   - in the daemon, for the user endpoints, logging through the vault `LogSink`;
   - in the node, for rollouts and judges (`WorkerHostService.GatewayGenerate`), logging to the run's event log.
   The gateway never imports the store.
6. **Incognito and retention are enforced in the gateway.** An incognito request never calls the `LogSink`.
7. **Streaming responses set `TCP_NODELAY`.** Without it, both bake-off backends stalled 40 ms per reply.
8. **Python-side token counts** (`rastai_data` previews) use the Python `tokenizers` package, pinned to the same release as the crate.

**Checked by:** the 162-case corpus in `fixtures/tokenizer/` runs in CI against the gateway tokenizer and the pinned `llama-server` build (CPU), expecting 0 mismatches; template tests render each supported family's chat template against reference strings; an incognito test finds zero log rows.

## Consequences

**Good**
- One source of truth for token IDs. Replay twins and token parity are checkable.
- Exact replays need only the token IDs, not the text.
- A tokenizer or template bug is fixed in one place.

**Costs**
- Each loaded tokenizer costs memory (~250 MiB for Gemma's).
- The gateway must know each model's template, special tokens and stop tokens. Template quirks between Jinja2 and minijinja are ours to find, and template tests cover them.
- Runtimes that only accept text can't be used as local runtimes without losing exact token IDs.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Each runtime tokenizes its own text | Logged IDs may not be what the model saw, and the two runtimes could disagree silently. |
| Tokenize in the Python worker | The Fast runtime bypasses the worker, so its IDs would come from elsewhere. Adds a hop. |
| An in-house tokenizer | A byte-exact surface across hundreds of model vocabularies. High risk, no benefit. |
| Java with DJL | Truncates by default, phones home, and wraps the same Rust library through JNI (spike 07). |

## Links

- Spec: [§8](../spec/rastai_spec_v5.md#8-inference), [§18](../spec/rastai_spec_v5.md#18-gateway-and-tokenization), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new)
- Contracts: [§2.1 rastai-gateway](../architecture/contracts.md#2-component-inventory), [§8.3 vault.db](../architecture/contracts.md#8-storage)
- Spike: [04 tokenizer and Fast runtime](../../spike/results/04_tokenizer_and_fast_runtime.md), [07 backend bake-off](../../spike/results/07_backend_bakeoff.md)
- Related: [ADR-0016](0016-backend-language-rust.md), [ADR-0025](0025-two-runtimes-per-model.md), [ADR-0027](0027-replay-from-token-ids.md)
