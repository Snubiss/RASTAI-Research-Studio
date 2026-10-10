# ADR-0021: Technique coverage: MoE, MLA, ALiBi, AttnRes, the RLHF chain and GSPO are in scope

**Status:** Accepted, 2026-10-10

## Context

- A check against a common list of 35 LLM techniques found 11 built in the spike, 12 planned and 12 missing (§0.17). The owner chose to cover all of them.
- The 12 missing ones are architectures (MoE, MLA, ALiBi, AttnRes), training methods (reward models, PPO, the RLHF chain, GSPO) and evaluation metrics (LLM-as-a-judge, Pass@k, Elo, perplexity). The metrics are covered in ADR-0020 and PPO's memory layout in ADR-0022.
- Most real MoE and MLA models don't fit on one consumer GPU (§28 risk 14). DeepSeek-V2-Lite is about 31 GB in bf16. The owner has 16 GB + 24 GB.
- Every block must stay observable and editable, or the unified-runtime promise (ADR-0013) breaks for these families.
- CUDA-graph decode needs static shapes (ADR-0026). MoE routing is data-dependent.

## Decision

**Architectures** (ModelSpec ops from contracts §5.3, compiled by `rastai_models`):

| Technique | How it's built | Observation and editing | Families |
|---|---|---|---|
| **MoE** | `moe` contains a `moe_router`, the `moe_expert` nodes and any shared experts, each a separate graph node. The first version loops over the experts that received tokens; grouped expert matmuls come only when measurement shows the loop is the bottleneck. Single-token decode gathers the k chosen experts' weights by index, so the step keeps a static shape for CUDA graphs. Designer settings: expert count, top-k, shared experts, load-balancing loss; live stats show total vs active parameters. | `blocks.<i>.router_logits`, `blocks.<i>.expert_choice`; `expert_overrides` (force, block, swap) in `InterventionSet`; `psychology.routing.stats`; routing shifts in the model diff | Mixtral, Qwen3-MoE, OLMoE, gpt-oss |
| **MLA** | `attention` with `variant: mla`. The cache holds the compressed latent, not full keys and values. Decode folds the up-projections into the query and output projections (the absorbed form). | `blocks.<i>.latent_cache` is its own observation point. Attention maps are computed from the compressed keys and values. | DeepSeek-V2/V3, Kimi (with MoE) |
| **ALiBi** | An `alibi` op producing a per-head distance bias, passed to attention as a mask. This rules out the fastest SDPA kernel, which only affects the older families that use ALiBi. | Same as other attention | BLOOM, MPT |
| **AttnRes** (experimental) | An `attn_res` op: each layer attends over earlier layers' outputs instead of adding to one running sum. Designer node first; imported families follow once public weights use it. | Each layer's depth-attention weights. The logit lens and SAEs read each layer's output and are labeled **approximate**, because they assume one residual stream. | none yet |

- **Layer placement across GPUs:** `placement{node_id: gpu_index}` in ModelSpec. Placement failures use `RRS-M5xxx`. DeepSeek-V2-Lite runs across the 5080 and the 3090.
- **Parity** (ADR-0013 rules): CI uses tiny random-weight configs of each new family. Real-weight parity runs nightly on OLMoE-1B-7B (one GPU) and DeepSeek-V2-Lite (both GPUs). MLA parity is checked in both the plain and the absorbed form.

**Training** (`rastai_train` recipes, each also usable on its own):
- **Reward model:** a scalar head on a backbone, trained with LoRA on preference pairs with a Bradley-Terry loss. Pairs come from "Prefer this branch" and imported DPO data. A trained reward model is also an eval metric and can rerank samples (best-of-n).
- **PPO:** a value head, generalized advantage estimation, and a KL penalty against the reference model. The reward comes from a reward model or a reward-function plugin. Memory layout: ADR-0022.
- **The RLHF chain:** `rlhf.start` runs preference pairs → reward model → PPO as one training run with a `verifying` step. The UI shows it as one chain.
- **GSPO is an option on the GRPO recipe**, not a separate recipe. It computes the importance ratio and clipping per sequence instead of per token. It is the default when the policy model contains a `moe` node, because per-token ratios make RL unstable there.

**Order:** these come after the core families. MoE, MLA, ALiBi, AttnRes, placement and the routing lens are in build step 6; the reward model, PPO, RLHF and GSPO are in step 5 (§31).

## Consequences

**Good**
- Rastai can open, inspect and fine-tune the current open MoE and MLA models, which is where the field is.
- Routing is observable and editable per token, which no wrapper-based tool offers.
- RLHF is one button for Basic users and separate recipes for Advanced ones.

**Costs**
- Several multi-week efforts: each family, the absorbed MLA form, placement, and the RL recipes.
- Real-weight parity for large models needs both GPUs and takes nightly GPU time.
- The MoE expert loop is slower than grouped kernels until measurement justifies those kernels.
- ALiBi families run on a slower attention kernel.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Leave the 12 out of the prototype | The owner decided on full coverage (§0.17). Leaving them out means the newest models only work as opaque nodes. |
| Grouped expert kernels from the start | Premature. It needs external or Triton kernels, and Blackwell has gaps (§28 risk 7). |
| MLA without the absorbed form | The full-KV cache defeats MLA's memory saving. |
| GSPO as a separate recipe | It differs from GRPO in one computation. A separate recipe would duplicate the rest. |
| Import AttnRes families now | No public weights use it yet. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 17, [§9](../spec/rastai_spec_v5.md#9-psychology-first-party-plugin), [§10](../spec/rastai_spec_v5.md#10-training-and-evaluation), [§11](../spec/rastai_spec_v5.md#11-model-designer), [§17](../spec/rastai_spec_v5.md#17-unified-model-runtime-new), [§28](../spec/rastai_spec_v5.md#28-risk-register) risks 14–15
- Contracts: [§5.3 ops and observation points](../architecture/contracts.md#5-documents-owned-by-rastai-formats), [§4.4 InterventionSet](../architecture/contracts.md#4-wire-contracts), [§6.4 commands](../architecture/contracts.md#6-commands)
- Related: [ADR-0013](0013-unified-model-runtime.md), [ADR-0020](0020-evaluation-first-party-plugin.md), [ADR-0022](0022-ppo-one-base-model-in-vram.md), [ADR-0026](0026-live-panel-in-decode-step.md)
