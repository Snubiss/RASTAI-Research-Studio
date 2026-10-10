# ADR-0010: VRAM leases in the node; Rastai manages its own llama-server

**Status:** Accepted, 2026-10-10

## Context

- Chat, Psychology and training compete for an 8–16 GB GPU (§28 risk 5). If they just compete, the loser crashes with an OOM at a random moment.
- The core loop keeps the chat model loaded while a LoRA trains (Teach it).
- Two runtimes (ADR-0025) and the trainer are separate processes, so no single framework sees the whole GPU.
- Ollama has its own scheduler that loads and evicts models on its own terms.
- Spike 03: Teach it trained with the chat model resident under an 8 GiB cap, peaking at **4.7 GiB**, while the chat model kept serving at ~71 tokens/s between epochs. The live Psychology stack peaked at **2.7 GiB** (spike 02).

## Decision

1. **The node owns GPU memory** through VRAM leases: `RequestLease`, `ReleaseLease` and `ListLeases` on `NodeService`, and `ResizeLease` on `WorkerHostService`.
2. **Two lease classes:**
   - `evictable` for interactive models (chat, Psychology);
   - `pinned` for training and gym jobs.
   States: `requested → granted | denied`; `granted → reclaim_requested → released` (evictable only); `granted → released`.
3. **A pinned request that doesn't fit reclaims evictable leases**, oldest use first. The holder unloads (model state `ready → evicting → unloaded`), and the UI says which model was unloaded and why. A pinned lease is never reclaimed; a request that can't fit is `denied` with `RRS-H3xxx` and the numbers.
4. **Each worker caps itself.** It applies `torch.cuda.set_per_process_memory_fraction` from the granted bytes at start and after every resize. An OOM inside the cap is reported as `RRS-H3xxx` with the lease size, not a bare traceback.
5. **`compute.vram_reserve_bytes`** is held back on every GPU for the driver and the UI.
6. **Rastai manages its own `llama-server`** (job kind `serve_fast`, `rastai-node/src/fast_runtime.rs`):
   - the build is pinned in `deps/engines/llama-server.json` with its SHA-256 and installed under `<data>/engines/llama-server/<build>/`;
   - it listens on loopback only, on a random port, with a random `--api-key`;
   - it can't cap its own memory, so its lease is the estimate from the GGUF size plus the KV cache for the chosen context length, corrected from measured usage after load.
7. **Ollama is an external adapter only**, never a managed runtime, because its scheduler would fight the leases.
8. **GPUs are assigned per role:** `compute.gpu_assignment{chat, training}` (for example, chat on GPU 0 and training on GPU 1). The Inspectable runtime uses one CUDA-graph capture stream per device (spike 02: the default capture stream broke the second GPU).

**Checked by:** a GPU test that loads chat on an 8 GiB cap, starts Teach it, and finishes with no OOM and the chat still answering; a test that a pinned request reclaims an evictable model and emits `lease.state_changed` and `model.state_changed`; a unit test of the lease arithmetic on fake GPUs (CPU CI).

## Consequences

**Good**
- Chat, Psychology and training share one consumer GPU predictably.
- OOMs become coded, explained events instead of random crashes.
- The node can tell the user what fits before anything loads.

**Costs**
- Memory estimates must be good. A wrong one wastes VRAM or causes an in-cap OOM. The Designer's memory estimate (ADR-0013) and measured corrections feed it.
- An evicted chat model has to reload, which costs seconds when the user returns to it.
- `llama-server` memory is enforced by estimate, not by a hard cap.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Let processes compete | OOM roulette. Training would lose hours of work. |
| CUDA MPS or MIG | NVIDIA-only (MIG is datacenter-only), and there's no ROCm equivalent. |
| One process holding every model | Couples crashes, and per-process caps stop working. |
| Ollama as the Fast runtime | Its own loading and eviction fights the leases. Rastai wants token IDs in and out, under its control. |

## Links

- Spec: [§8](../spec/rastai_spec_v5.md#8-inference), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new), [§22](../spec/rastai_spec_v5.md#22-compute-nodes-and-data), [§28](../spec/rastai_spec_v5.md#28-risk-register)
- Contracts: [§4.4 NodeService](../architecture/contracts.md#4-wire-contracts), [§9.3–9.4 model and lease states](../architecture/contracts.md#9-state-machines), [§11.2 settings](../architecture/contracts.md#11-configuration)
- Spike: [02 live lens overhead](../../spike/results/02_live_lens_overhead.md), [03 Teach it](../../spike/results/03_teach_it_lora.md), [04 Fast runtime](../../spike/results/04_tokenizer_and_fast_runtime.md)
- Related: [ADR-0001](0001-process-layout.md), [ADR-0022](0022-ppo-one-base-model-in-vram.md), [ADR-0025](0025-two-runtimes-per-model.md)
