# Rastai Research Studio: Product and Technical Scope v5

**Status:** draft v5.3, 2026-10-10. v5.1 added the business model (§36). v5.2 applies the spike's findings (`spike/README.md`) and locks the license. v5.3 closes the technique coverage gaps and makes Evaluation a first-party plugin (§10).

**Merges:**
- v4.1 (three reviews: ecosystem/interop, product, compute/cloud);
- the CTO review;
- the owner's answers to the CTO's questions.

**Biggest change from v4.1:** the owner prefers building in-house over adopting external libraries. Most build-vs-buy calls are flipped (§27). Imported models now run on Rastai's own runtime, which is the same code as the Designer's compiler (§17).

v4 (`rastai_spec_v4.md`) is kept for history.

---

# Part I: Product

## 0. Decisions

**Decided**
1. **The prototype is the whole suite.** Every section and function in this scope works end to end, at prototype quality (§5).
2. **Build in-house by default.** External code is used only for foundations, security-critical pieces, byte-exact compatibility surfaces, and swappable external engines (§27).
3. **Audience:** the owner first, everyone later. The prototype puts depth (Advanced mode) first. Basic-mode polish and the novice First Run come in Phase 2. The Basic/Advanced split is built into the architecture from day one (§4).
4. **Platforms:** Linux first, on NVIDIA and AMD (ROCm), both tier 1. macOS and Windows come later; all three are the long-term goal. There's no Mac on hand, and an AMD GPU is available for testing (§4).
5. **Remote in the prototype:** SSH pairing to your own boxes, plus headless mode. RunPod and other cloud providers come in Phase 3 (§22).
6. **Agent-gym training runs on Linux only** for now.
7. **Encryption at rest comes later (Phase 2).** Conversations are kept in their own database file now, so encryption can be switched on without a migration (§19).
8. **No target date.** Progress is measured by milestone exit criteria (§32).
9. **JEV is dropped as a template for now.** It's a CNN trained a specific way, and it gets built as a Rastai template once the data and custom input/output work is finished.
10. **Name:** "Rastai Research Studio" has no existing hits. The follow-ups are in §34.
11. **Business model:** free for everyone, including companies; businesses pay for org features, hosted services and support; a marketplace for creators (§36).
12. **License:** Apache-2.0 for the app, formats and SDKs; a trademark policy for the name; contributions under a DCO (§35).
13. **The spike (§32, weeks 1–2) is done.** The unified-runtime bet holds; results and the changes they caused are in `spike/README.md` and marked "(spike)" below.
14. **Backend language: Rust, re-examined and kept.** The owner asked for Java to be tested too. The spike built the same backend pieces in both (safetensors reader, gateway tokenization, streaming gateway with SQLite logging; experiment 07). They tie on correctness. Rust leads on throughput, memory, startup and shippable size, and Java's practical tokenizer is the same Rust library behind JNI, so Rust stays.
15. **Definition of done:** L0–L3 (§5) adopted.
16. **AMD testing waits for a discrete AMD GPU (RDNA3 or newer).** AMD stays tier 1 in the design (§4), and the code keeps it cheap to add (PyTorch's ROCm build uses the same `torch.cuda` API, llama.cpp ships ROCm builds). The owner's box only has the Ryzen's integrated graphics, which ROCm can't use.
17. **Technique coverage: all of it.** A check against a common list of 35 LLM techniques found 11 built in the spike, 12 planned and 12 missing. The 12 missing ones are now in scope:
    - architectures: mixture of experts (MoE), multi-head latent attention (MLA), ALiBi, and attention residuals (AttnRes, experimental) (§11, §17);
    - training: reward models, PPO, the full RLHF chain, and GSPO (§10, §12);
    - evaluation: LLM-as-a-judge, Pass@k, Elo ratings and perplexity (§10).
18. **Evaluation is a first-party plugin,** the second after Psychology. It tests models the way a test suite tests code (§10).

**Open**
- Nothing blocking.

## 1. Goal
A professional, all-in-one suite for designing, training, fine-tuning, testing, serving and studying neural networks. Basic users need little or no programming knowledge, and professionals can go as deep as they want. Like Photoshop, it's simple on the surface and sophisticated underneath.

Models can be trained on any data: files, datasets, streaming data or single inputs. The user picks what goes in and what they expect out, without needing to understand the code underneath.

It runs on local hardware or on GPUs you rent in a datacenter. Switching between them is a dropdown, not a code change.

## 2. Principles
- **So good no one wants to clone it.** People just use it.
- **Build it ourselves.** Depth is the moat, and in-house code is where the depth comes from. We own the engine, its behavior and its bugs. Upstream changes and supply-chain attacks are the risk we avoid by not depending on others.
- **Open source from day one.** Nobody clones what they can fork, and improvements come back as contributions.
- **Be the default file format.** Every shared architecture, harness, replay or dataset is an invitation to Rastai.
- **Useful before install.** A shared file is useful even before anyone installs Rastai: a web viewer for replays and designs, and `pip install rastai` to load designs.
- **Zero friction.** Install to first meaningful result in under 10 minutes. The novice version of this lands in Phase 2.
- **Extract APIs, don't invent them.** Plugin APIs are extracted from first-party code that already uses them.
- **Every step ships.** Each build step adds a section that works.
- **Durability first.** Nothing the user did is ever lost to a crash.

## 3. Positioning
**Rastai is the studio for your own models: design them, see inside them, fix them, prove it.**

Every feature serves one loop:

**Design / Import → Observe → Explain → Edit → Train → Verify**

| Stage | What happens | Where |
|---|---|---|
| Design / Import | Build a model, combine pretrained blocks, or import any HF model | Designer |
| Observe | Every model call is logged with its token IDs | Gateway / Inference |
| Explain | See why the model said what it said | Psychology |
| Edit | Steer, ablate, branch, pick the better answer | Psychology |
| Train | Edits become data (DPO pairs, SFT targets, LoRA) | Training |
| Verify | Behavior tests graded by a judge (not just keywords), drift on a fixed general prompt set, and a model diff prove the fix | Evaluation plugin |

**What we don't compete on:**
- Chat UI breadth. Unsloth Studio, LM Studio and Open WebUI own that.
- A no-code fine-tuning wizard on its own.

Those surfaces exist here to serve the loop.

**Gaps nobody fills:**
- Interpretability on *your own* fine-tunes, run locally and wired into training. Neuronpedia covers base models, on the web.
- A professional designer whose model *is* the runtime: one design carried from import through training and serving to re-import.
- Edits that become training data, and proof that a fix worked.
- An in-house engine where every layer can be observed, because we wrote every layer.

**Prior art we learn from (not dependencies):**
- Fine-tuning studios: Unsloth Studio, Transformer Lab, LLaMA-Factory, Kiln.
- Agent RL: rLLM and Harbor.
- Interpretability tooling: nnsight, TransformerLens, SAELens, circuit-tracer.
- Data and run tooling: Ludwig, Trackio.

## 4. Audience and Platforms
**Audience**
- **Prototype:** the owner, an experienced ML/engineering power user. Advanced depth gets polished first.
- **Phase 2 onward:** everyone. Basic mode, the novice First Run, guided tutorials and plain-English explanations get polished.
- The architecture supports both from day one: one global Basic/Advanced setting, and every feature has a Basic surface defined, even if it starts out rough.

**Platform tiers**

| Tier | Platform | Status |
|---|---|---|
| 1 | Linux + NVIDIA (CUDA 13; RTX 5080 16 GB and RTX 3090 24 GB on hand) | Built and tested in the prototype |
| 1 | Linux + AMD (ROCm; needs a discrete RDNA3+ card, not yet on hand) | Built and tested in the prototype once the card is in |
| 1 | Linux CPU-only | Built and tested (CI, small models) |
| 2 | macOS Apple Silicon (MPS) | Designed for; untested until a Mac is available |
| 2 | Windows (native or WSL2) | Designed for; Phase 2+ |

Design rules that keep tier 2 cheap later:
- The UI is web-based, with Tauri as the shell.
- Local IPC uses an abstraction that works over Unix sockets and Windows named pipes.
- Sandboxes work with any Docker-API engine.
- Paths are never hard-coded.
- Every backend is selected through hardware detection.

**Budget (no spend needed for the prototype)**
- **Code signing:** macOS and Windows warn about or block unsigned installers. It's only needed when shipping those platforms.
- **GPU rental:** only for testing Phase 3 cloud providers and hardware you don't own.
- **A Mac:** needed to bring macOS to tier 1.
- **A discrete AMD GPU (RDNA3+):** needed to test ROCm for real. The Ryzen's integrated graphics (gfx1036) has no ROCm kernels, and forcing it gives silently wrong results (spike).
- **The owner's box as a CI runner** (optional): lets GPU tests run automatically every night instead of by hand.

## 5. Definition of Done
Every feature is rated on four levels:
- **L0:** the command exists and fails cleanly with a catalog error code.
- **L1:** the main path works end to end; failures are recoverable and nothing is lost.
- **L2:** common variants work and have tests.
- **L3:** polished: Basic-mode UX, docs, performance budgets met.

**The prototype is done when:**
- Every section is at L1.
- The core loop is at L2: chat → see inside → steer or branch → train → verify.
- It runs on Linux NVIDIA, Linux AMD and a paired SSH box.

## 6. First Run
This is the target experience. In the prototype the steps work at L1 for the owner; the novice polish comes in Phase 2.
- **0–3 min:** the installer detects NVIDIA, AMD or CPU (and, later, Apple). It creates the pinned Python environment and fetches Gemma 3 1B-it plus SAE weights for 3 layers (Gemma Scope 2) in the background, showing the model license. Other layers download on demand.
  - It shows the download size per backend before starting. Measured (spike): CUDA 5.2 GB in 33 s, ROCm 16.5 GB in 91 s, CPU 0.75 GB in 3 s on the owner's connection; a reinstall from cache takes under 1 s.
  - Gemma's official weights are gated on Hugging Face, so the first run includes the HF sign-in and license-acceptance flow (§25).
- **3–5 min:** chat. Text is shaded by confidence; click a word to see what the model almost said and which concepts were active.
- **5–7 min:** drag a concept slider and watch the answer change live.
- **7–10 min:** rewind, branch, pick the better answers, and press **Teach it**. A LoRA trains in a few minutes, then before and after show side by side with the model diff.
- **No usable GPU:** say so plainly. Offer the 270M model, or pairing a rented GPU from any provider over SSH. One-click RunPod comes later.
- **Remote:** first remote run in under 15 minutes, from pairing a box to training on it.
- **Home screen, organized by outcome:** Chat · Teach it my data · See inside · Fix a behavior · Design a network · Train an agent.

## 7. UX System
- **Basic/Advanced:** one global setting, overridable per panel. Advanced adds controls and never hides or moves Basic ones; switching keeps all state.
- **Command palette** (Ctrl/Cmd-K) over the command layer, showing each command's shortcut and "copy as CLI / Python / MCP".
- **Design system:** one in-house package (`packages/ui`) used by first-party sections and plugins alike.
- **One catalog** covers error diagnostics and plain-English definitions of terms.
- **Tutorials** check real app state through commands. The assistant can "show me" or "do it". (Phase 2 for polish.)
- **Accessibility (WCAG 2.2 AA):** colorblind-safe palettes, a table behind every chart, a keyboard-driven token timeline, reduced motion for 3D views.
- **Honest wording:** "what it's representing", not "what it's thinking".
- **Workspace layouts** are files, stored in the user's profile or in a project.

## 8. Inference
- Chat interface for direct conversation, manual testing and basic chat.
- Side panel listing models and sessions.
- Direct inference through third-party APIs (OpenAI, Anthropic, others), using in-house adapters.
- Local OpenAI-compatible and Anthropic-compatible endpoints. Navi and any other client infer through them, with logging and the same adapters.
- Adjustable generation settings for local models: temperature, top-k, top-p, repetition penalty, max tokens, seed.
- **Two runtimes per local model:**
  - **Inspectable (in-house):** the model runs through Rastai's own runtime (§17), with every layer observable and editable.
  - **Fast (external engine):** llama.cpp's `llama-server`, managed by Rastai. vLLM is optional, for Linux serving nodes.
  - One toggle in chat switches between them.
- **Replay twins:** a quantized Fast-runtime model maps to its full-precision weights for inspection, with the fidelity labeled ("approximate: Q4 → bf16").
- **API-model chats** log text plus any logprobs the API returns. They replay through a local observer model the user picks, labeled "proxy".
- Delete, archive, star or pin conversations.
  - Starred → dataset.
  - Pinned → behavior test (§10).
- Open any conversation in Psychology for replay.
- An optional live Psychology panel sits beside the chat.
- Incognito chats are never logged.
- Retention setting for gateway logs.

## 9. Psychology (first-party plugin)
Inference plus analysis tools for seeing inside a model. It's all built in-house on the runtime's observation points (§17). It works on any model the Inspectable runtime can run, and is greyed out (or proxy-only) for API models.

**Basic view: "what it's representing"**
- **Active concepts per token:** labeled SAE features.
- **What it almost said:** the model's top guesses at each layer (logit lens).
- **What it's looking at:** attention heatmaps.
- **Confidence meter:** per-token entropy.

**Advanced view**
- 2D/3D activation projections per layer, attention per head, linear probes and feature dashboards.
- Attribution graphs: an in-house implementation running as an offline job, starting with Gemma 3 270M/1B. Prior art: circuit-tracer.
- **Which experts (MoE models):**
  - for each token and layer, the experts the router picked and their gate weights;
  - how often each expert is used over a conversation or dataset, with dead and overloaded experts flagged;
  - interventions to force, block or swap an expert.
  - The model diff includes routing shifts after fine-tuning.
- **Latent attention (MLA models):** attention maps are still shown (they're computed from the compressed keys and values), and the compressed cache is an observation point of its own.
- **Depth attention (AttnRes designs):** each layer's weights over the earlier layers it draws from. The logit lens and SAEs assume one residual stream, so on these models they read each layer's output and are labeled approximate.

**Visuals** (in-house WebGL2 renderer)
- **Vector space view:** the projection is fit once and new points are placed into that fixed space, so the map stays stable.
- **3D data-flow view:** layers × tokens, with level of detail (summaries zoomed out, heads and features zoomed in).

**Replay**
- Play, pause, stop, rewind, slow down or speed up along a token timeline.
- The full view is recomputed from token IDs with one forward pass when the model is present.
- Every `.rrsreplay` embeds a summary layer (per-token entropy, top-k logit lens, top-k labeled features), targeting under 5 MB per 2k tokens. It opens with no model and no install in the web viewer.

**Live**
- A panel beside the chat streaming per-token summaries. Step mode: generate one token, pause, inspect, continue.
- Performance rules, from the CTO review and the spike:
  - one batched unembedding multiply for all layers;
  - **fused logit lens:** the lens's last row is the model's own output, so the unembedding matrix is read once, not twice (spike);
  - attention captured for the last token only, without copying shared key heads;
  - top-k computed on the GPU;
  - no extra GPU→CPU sync per lens: the whole readout is one packed tensor, one copy per token;
  - decoding runs as a captured static-shape step (a CUDA graph); a Python decode loop is launch-bound at a third of the speed (spike).
- **Measured budget (spike, Gemma 3 1B, bf16):** the full panel costs +16–31% (1.5–2.4 ms per token) at CUDA-graph speed, the packet is 8.3 KB per token, and the whole stack peaks at 2.7 GiB.
- The measured overhead budget is shown in the UI (tokens/s with the panel on vs off).
- **Remote models:**
  - Summaries are computed on the node, about 20 KB per token.
  - Full tensors are fetched only when the user zooms in.
  - Replays recompute where the weights live, so only token IDs are shipped.

**Interventions and branching**
- Boost or suppress concepts, ablate heads, apply steering vectors.
- Rewind to any token, intervene, regenerate, and compare branches side by side.

**Edits become training (the loop)**
- **Prefer this branch:** a branch pair saves as a DPO pair or SFT target, linked to its replay.
- **Steering presets:** a saved intervention can be served through the gateway, or distilled into a LoRA.
- **Model diff** (base vs checkpoint): changes in how often features fire, logit-lens shifts and eval deltas. Unexpected changes are flagged.
- **SAE fit check:** after fine-tuning, measure how well the SAEs still reconstruct the model. Below a threshold, labels show "stale" and SAE fine-tuning is offered.

**Other**
- **Training view:** watch representations organize themselves epoch by epoch.
- **Honest readouts:** every label shows a confidence score and example inputs. They are evidence, not mind-reading.
- **Data, not code:** SAE weights (Gemma Scope 2, Llama Scope) and Neuronpedia labels are loaded as data by our runtime. Their licenses are checked and shown.

## 10. Training and Evaluation
All on the in-house trainer (§17).
- Same interface as inference, plus a second window for live training and the dashboard.
- **Data import:**
  - Drop in a file or folder and the task is detected: label column → classifier; prompt/response → SFT; pairs → DPO; raw documents → synthetic Q&A recipe.
  - Before training: preview, token counts, splits, dedupe (in-house MinHash), train/test leakage check and PII flag.
  - Converters are generated from the preview, not written by hand.
- **Converters (data formats):** a declarative instruction file, with a Python fallback. Adding a format means adding one file.
- **Recipes (training methods):** classification, pretraining, SFT, DPO, GRPO, GSPO, reward model, PPO, LoRA, QLoRA, full fine-tuning, SAE training, JEPA objectives. Each recipe is an in-house plugin on the trainer.
- **RLHF, end to end:** preference pairs → reward model → PPO. The UI shows it as one chain, and each step is also a recipe on its own.
  - **Reward model:** a scalar head on a backbone, trained with LoRA on preference pairs (Bradley-Terry loss). The pairs come from "Prefer this branch" (§9) and imported DPO data. A trained reward model is also an eval metric and can rerank samples (best-of-n).
  - **PPO:** a value head, generalized advantage estimation, and a KL penalty against the reference model. The reward comes from a reward model or a reward-function plugin.
  - **One base model in VRAM:** the policy is a LoRA, the reference is the same base with the adapter switched off, and the value head shares the policy's backbone. A reward model on the same base is a second adapter. PPO then fits where LoRA fits, not four times over.
  - **GSPO** is an option on the GRPO recipe: it computes the importance ratio and clipping per sequence instead of per token. It's recommended for MoE models, where per-token ratios make RL unstable.
- **LoRA is native to ModelSpec:** any node can be set to freeze, LoRA or train. No separate adapter library is needed.
- **Teach it keeps everything else in place (spike):**
  - training mixes in *anchors*, the base model's own answers to a fixed set of unrelated questions, by default; in the spike this cut drift on unseen prompts by 83–87% with no loss of the new facts;
  - Verify reports drift (KL from the base model on a fixed general prompt set) next to the behavior tests, because keyword checks missed both a style change and an invented fact;
  - measured: ~10 s end to end for 40 examples on the 5080 under an 8 GiB cap, with the chat model resident, peaking at 4.7 GiB.
- **Training memory defaults (spike):** loss computed only at answer positions (full-vocabulary logits everywhere cost gigabytes), LoRA master weights in float32 with bf16 compute, gradient checkpointing on.
- **QLoRA** uses an external 4-bit kernel (bitsandbytes) behind our kernel interface until in-house kernels exist.
- **Hardware-aware defaults:** dtype, quantization, batch size and recipe availability come from the hardware profile plus the designer's memory estimate. A capability matrix enables recipes per backend (e.g. bf16 needs Ampere or newer; QLoRA availability depends on the kernel backend).
- **Preemption-safe by default:**
  - Every recipe has a resume contract: weights, optimizer state, RNG state and dataloader position.
  - Checkpoint frequency follows the risk of interruption.
  - On SIGTERM: checkpoint, then resume on relaunch.
- **Multi-GPU on one machine:**
  - Each GPU is scheduled separately (e.g. chat on GPU 0, training on GPU 1).
  - DDP via `torch.distributed`, and FSDP2 when needed.
  - Multi-node training is deferred.
- **Live dashboard:** a simple view for the common stats, and an advanced view for the nerds. Charts use the in-house renderer.
- **Runs table:** config diff, metric overlay, tags and sweeps. Imports W&B, TensorBoard and Trackio logs.
- **Evaluation:** the Evaluation plugin, below.
- **Export:** GGUF (in-house writer), safetensors, Ollama Modelfile, HF Hub push. Base-model and dataset licenses carry through.
- **Streaming and single-input training** (online learning from data-source plugins) needs a definition before it's built. It ties into the custom input/output work that JEV needs.

**Evaluation (first-party plugin)**

Evaluation tests a model the way a test suite tests code. It's the second first-party plugin after Psychology. It is built only on the public plugin API, and it's the reference implementation of the eval-metric extension point (§13).
- **Model tests:** each test has an input, an expectation and a pass threshold. A suite (`.rrseval`) passes or fails as a whole.
  - `rastai eval test <suite> --model <model or checkpoint>` exits non-zero on failure, so a suite can gate a checkpoint, a Teach it result or a CI job.
  - Any pinned conversation becomes a test (§8), and suites run on every new checkpoint automatically.
  - Results are stored per checkpoint, so a regression shows as a diff between two runs.
- **Metrics.** Each one is a plugin on the same extension point third parties use.
  - **Exact match, keyword and regex checks** are cheap. The spike showed they miss real regressions, so they're never the only check on generated text.
  - **LLM-as-a-judge:**
    - any model reachable through the gateway, local or API, grades answers against a rubric, with a reference answer for factual tests;
    - in pairwise comparisons the judge sees both orders, and a disagreement counts as a tie;
    - before a judge is trusted, it's checked against a few hand-labeled examples and its agreement rate is shown;
    - the judge's prompt and verdict are logged with every result.
  - **Pass@k** for code and agent tasks: n samples per task run in the sandbox manager (§12) and are scored with the unbiased estimator.
  - **Elo ratings** come from pairwise preferences: branch picks, blind A/B in chat, or judge verdicts. They're fit with a Bradley-Terry model, which doesn't depend on the order the comparisons happened in, and shown on the Elo scale with bootstrap confidence intervals.
  - **Perplexity** on held-out text, plus bits per byte, because perplexity can't be compared across tokenizers.
  - **Drift:** KL from the base model on a fixed general prompt set (spike, above).
  - **Reward-model scores**, and Psychology's model diff (§9).
- **Benchmarks:** an in-house runner. Benchmark tasks are imported as data (e.g. lm-eval-harness task definitions), and models and checkpoints are compared side by side.
- **Basic view, "Did it get better?":** pass/fail, the tests whose results changed, and before/after answers side by side.

## 11. Model Designer
A professional architecture tool built on an in-house canvas renderer. Basic users start from an import or a template; professionals edit everything.

**Real models**
- **Import into ModelSpec:**
  - HF `config.json` for the core families: Llama, Qwen, Mistral, Gemma, GPT-2, BERT, ViT;
  - plus the families that need the newer blocks (§17): MoE (Mixtral, Qwen3-MoE, OLMoE, gpt-oss), MLA + MoE (DeepSeek-V2/V3, Kimi), and ALiBi (BLOOM, MPT).
  - `torch.export` graphs.
  - ONNX.
- **Anything unrecognised imports as an opaque node:** shapes known, internals sealed, still observable at its edges. Import never fails outright.
- **Golden test:** import → compile → logits match the HF reference within tolerance, for every supported family (§17).
- **Architecture map:** an imported model is an editable map. Psychology uses it for navigation: click a block to see inside it.
- **Pretrained-block composer:** any supported backbone becomes a node with a freeze / LoRA / train toggle, plus new heads. A pretrained encoder plus a new head is the design that's actually useful to a novice.
- **Live stats on every edit:**
  - params, weights, FLOPs per token;
  - memory for batch × sequence;
  - a fits check per compute target;
  - shape errors;
  - a time estimate per target.
  - It warns when the model is far larger than the dataset.
- In Basic mode, text-generation goals are steered toward fine-tuning, with an explanation of why.

**Pro, from scratch**
- **Templates:** Transformer (decoder, encoder, encoder-decoder), CNN, MLP, RNN/LSTM, ViT, JEPA. TextCNN is ported here, off the deprecated torchtext. JEV comes later.
- **Three separate choices:** input encoders → backbone → output heads. The training objective is chosen separately.
- Hidden layer count, attention per layer, expected input size, and the output head. A custom build-your-own transformer with simple visuals.
- **Block choices** for a transformer:
  - attention: multi-head, grouped-query, sliding-window, or multi-head latent (MLA);
  - position encoding: RoPE, learned, or ALiBi;
  - feed-forward: dense or mixture of experts. MoE sets the expert count, top-k, shared experts and the load-balancing loss, and live stats show total vs active parameters.
  - **Attention residuals (AttnRes), experimental:** each layer attends over earlier layers' outputs instead of adding to one running sum. It's flagged because the logit lens and SAEs assume one residual stream (§9).
- **Multi-input:** drag and resize ranges on a visual input map. Each range gets its own encoder, and a fusion node (concat, cross-attention or late fusion) combines them.
- **Hierarchy and reuse:** define a block once and stamp out N copies; zoom from the whole model to a block to a single op.
- **Automatic observation points** at every node, because the runtime *is* the compiled design.
- **Plugin node types** for custom layers and blocks.
- **Eject:** turns a design into readable PyTorch source plus safetensors, with no rastai import. "Export to PyTorch" means exactly this.
- **Exports:** ONNX (via `torch.onnx`), with native Rust compute considered later.
- Designs are versioned and diffable. Save locally or to a remote server. A play button generates the model, ready for training.
- `pip install rastai` → `rastai.load("x.rrsmodel").to_torch()` returns a plain `nn.Module`, with no app or server.

**Canvas performance targets:**
- 1,000 visible nodes at 60 fps in Chrome and WebKitGTK. *Spike:* met with ~35–40× headroom; both engines hold their refresh cap up to 50,000 nodes. WebKitGTK animates at ~60 Hz even on high-refresh displays.
- Hierarchy collapse keeps big models (80+ blocks) under that limit.

## 12. Agent Gym
An in-house RL engine on the in-house trainer, gateway and sandbox manager.
- **Rollouts** go through the gateway, which captures token IDs and logprobs. Group advantage computation and the GRPO, GSPO or PPO update run on the trainer (§10).
- Agent tasks are scored with the Evaluation plugin's metrics (Pass@k, judges, reward functions), so a harness's reward and its eval share one definition.
- **Task formats:** Harbor task directories are imported as data (Harbor is not a dependency). Rastai has its own task format inside `.rrsharness`.
- **Sandbox manager (in-house):** talks to any Docker-API engine (Docker, Podman) over its HTTP API. It enforces caps, labels containers and cleans up orphans. The prototype is Linux only; microVMs (Firecracker) are deferred.
- **Live peek:** watch an agent's thoughts and actions in real time. Read-only, with a measured overhead budget.
  - **Step mode:** pauses one sandbox and removes it from the batch. Its group is either recomputed without it or marked partial, chosen per recipe, so GRPO advantages stay correct.
- **Sandbox cap per node.** Rollouts and sandboxes run on the node, next to the model being trained. Only traces and rewards stream back.
- **Harness management:** toggle tools on and off. Tools are MCP servers.
- **Harness import/export:**
  - MCP client configs (`.mcp.json`, Claude Desktop, VS Code);
  - MCP Registry `server.json` references;
  - `.mcpb` bundles;
  - Agent Skills folders.
- Remote HTTP MCP servers count as network permissions (§24), not as sandboxed tools.
- Psychology works on agent traces.

## 13. Plugins and Ecosystem
**API contract**
- The plugin API uses semver, versioned separately from the app. The manifest declares `engines.rastai = ">=1.2, <2"`.
- **Capability negotiation:** a manifest lists `requires = ["designer.node@1", "psychology.lens@2"]`. If the host can't provide something, the plugin is disabled with a diagnostic instead of crashing.
- Experimental APIs sit behind a flag, and registry plugins can't use them.
- **Deprecation:** at least one minor release first; the previous major version stays supported for 12 months.
- **Compatibility CI** runs the test suites of the top-N registry plugins on every release candidate (Phase 2).
- First-party sections may use only the public API, enforced in the build.
- The prototype uses the plugin manifest internally. The API is frozen as 1.0 in Phase 2, extracted from the prototype's first-party code.

**Extension points**
- **Declarative (no code):**
  - theme, icon set, workspace layout, keybindings;
  - command-palette actions and macros, locale;
  - templates (design / harness / recipe / project), chat templates, eval suites.
- **Code:**
  - section, panel;
  - importer/exporter (a file type maps to a handler for double-click and drag-drop) and data source (streaming);
  - adapter, compute target, converter, recipe;
  - eval metric (including judges and benchmark loaders), reward function, sampler/logits processor, tokenizer;
  - sandbox image (OCI), designer node type;
  - Psychology lens/visualizer, SAE/dictionary loader;
  - assistant skill, settings page.
- Every contribution registers commands, so every plugin feature becomes an MCP tool and a CLI verb automatically.

**Plugin anatomy (`.rrsplugin`):** a manifest plus any of:
- **Frontend UI:** a sandboxed iframe using our in-house bridge, which is compatible with MCP Apps (`ui://` resources, JSON-RPC over postMessage). Plugins use the shared design-system package, so they look native.
- **Python job module:** runs as its own process under the node, with network off by default.
- **Provider process:** an out-of-process gRPC compute-target provider (§22).
- **WASM module (deferred):** comes when first-party code needs it, on wasmtime with WASI 0.3 component interfaces.

**First-party vs third-party UI**
- Both use the same manifest and the same public API.
- First-party sections run in-process (trusted) for 3D views and per-token streaming.
- Third-party UI runs in iframes.

**SDK: a plugin in an afternoon** (Phase 1 basics, Phase 2 complete)
- `rastai plugin new <template>`, with templates for panel, section, converter, recipe, designer-node, lens, metric, reward, adapter, importer, exporter and theme.
- **Typed SDKs from one source:** `rastai-plugin` (Rust), `@rastai/plugin` (TS) and `rastai_plugin` (Python).
- `rastai plugin dev`: loads the plugin unpacked into a running app, hot-reloads UI, restarts the Python module on save, and opens devtools.
- `rastai plugin test`: a headless host with a mock worker, tiny fixture models and UI snapshot tests.
- `rastai plugin lint | pack | publish`. Plugin errors get `RRS-Pxxxx` codes.
- Each extension point gets a reference page plus a runnable sample of about 50 lines.

**Dependencies**
- Every `.rrs` header can declare `requires: {plugin_id: semver}`.
- A file that needs a missing plugin opens with placeholder nodes (shapes known) and a banner: "Install 3 plugins (permissions: …)". It never fails hard.
- Plugins can depend on other plugins through semver ranges, resolved at install time.
- Python dependencies use a per-plugin `uv` lockfile with hashes, on top of the shared base environment. Installing packages at runtime is forbidden.

**Registry (Phase 2)**
- A git index repo (manifests, hashes, signatures), reviewed by PR. Artifacts live on GitHub Releases or OCI. Users can add their own sources, such as private or company registries.
- Models, datasets and replays live on HF Hub under a `rastai` tag. Rastai doesn't host weights.
- Register Rastai as an HF library ("Use in Rastai" on model pages) and get into HF's "Use this model" list of local apps.
- The in-app listing shows installs, ratings, a "tested on x.y" badge, and the permissions before install.
- **Pull loop:** dropping an unknown file, or opening one with a missing node, suggests the plugins that handle it.
- Each plugin can publish stable, beta and dev channels.

**Provenance and remix**
- Every export embeds a pointer back to its recipe:
  - safetensors `__metadata__`;
  - GGUF key-values;
  - ONNX `metadata_props`;
  - a generated model card: "Made with Rastai, open recipe".
- The gallery shows fork and remix trees. "Remix" forks with attribution filled in automatically.

**Malleability**
- **Macros:** recording UI actions produces a Python or TS script against the SDK client, which can be shared.
- **Embeds:** a read-only `<rastai-viewer>` web component embeds live designs and replays in docs, blogs, READMEs and HF Spaces (Phase 2).

## 14. Import / Export
All importers and exporters are in-house.

| Area | Core (prototype) | Plugin / later |
|---|---|---|
| Conversations | Rastai schema; OpenAI messages JSONL (with tools); ShareGPT; ChatGPT export (branches kept); Claude export | Open WebUI, LM Studio, SillyTavern (and character cards), LibreChat, Gemini/Grok takeouts |
| Models | HF Hub pull/push; safetensors; LoRA adapters (PEFT layout read/write); GGUF export; Ollama Modelfile; ONNX export | MLX, CoreML, ExecuTorch, TensorRT-LLM, AWQ/GPTQ/EXL2 loading, model merging |
| Datasets | HF datasets (including streaming); JSONL, Parquet, CSV; Alpaca and chat formats; DPO pairs; Croissant metadata | WebDataset, Kafka and other streams, SQL/DuckDB, S3, audio/video folders |
| Harnesses | MCP configs, server.json, .mcpb, Agent Skills; Harbor tasks | LangGraph, CrewAI, OpenAI Agents SDK, Inspect, lm-eval-harness tasks |
| Designs | HF config.json, torch.export, ONNX | Keras, JAX/Flax, timm, nanoGPT configs |
| Runs | Trackio / W&B / TensorBoard log import | MLflow, Aim |

**Conversation schema `rastai.conversation/1`**
- A JSONL message tree with branches, tool calls and results, and multimodal parts.
- Optionally carries token IDs, the model hash and the chat template.
- Importers keep branches (ChatGPT's `mapping` tree, Open WebUI's parent links).
- Text-only imports are retokenized and flagged "approximate replay".

The long tail of importers ships as community plugins and doubles as the showcase for the converter template.

## 15. File Formats
| Extension | Holds | Format | Double-click |
|---|---|---|---|
| `.rrsmodel` | ModelSpec design: graph, input map, defaults, objective | JSON | Opens in the Designer |
| `.rrsharness` | System prompt, tool list (MCP refs), environment config, reward, tasks | JSON | Opens in the Harness editor |
| `.rrsdata` | Dataset: sources (path/HF/URL + hash), converter, splits, checks; Croissant-compatible | JSON | Opens in the Data preview |
| `.rrseval` | Eval suite: tests (input, expectation, pass threshold), pinned conversations, judge rubrics, benchmarks, metrics | JSON | Opens in Evaluation |
| `.rrsreplay` | Conversation (`rastai.conversation/1`), token IDs, model hash, settings, branches, annotations, summary layer (always), optional cached activations, hardware fingerprint | Zip | Opens Psychology replay (web viewer if not installed) |
| `.rrsplugin` | Manifest + optional UI and Python modules | Zip | Installs (trust prompt per §24) |
| `.rrsproj` | Designs, harnesses, data, evals, runs, `rastai.lock`; weights referenced by content hash or HF repo, embedded only for offline sharing | Zip or folder | Opens the whole project |

- JSON for every definition file, so the `$schema` key works.
- **Shared header:** `$schema`, `type`, `schema_version`, `created_with`, `id`, `content_hash`, `derived_from: [id@hash]`, `authors`, `license`, `source_url`, `requires`.
- Never use bare `.rrs`; Road Rash saves and R-Studio recovery files already use it.
- **`rastai-formats`** is the single source of truth for every document type. Its JSON Schemas are published at stable URLs (`schemas.<domain>/<type>/<version>.json`) in Phase 2.
- **Standards for everything else:**
  - `.safetensors` (in-house reader/writer) and `.gguf` (in-house writer) for weights;
  - `.jsonl` / `.parquet` for data;
  - `.onnx` for interop.
- Never pickle-based `.pt` files.
- **Checkpoints:** a folder holding safetensors weights, optimizer state as safetensors, and `resume.json` (RNG state, dataloader position, step). Resuming needs no pickle.
- **Hardware fingerprint** in every run record and `.rrsreplay`: environment hash, GPU, driver, dtype, quantization, kernels, seed.

---

# Part II: Technical

## 16. Architecture Overview
**Process layout**
- **One binary, `rastai`:** the CLI plus the daemon (control plane), with the local node embedded.
- On remote machines, `rastai node` runs the same binary as a node only.
- CI also runs the local node over the real network transport, so local and remote never drift apart.

**The pieces:**
- **Daemon (Rust):** API, scheduling, model gateway, storage, the command layer, the plugin host, import/export, ModelSpec analysis.
- **Node (Rust, inside the daemon or standalone):**
  - transport and pairing, file transfers;
  - GPU telemetry and VRAM leases;
  - uv environments;
  - job supervision and the sandbox manager;
  - enforcing limits on the machine.
- **Python jobs:** each job runs as a process in its own process group under the node, talking to it over a local socket.
  - A crash or OOM is reported with its cause.
  - A daemon crash orphans nothing: on restart, the daemon reattaches to running jobs.
- **UI:** a React web app talking to the daemon **only over HTTP** (REST + server-sent events / WebSocket).
  - Tauri is just a shell: the window, file associations and the updater.
  - A plain browser and headless mode are full clients.
  - This also hedges against WebKitGTK on Linux.
- **Headless:** `rastai serve --headless` runs the API and MCP with no UI. On a GPU box, the UI connects over an SSH tunnel.

**Parsing boundary**
- `rastai-formats` parses every `.rrs` file.
- A ModelSpec reaches a worker as validated JSON bytes, read through the same crate's PyO3 wheel (the one `rastai.load()` uses).
- Raw user data (CSV, PDFs, etc.) is parsed by converters inside the job sandbox on the node, next to the data.

## 17. Unified Model Runtime (new)
The in-house policy turns two things into one: the Designer's compiler *is* the Inspectable runtime.

- **One implementation:** every model Rastai inspects or trains runs through a ModelSpec.
  - A designed model is a ModelSpec.
  - An imported HF model is converted to a ModelSpec by the in-house importer, compiled to PyTorch modules by the in-house compiler, and loaded with its safetensors weights.
- **Observation points come for free:** every node in the graph is a hook point. Psychology, steering, ablation and LoRA all attach to nodes, with no third-party hooking library.
- **Model families:** each supported family is a mapping from HF `config.json` to ModelSpec blocks. The core families are Llama, Gemma, Qwen, Mistral, GPT-2, BERT and ViT, followed by the MoE, MLA and ALiBi families (§11).
  - The prototype starts with Gemma 3 and Llama, which the First Run and SAE weights need.
  - Each family must pass the parity suite: logits match the HF reference within tolerance on fixed prompts, in fp32 and bf16.
- **Newer blocks:**
  - **MoE:**
    - the router and each expert are separate nodes, so routing can be observed and edited;
    - the first version loops over the experts that received tokens, and grouped expert matmuls come when measurement shows the loop is the bottleneck;
    - single-token decode gathers the k chosen experts' weights by index, which keeps the step a static shape for CUDA graphs.
  - **MLA:**
    - the cache holds the compressed latent, not full keys and values;
    - decode folds the up-projections into the query and output projections (the "absorbed" form);
    - parity is checked in both the plain and absorbed forms.
  - **ALiBi:** a per-head distance bias passed to attention as a mask. It rules out the fastest SDPA kernel, which only affects the older families that use it.
  - **AttnRes:** a depth-attention node over earlier layers' outputs. It starts as a Designer node, and imported families follow once public weights use it.
- **Models bigger than one GPU:** most real MoE and MLA models don't fit on one consumer GPU.
  - The runtime can place layers across GPUs. For example, DeepSeek-V2-Lite (MLA + MoE, about 31 GB in bf16) fits across the 5080 and the 3090.
  - CI parity uses tiny random-weight configs of each family, as it does for the core families. Real-weight parity runs on OLMoE-1B-7B (one GPU) and DeepSeek-V2-Lite (both GPUs).
- **Spike result:** an in-house Gemma 3 1B is bit-identical to Hugging Face in fp32 with eager attention, within 6×10⁻⁴ with SDPA, and every greedy generation matches.
- **Parity rules (spike):**
  - parity checks skip positions whose next token is forced by the chat template; the model's guess there is noise;
  - exact replays pin the attention kernel (math or eager); fast SDPA kernels sum in a different order in bf16 and can flip near-ties.
- **Performance:**
  - PyTorch SDPA attention by default, with eager attention only where attention maps are captured;
  - decode as a static-shape step captured once as a CUDA graph, with one capture stream per device (the default capture stream lives on whichever GPU was used first, which breaks the second GPU);
  - sliding-window layers keep a ring-buffer cache of their window instead of attending over the whole cache;
  - `torch.compile` and fused kernels where measurement shows decode is launch-bound (the spike's runtime reaches ~155 tok/s against llama.cpp's 311 for the same model);
  - KV cache for generation;
  - gradient checkpointing for training.
  - In-house kernels (Triton) come later, where measurement shows they're needed.
- **Unsupported architectures:** they import as opaque nodes. An optional compat runtime uses HF `transformers`, so they can still chat and train, with Psychology limited to the edges of the opaque node. `transformers` is otherwise only a test-time reference for parity.
- **Fast runtime:** external engines behind the adapter interface (`llama-server`, optional vLLM). They are separate processes, not linked libraries, so they can be swapped. An in-house fast runtime is a long-term option.
- **Trainer:** in-house loops on the same compiled modules.
  - LoRA and freezing are node attributes.
  - Optimizers and distributed training come from PyTorch.
  - Recipes are plugins on the trainer.

## 18. Gateway and Tokenization
- **In-house gateway (Rust, axum):**
  - OpenAI- and Anthropic-compatible endpoints;
  - routes to adapters;
  - logs token IDs, the exact model version and sampling settings;
  - serves steering presets.
- **The gateway owns chat templates and tokenization:**
  - It uses the HF `tokenizers` crate, a compatibility surface that has to match every model exactly.
  - It renders templates with `minijinja`, because HF chat templates are Jinja.
  - It sends token IDs, not text, to every runtime that accepts them. All logged token IDs come from one source, which makes token parity between replay twins checkable.
- **Third-party API adapters (in-house):** OpenAI, Anthropic, and any OpenAI-compatible endpoint. They log text plus any logprobs returned.
- **Incognito and retention** are enforced at the gateway.

## 19. Storage and Durability (new)
**Workers own their logs**
- Each job is the only writer of `runs/<id>/events.log`: append-only, with a sequence number and checksum on every record.
- The node tails that log and resumes from the last sequence number the daemon acknowledged.
- Event classes:
  - **durable:** metrics, state changes, checkpoints;
  - **ephemeral:** per-token streams, droppable.
- Large payloads go to the blob store; events carry only their hash.

**Stores**
- **`rastai.db` (SQLite, WAL mode):** runs, metrics, nodes, the artifact catalog, lineage, the command journal and settings. It's an index, rebuildable from the event logs plus the blob store.
- **`vault.db` (SQLite):** conversations (as a tree of message rows) and gateway logs. It's a separate file so it can be switched to SQLCipher encryption in Phase 2 with the same schema and no migration.
- **Blob store:** a folder of files named by their BLAKE3 hash, holding artifacts, replays, checkpoints and dataset snapshots.
- **Model weights** stay in the user's existing Hugging Face cache, never copied.
- **No Postgres.** One user for now; the principal on every command (§21) keeps team mode possible.

**Crash and disk safety**
- kill -9 tests run in CI for the daemon, node and jobs.
- Disk quotas and garbage collection for blobs, checkpoints and logs, with a "pinned" flag.
- Free space is checked before downloads and training.

## 20. Schemas, Protocols and Versioning
**One owner per schema; no type is defined twice**
- **Wire (between processes):** protobuf, managed with buf (prost + tonic in Rust, connect-es in the browser, grpcio in Python).
- **Documents and command arguments:** the `rastai-formats` Rust types. JSON Schema, TypeScript types and Python (pydantic) types are generated from them.
- Golden test messages are checked in and round-tripped in every language, and `buf breaking` runs in CI.

**Protocols**
- **Node protocol:** daemon ↔ node, over a Unix socket / named pipe locally, and SSH (later QUIC) remotely.
- **Worker protocol:** node ↔ Python job, over a local socket.
- **Event log format:** versioned records with sequence numbers and checksums.

**Versioning**
- The app, node and worker ship together under one version.
- `rastai-formats` has its own semver, and so does the plugin API.
- The node protocol has an integer version plus capability flags, negotiated in the handshake.
- A compatibility matrix lists app ↔ protocol ↔ worker ↔ environment lock ↔ plugin API.

## 21. Command Layer and Local API Security
**Commands**
- Every action is a command with an id, a typed schema, a permission scope and a principal (the acting user).
- One generic endpoint, `Execute(id, json)`, plus `ListCommands`.
- The UI, command palette, keybindings, menus, CLI verbs, MCP tools and "Copy as" are all generated from the command registry.
- Undo covers document edits only (designs, harnesses, datasets), not side effects like training runs.
- The MCP server and client are in-house: JSON-RPC, generated from the registry.

**Local API security**
- Listen on loopback plus a Unix socket / named pipe.
- The access token lives in a file only the user can read (mode 0600).
- Host and Origin headers are checked to block DNS-rebinding attacks.
- Navi and MCP clients get scoped tokens (for example "chat only").
- The remote UI goes through an SSH tunnel; no port is ever opened to the network by default.

## 22. Compute, Nodes and Data
**Hardware**
- Detect GPU, VRAM, driver, topology, CPU, RAM and disk (and, later, the Apple chip and unified memory). Then run a 20 s self-test with `rastai doctor`.
- **The self-test checks numerical correctness** (matmuls and a tiny forward pass against a CPU reference), not just that kernels run. Never enable ROCm through `HSA_OVERRIDE_GFX_VERSION` on unsupported chips: on the owner's integrated AMD graphics it ran and returned silently wrong results (spike).
- The PyTorch build is picked automatically:
  - CUDA 13.x by default (needs driver 580+);
  - the legacy cu126 build for Maxwell/Pascal/Volta;
  - ROCm per GPU architecture;
  - CPU fallback;
  - MPS later.
- Watch for Blackwell (sm_120, the RTX 5080's architecture) gaps in external kernels.

**VRAM leases (in the node)**
- Interactive models (chat, Psychology) hold evictable leases; training jobs hold non-evictable ones.
- Memory limits are enforced per process (`set_per_process_memory_fraction`).
- The node arbitrates between the Fast runtime, the Inspectable runtime and training.
- Rastai manages its own `llama-server`; Ollama is only an external adapter, because its own scheduler would fight the leases.

**Compute targets (ranked)**
1. **Local:** Linux NVIDIA, Linux AMD, CPU. *(Prototype.)*
2. **Bring your own box over SSH** + headless `rastai serve`. Covers Lambda, Vast, RunPod pods, home labs and university machines with no provider-specific code. *(Prototype.)*
3. **Pairing codes over QUIC (iroh 1.x):** NAT traversal and QR pairing. *(Phase 2.)*
4. **RunPod native** (REST v2), with live prices, spot instances and cost tooling. *(Phase 3.)*
5. **SkyPilot provider plugin:** AWS, GCP, Azure, Lambda, Nebius, CoreWeave, Kubernetes, Slurm. SkyPilot is an orchestrator, a layer above providers. *(Phase 3.)*
6. **Modal:** serverless bursts for evals and gym sandboxes; its 24 h cap per attempt makes long training awkward. *(Phase 3.)*

**Provider plugins (Phase 3)**
- A target only provisions a machine and starts `rastai node` on it.
- Provider plugins are separate processes speaking gRPC, like Terraform providers: `offers`, `provision`, `status`, `stop`, `terminate`, `list_billable`.
- Credentials come from the OS keychain on each call.

**Pairing and identity**
- Each node generates an Ed25519 identity on first run; its node ID is its public key.
- **SSH (prototype):** `rastai node pair --ssh user@host` installs the binary, exchanges keys and starts the node. Both sides pin each other's keys.
- **Pairing codes (Phase 2):** `rastai node pair <code>` or a QR scan.
  - The code holds the daemon's ID, the relay address and a one-time secret, and expires after 10 minutes.
  - The relay can be self-hosted.
- Nodes always dial out. They send a heartbeat every 5 s with telemetry and job states, and three missed heartbeats mark a node unreachable.
- Signed self-updates; the daemon pushes the matching binary to paired nodes.

**Data and artifacts**
- **Prototype:** whole-file BLAKE3 hashes with resumable, verified transfers. Chunked, deduplicated transfers come in Phase 2.
- **Fetch from the source; upload only what exists nowhere else.**
  - Nodes pull HF weights and data directly.
  - Only private local data is uploaded from the laptop.
  - A cache persists per node, so the second run starts warm.
- Converters run on the node, next to the data.
- **Artifact catalog:** records each artifact's home and its copies.
  - LoRA adapters always sync back.
  - Full checkpoints stay remote and are fetched on demand.
  - Teardown wipes unpinned remote data.

**Cost safety (Phase 3, with providers)**
- Time and cost quote before launch:
  - time ≈ FLOPs ÷ (GPUs × peak FLOPS × MFU), where MFU is the fraction of peak throughput the run actually reaches;
  - a 60 s probe run corrects the quote.
- Spending caps per run, day and month, enforced on the node.
- Idle machines stop after 15 minutes.
- An orphan sweeper lists every billable resource.
- Rastai can't control billing on a bring-your-own box, so these features arrive with provider integrations.

**Offline / air-gapped (Phase 2)**
- An offline bundle: node binary, wheels per backend, model weights, SAEs and labels.
- `HF_HUB_OFFLINE=1`, the registry mirrored as a folder, LAN only.
- A CI test checks for zero outbound connections.

## 23. Validation
- **Parse, don't validate:** files become strongly typed Rust structs at the boundary, and internal code never re-checks.
- **One crate** (`rastai-formats`) does all reading and writing. The backend, the frontend (WASM), the CLI (`rastai validate`) and the PyPI wheel all reuse it.
- **Four layers:**
  1. Syntax: line and column.
  2. Structure: Rust types and generated JSON Schema, with exact paths.
  3. Meaning: shape inference, references resolve, ranges don't overlap, the graph is acyclic unless recurrent.
  4. Environment: plugins installed, fits on the GPU, adapter supports it. These are warnings, not failures.
- **Compiler-style diagnostics:** all errors are collected in one pass. Each has a code (`RRS-Exxxx` for formats, `RRS-Pxxxx` for plugins), a path, a severity and a suggested fix, shown by an in-house diagnostics renderer.
  - Basic view: plain English plus a fix button. Advanced view: the full diagnostic.
  - One catalog serves the UI, the docs and plugin authors.
- **Migrations:** `schema_version` drives an upgrade chain on load, so old files always open. Files from a newer version open read-only.
- **Untrusted imports:**
  - limits on size, depth, node count and parameter count, checked before allocating;
  - zip-bomb caps;
  - zip-slip paths (`..` in archive entries) rejected;
  - no remote code execution from model repos: in-house runtime, and `trust_remote_code` is off in the compat runtime.
- **Strict safetensors contract (spike):** no duplicate JSON keys; integers must be real integers (booleans and floats rejected); byte sizes computed with overflow checks (a shape of [2³², 2³²] must not wrap to 0); tensors must tile the data section exactly, with no overlaps, gaps or trailing bytes; metadata maps strings to strings. Writing this corpus found four gaps in the spike's own first reader.
- **Testing:** fuzz the parsers (cargo-fuzz), round-trip property tests, and a golden corpus of broken files with expected error codes. The spike's 33-file safetensors corpus (6 valid, 27 malformed) is its seed.
- **Beyond files:** typed errors in libraries, catch-all errors only at the edges, and every user-facing error mapped to a catalog code.

## 24. Trust and Privacy
- No telemetry and no account. Crash reports go out only if the user opts in.
- A visible badge shows whenever data leaves the machine.
- **Gateway logs:** retention setting, incognito chats, and PII/secret redaction on dataset export. Encryption at rest arrives in Phase 2 (`vault.db` → SQLCipher).
- **Secrets** live in the OS keychain and never go into `.rrsproj`. Headless Linux has no keychain service, so it falls back to the kernel keyring or an encrypted file.
- Cloud credentials never leave the daemon. Nodes get short-lived scoped tokens.
- **Data policy per project:** `local-only`, `trusted-remote` or `any`.
- **Fine-grained plugin permissions:**
  - `network:[hosts]`, `fs:project|ro`, `secrets`;
  - `gpu`, `weights.read`, `conversations.read` (sensitive);
  - `gateway.spend`, `sandbox.spawn`, `commands:[scopes]`.
- Any network or data permission triggers the trust prompt, even for a declarative plugin.
- **Enforcement:** Python plugin modules run as separate processes with network off by default. They are still trusted code, and the trust prompt says so.
- **Phase 2:** Sigstore signing, a verified-publisher badge, scanning of every version, and a signed revocation list. An update that adds permissions asks again.
- **We own our security now:** in-house protocols (gateway, MCP, node) get a threat model, fuzzing and review. Crypto, TLS/QUIC and SSH are never written in-house.

## 25. Observability, Updates and Maintenance (new)
- **Logs:** structured JSON logs (Rust `tracing`), with run IDs carried into Python jobs.
- `rastai doctor --bundle` packs logs, the hardware profile and versions into one file for bug reports.
- **Self-update:**
  - Linux packages: AppImage and `.deb` in the prototype, through the Tauri updater.
  - Remote nodes get the matching binary pushed by the daemon.
- **Model downloads:** gated HF models (token, license acceptance), resumable downloads, and disk checks.
- **Disk:** quotas, garbage collection, pinned items, and a storage view in settings.

## 26. Code and Reproducibility
- **Escape hatches in both directions:**
  - `rastai.observe(model)` gives any PyTorch model in a notebook the Psychology views. Models that ModelSpec can import get the full views; others get edge views.
  - `rastai.log()` is wandb-compatible.
- Any run opens as a notebook or script. Eject turns designs into plain PyTorch.
- **Run record:** job spec + dataset hash + base-model hash + plugin versions + environment lock hash + seed + hardware fingerprint + git commit. `rastai rerun <id>` reproduces it.
- **Honest reproducibility:** a rerun or replay on different hardware matches within a stated tolerance, not bit for bit. A deterministic mode is offered where the stack supports it.
- **Lineage panel** on every object: replay → pairs → dataset → run → checkpoint → diff.

## 27. Build-In-House Policy and Dependency Register (new)
**Policy:** build in-house by default. External code is allowed only in five categories, and always behind our own interface so it can be replaced:
1. **Foundations** that are impractical to rebuild: language runtimes, PyTorch and its GPU backends, the browser engine, the OS, the container engine.
2. **Security-critical** code where writing our own is dangerous: crypto, TLS/QUIC, SSH, database encryption, sandboxing runtimes.
3. **Byte-exact compatibility surfaces** that must match an ecosystem exactly: tokenizers, Jinja chat templates, protobuf.
4. **External engines:** whole processes behind the adapter interface, optional and swappable (`llama-server`, vLLM).
5. **Dev tooling** that never ships in the product: linters, test runners, release tools.

Thin utility libraries are written in-house when it's cheap to do so. Every external dependency is listed here, with a reason and a replacement path. CI enforces the list (cargo-deny, a license allowlist, hash-pinned lockfiles).

**In-house (ours)**

| Component | Notes |
|---|---|
| Gateway / router | Rust, axum. Token logging, adapters, steering presets |
| Command layer, MCP server and client | JSON-RPC, generated from the command registry |
| ModelSpec compiler + model families | The Inspectable runtime (§17), including MoE, MLA, ALiBi and AttnRes blocks. Parity-tested against HF |
| Trainer + recipes | Loops, LoRA/QLoRA wiring, SFT, DPO, GRPO, GSPO, reward model, PPO, classification, pretraining, SAE training, JEPA |
| Interpretability | Observation points, logit lens, attention capture, SAE runtime, probes, steering, ablation, model diff, SAE fit check, attribution graphs, MoE routing lens |
| RL engine | Rollout manager, group advantages, GRPO / GSPO / PPO; Harbor task importer |
| Sandbox manager | A client for the Docker HTTP API: caps, labels, orphan cleanup |
| Experiment tracking | Runs table, metrics store, lineage, log importers |
| Dataset tooling | Task detection, preview, MinHash dedupe, leakage, PII flags, converters |
| Evaluation plugin | Model tests, metrics (judges, Pass@k, Bradley-Terry/Elo, perplexity, drift), benchmark task import |
| Designer canvas | WebGL2 / Canvas2D renderer for graphs |
| Visualization | Charts, vector-space and 3D data-flow renderer (WebGL2; WebGPU optional) |
| Design system | `packages/ui` |
| Plugin host | Manifest, permissions, iframe bridge (compatible with MCP Apps) |
| Formats and I/O | `rastai-formats`; safetensors reader/writer; GGUF writer; conversation and design importers |
| Diagnostics renderer | Compiler-style error output |
| Transfers | Whole-file BLAKE3 + resumable transfer protocol |
| Installer, `rastai doctor`, hardware detection | Reads nvidia-smi, rocm-smi and sysfs |

**External (allowlist)**

| Category | Dependency | Why | Replacement path |
|---|---|---|---|
| Foundation | Rust toolchain, tokio, axum, serde | Language runtime and HTTP | — |
| Foundation | Python 3.12, PyTorch (CUDA / ROCm / CPU; MPS later), `torch.distributed`, `torch.onnx` | GPU math, autograd, kernels | Native Rust compute (Burn / tch-rs) is a long-term option |
| Foundation | uv | Python environments and lockfiles; packaging is a swamp | — |
| Foundation | React + TypeScript, Tauri 2 | UI framework and desktop shell | The browser is already a full client |
| Foundation | SQLite (rusqlite) | Embedded database | — |
| Foundation | Docker / Podman engine, OpenSSH | Containers and remote access | — |
| Security | blake3, ed25519-dalek, rustls, OS keychain bindings | Crypto, identity, TLS | — |
| Security (later) | SQLCipher, iroh / quinn, wasmtime, Sigstore | Encryption at rest, QUIC/NAT traversal, WASM sandbox, signing | — |
| Compatibility | HF `tokenizers` | Exact token IDs for every model | — |
| Compatibility | minijinja | HF chat templates are Jinja | — |
| Compatibility | protobuf: prost / tonic, connect-es, grpcio | The wire format | — |
| Engine | llama.cpp `llama-server` | Fast runtime on every backend. Official CUDA 13 Linux builds ship their runtime libraries, so no CUDA toolkit is needed; pin a build and check its SHA-256 | An in-house fast runtime (long term) |
| Engine (optional) | vLLM | High-throughput serving on Linux nodes | — |
| Engine (optional) | HF `transformers` | Compat runtime for unsupported architectures; parity reference in tests | Add the architecture to ModelSpec |
| Kernel (optional) | bitsandbytes | 4-bit kernels for QLoRA | In-house Triton kernels |
| Dev tooling | buf, cargo-deny, cargo-fuzz, Playwright, maturin, dist, release-please, just | Build, test, release | — |
| Data (not code) | Gemma Scope 2, Llama Scope SAE weights; Neuronpedia labels | Pretrained dictionaries and labels | Train our own with the SAE recipe |
| Service (Phase 3) | Payment provider / merchant of record (e.g. Paddle or Stripe) | Payments and global tax are compliance-critical; never built in-house | Another provider |

**Dropped (prior art, not dependencies):** Unsloth, TRL, PEFT, Accelerate, nnsight, TransformerLens, SAELens, circuit-tracer, rLLM, Harbor (its task format is imported), React Flow, three.js, deck.gl, uPlot, miette, Extism, Trackio (its logs are imported). Never: LiteLLM, which was compromised on PyPI in March 2026.

**The cost, stated honestly:** in-house adds months to the CTO's 9–12 month estimate for the wrapped stack. The model families, trainer, interpretability stack, RL engine and canvas are each multi-week efforts. The payoff: one runtime where everything is observable, no upstream churn, no license traps, and depth nobody can copy by wrapping the same libraries. AI agents plus parity tests make the extra work tractable.

## 28. Risk Register
| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Scope outruns a single human reviewer | High | High | L0–L3 bar; at most 2 work tracks merging at once; a demo every week; contracts land first |
| 2 | In-house model families have parity bugs or lag behind new architectures | High | High | A parity suite per family (fp32 + bf16); opaque node + compat runtime fallback; start with 2 families |
| 3 | In-house trainer is slower or hungrier for VRAM than Unsloth | Medium | Medium | SDPA, `torch.compile`, gradient checkpointing, published benchmarks; Triton kernels where measured |
| 4 | Live Psychology slows generation | High | Medium | Batched unembedding, attention for the last token only, top-k on the GPU; overhead shown in the UI |
| 5 | Chat, training and Psychology compete for an 8–16 GB GPU | Medium | High | VRAM leases, eviction, memory fractions, clear OOM diagnostics |
| 6 | ROCm gaps (kernels, 4-bit support) | Medium | Medium | AMD GPU as a tier-1 CI target; QLoRA falls back to LoRA; capability matrix |
| 7 | Blackwell (sm_120) gaps in external kernels | Medium | Medium | The 3090 as a fallback test GPU; minimal external kernels |
| 8 | Schemas drift between Rust, Python and TS | Medium | High | One owner per schema; codegen; `buf breaking`; golden messages in every language |
| 9 | Durability bugs: orphaned workers, torn logs, full disks | Medium | High | Checksummed logs, process groups, quotas, kill -9 tests |
| 10 | Security bugs in in-house protocols (gateway auth, MCP, node) | Medium | High | Threat model, fuzzing, scoped tokens, loopback only; never in-house crypto |
| 11 | Version pins conflict (torch, vLLM, bitsandbytes) across backends | Medium | Medium | Fewer dependencies now; one environment per backend; CI resolves every lockfile |
| 12 | Licenses (Gemma/Llama model terms, SAE weights) and supply chain | Medium | High | License display and checks; hash-pinned lockfiles; no pip installs at runtime; Trusted Publishing |
| 13 | Tier-2 platforms rot while unsupported | Medium | Medium | The portability design rules (§4); CPU tests on macOS and Windows CI runners from day one |
| 14 | Real MoE and MLA models are bigger than the GPUs on hand | High | Medium | Tiny random-weight configs for parity in CI; layer placement across both GPUs; quantized Fast runtime for chat |
| 15 | LLM judges are biased or wrong, so a passing suite gives false confidence | Medium | High | Both-order pairwise judging; agreement with hand labels shown before a judge is trusted; prompts and verdicts logged; drift and model diff always run alongside |

## 29. Repo and Engineering Setup
**Monorepo layout (everything runs through `just`)**
```
proto/rastai/{node,worker,events,commands}/v1
crates/
  rastai-formats      # documents, schemas, validation; WASM + PyO3 builds
  rastai-core         # command registry, domain types
  rastai-store        # rastai.db, vault.db, blob store
  rastai-node         # node agent: jobs, envs, VRAM leases, telemetry
  rastai-sandbox      # Docker HTTP API client, caps, cleanup
  rastai-gateway      # endpoints, adapters, tokenization, logging
  rastai-mcp          # MCP server/client (JSON-RPC)
  rastai-transport    # local IPC, SSH, (later) QUIC
  rastai-diag         # diagnostics catalog + renderer
  rastai-server       # daemon: HTTP API, scheduling, plugin host
  rastai-cli          # the `rastai` binary
python/
  rastai              # SDK: load(), observe(), log()
  rastai_worker       # job runtime, worker protocol
  rastai_models       # ModelSpec compiler + model families
  rastai_train        # trainer, LoRA, recipes
  rastai_lens         # interpretability
  rastai_rl           # RL engine
  rastai_eval         # Evaluation plugin: model tests, metrics, judges
  rastai_data         # dataset tooling, converters
  envs/<backend>/uv.lock
apps/
  web                 # React app
  desktop             # Tauri shell
packages/
  ui                  # design system
  client              # generated API client
  canvas              # designer renderer
  viz                 # charts, vector space, 3D
schemas/  catalog/  fixtures/  docs/adr/  docs/spec/
tests/{parity,e2e,gpu,crash}
```

**CI**
- Linters, `buf breaking`, schema snapshot checks, cargo-deny (licenses + advisories).
- CPU tests on Linux, plus CPU tests on macOS and Windows runners to keep tier 2 honest.
- **A full CPU loop in under 10 minutes,** using TextCNN and tiny random-weight HF configs:
  - design → train → eval → ONNX export → re-import, and the outputs must match;
  - this needs `train_model.py` ported off torchtext and pickled `.pt` files.
- Model-family parity tests against the HF reference.
- Playwright end-to-end tests against the web build.
- kill -9 crash tests.
- **GPU tests:** run nightly on a self-hosted runner on the owner's box (5080, 3090, AMD), or by hand if the runner isn't set up.

**Making AI coding agents productive**
- A root `CLAUDE.md` holds the architecture map and the hard rules:
  - no pickle;
  - no pip installs at runtime;
  - no type defined twice;
  - in-house by default, and new dependencies need an entry in §27;
  - every user-facing error gets a catalog code.
- The `CLAUDE.md` also holds the owner's code style:
  - constants at the top of each file;
  - human-readable names (`embedding_size`, not `n_embd`);
  - comments that explain why, not restate syntax;
  - small functions;
  - a thin `main` that is mostly a sequence of calls;
  - no clever one-liners.
- A `CLAUDE.md` in each crate and package with its local rules.
- Contract changes (proto, schemas, commands) land in their own PR first, before any code that uses them.
- One git worktree per work track.
- "Done" means `just check` passes and a test was added.

## 30. ADRs to Write Now
1. **Process layout:** one binary, an embedded local node, jobs in their own process groups.
2. **Durability:** workers own their event logs; the database is an index; events are durable or ephemeral.
3. **Storage:** SQLite (WAL) for `rastai.db`; a separate `vault.db` (plain now, SQLCipher in Phase 2); a BLAKE3-named blob store; the HF cache is never copied.
4. **Schema ownership:** protobuf for the wire, `rastai-formats` for documents; everything else is generated.
5. **Commands:** one generic `Execute` endpoint; undo for document edits only; a principal on every call; MCP generated from the registry.
6. **UI transport:** HTTP only; Tauri is a shell; the browser is a full client.
7. **Local API security:** loopback plus socket, a 0600 token file, Host/Origin checks, scoped tokens.
8. **Tokenization:** owned by the gateway (HF `tokenizers` + minijinja); token IDs are sent to runtimes.
9. **Python environments:** uv, Python 3.12, one lockfile per backend.
10. **GPU sharing:** VRAM leases in the node; Rastai manages its own `llama-server`.
11. **Remote transport:** OpenSSH first, iroh 1.x pairing in Phase 2.
12. **Build-in-house policy:** the five allowed categories, the dependency register, CI enforcement.
13. **Unified model runtime:** ModelSpec is the Inspectable runtime; parity suite; opaque-node + compat fallback.
14. **Platform tiers:** Linux NVIDIA + AMD + CPU at tier 1; macOS and Windows at tier 2 with portability rules.
15. **Plugins and licensing:** WASM tier deferred; Apache-2.0 + DCO (decided, §0.12); dependency license allowlist.

---

# Part III: Plan

## 31. Build Order
**Phase 1: Prototype (the whole suite, every section at L1, the core loop at L2)**
0. **Spike** (§32, weeks 1–2).
1. **Foundation:**
   - `rastai-formats`, protobuf, the command layer + CLI + MCP;
   - storage and durability;
   - `rastai` daemon + embedded node;
   - installer + `rastai doctor`;
   - the plugin host and manifest (first-party sections load through it from day one).
2. **Unified runtime:** the ModelSpec compiler, Gemma 3 + Llama families with parity tests, the uv environments.
3. **Inference:**
   - gateway + tokenization;
   - Fast (`llama-server`) and Inspectable runtimes;
   - OpenAI/Anthropic-compatible endpoints (Navi);
   - the conversation tree;
   - archive / star / pin, incognito.
4. **Psychology:**
   - replay → basic lenses → live panel → steering → branching;
   - edits become data;
   - model diff and SAE fit check;
   - advanced views (projections, probes, attribution graphs, 3D data flow);
   - replay summary layer and web viewer.
5. **Training and Evaluation:**
   - data import with checks, converters;
   - the trainer and recipes (classification, SFT, LoRA/QLoRA, DPO, GRPO, GSPO, reward model, PPO, SAE, pretraining), with RLHF as one chain;
   - dashboard, runs table;
   - the Evaluation plugin: model tests and `rastai eval test`, judges, Pass@k, Elo, perplexity, drift, benchmarks;
   - export (GGUF / safetensors / Ollama / HF).
6. **Designer:**
   - the canvas;
   - importers (HF config.json, torch.export, ONNX) with opaque nodes;
   - the architecture map and the pretrained-block composer;
   - from-scratch templates (TextCNN), multi-input, hierarchy;
   - live stats, Eject, ONNX export, `rastai.load()`;
   - more model families (Qwen, Mistral, GPT-2, BERT, ViT);
   - MoE, MLA and ALiBi blocks and their families, the MoE routing lens, layer placement across GPUs, and the experimental AttnRes node.
7. **Agent Gym:**
   - the RL engine (GRPO, GSPO, PPO), sandbox manager (Linux), Harbor task import;
   - Pass@k and judge scoring for agent tasks, shared with Evaluation;
   - the harness editor with MCP import/export;
   - live peek and step mode;
   - Psychology on agent traces.
8. **Plugins:** SDK basics + `rastai plugin new | dev | test | pack`, local install of `.rrsplugin`, "install missing".
9. **Remote:** SSH pairing, headless `rastai serve`, resumable transfers, remote jobs that survive network drops, preemption-safe resume.

**Prototype done when:** every section is at L1, the core loop is at L2, and it all runs on Linux NVIDIA, Linux AMD and a paired SSH box.

**Phase 2: Harden, open the ecosystem, everyone-ready**
- Basic mode and novice First Run polish, guided tutorials, accessibility pass (the L3 work).
- Plugin API 1.0 extracted from the prototype's first-party code; compatibility CI; the git-index registry; Sigstore signing; the revocation list.
- Publish `rastai-formats` and the SDKs to crates.io, PyPI and npm; schemas at stable URLs; HF library registration; `<rastai-viewer>` embeds.
- iroh pairing codes and QR; chunked, deduplicated transfers; the offline bundle.
- Encryption at rest (SQLCipher); macOS and Windows builds as hardware allows; code signing.
- The WASM plugin tier if first-party code needs it.

**Phase 3: Rented cloud and scale**
- RunPod native; cost/time quotes; spending caps, idle stop, orphan sweeper.
- SkyPilot provider plugin, then Modal.
- Multi-node training; in-house kernels where measured; team mode.

## 32. First 90 Days
The weeks are estimates; the exit criteria decide when a milestone is done.

**Weeks 1–2: Spike (done 2026-10-10; results in `spike/README.md`)**
- Build an in-house Gemma 3 1B from `config.json`; its logits must match the HF reference.
- Live lenses on the 5080 (capped at 8 GB), the 3090 and the AMD GPU: measure tokens/s and VRAM with the panel on and off.
- Check that llama.cpp and the gateway's tokenizer produce identical token IDs.
- Train an in-house LoRA in under 3 minutes while the chat model stays loaded.
- Time uv environment installs from scratch, for CUDA and ROCm.
- Canvas prototype: 1,000 nodes plus a 3D scene, in WebKitGTK vs Chrome.
- *Exit:* a report with the numbers, draft ADRs, TextCNN ported off torchtext and `.pt`.

**Weeks 3–4: Skeleton**
- The monorepo and CI.
- The daemon with SQLite and 3 commands, reachable over HTTP, the CLI and the in-house MCP.
- Worker protocol v0 running TextCNN training.
- *Exit:* kill -9 the daemon mid-run; it restarts and reattaches with no lost events.

**Weeks 5–6: Unified runtime**
- The ModelSpec compiler; the Gemma 3 + Llama families with parity tests in CI.
- uv environments per backend, `rastai doctor`, model downloads (including gated ones).
- *Exit:* both families pass parity in fp32 and bf16 on NVIDIA and AMD.

**Weeks 7–8: Inference**
- Gateway + tokenization; Fast + Inspectable runtimes; VRAM leases.
- The conversation tree; the endpoints.
- *Exit:* Navi chats through `/v1`, with token-ID logging and the runtime toggle working.

**Weeks 9–10: Thin Psychology**
- Entropy, logit lens, SAE features on 3 layers (in-house SAE runtime).
- One steering slider; replay.
- *Exit:* minutes 3–7 of the First Run work on an 8 GB GPU budget.

**Weeks 11–12: The loop, plus remote**
- Branch → DPO pair → in-house LoRA training → behavior test → before/after view.
- Evaluation plugin v0: model tests with `rastai eval test`, an LLM judge, and drift.
- `rastai node` over SSH.
- *Exit:*
  - the full loop runs in under 10 minutes from a cold start;
  - the eval suite catches an invented fact that keyword checks miss (the spike's failure case);
  - a remote job survives a network drop.

**Week 13: Thin Designer, Linux packaging and the next plan**
- The in-house canvas with the TextCNN template, HF config.json import and live stats.
- Compile → train from the UI.
- AppImage and `.deb` builds.
- *Exit:* design → train → ONNX export works from the UI, plus a written plan for months 4 onward.

## 33. Deferred Past the Prototype
- Everything in Phases 2 and 3 (§31).
- Hosted registry with reviews and comments; any paid marketplace.
- Two-way MCP Apps (Rastai panels running inside other hosts).
- Keras/JAX importers; parsing PyTorch from source.
- MLX, CoreML and TensorRT exporters; model merging.
- Native Rust compute.
- JEV template (after custom data and input/output are finished).
- Streaming and single-input training (needs a definition).
- MicroVM sandboxes (Firecracker); gym training on macOS and Windows.
- **Compute:**
  - multi-node training;
  - native Kubernetes / Slurm / Vast / Lambda / CoreWeave adapters (use SkyPilot or SSH instead);
  - failover across providers, spot bidding, autoscaled serving;
  - streaming full tensors over the internet;
  - Intel GPUs;
  - aggregators.
- Team mode (the principal field is already in place).

## 34. Open Questions
- **Business (Phase 2–3, §36):**
  - Legal entity and jurisdiction.
  - Bootstrap or raise venture funding.
  - Validate the pricing hypotheses with design partners.
  - Payment provider (merchant of record vs Stripe).
  - The marketplace fee within the 15–20% range.
- **Name follow-ups:**
  - Run a formal trademark search (USPTO, EUIPO, WIPO Global Brand Database) for "Rastai" in software/AI. The main conflict is RastAI (rastai.ai), a Persian AI learning platform.
  - Claim the names before announcing:
    - PyPI `rastai`;
    - crates.io `rastai`, `rastai-formats`, `rastai-node`;
    - the npm `@rastai` org and the HF org `rastai`;
    - a GitHub org (e.g. `rastai-research`, since user `rastai` is taken);
    - a domain (rastai.dev or rastairesearch.com appeared unregistered).
  - Publish small working packages rather than empty placeholders; PyPI's naming policy discourages squatting.
- **AMD GPU:** get a discrete RDNA3+ card; its model decides the ROCm target architecture and whether 4-bit kernels are available.
- **Live overhead budget:** the spike measured +16–31% for the full panel. *Proposed:* at most +35% at CUDA-graph speed, shown in the UI.
- **Verify before building on these** (flagged by the compute review): vLLM batch invariance's minimum GPU; the SkyPilot v0.13 release; the bandwidth and cost rules of thumb; RunPod field names (Phase 3).

## 35. Licensing and Community
- **Decided:** Apache-2.0 for the app, formats and SDKs.
  - Many companies ban AGPL, which is what Unsloth Studio's UI and Transformer Lab use, so a permissive license is an advantage.
  - A license won't stop a clean-room clone anyway; depth and ecosystem will.
- A trademark policy protects the name. Contributions use a DCO, not a CLA. Plugins can use any license.
- The format spec lives in its own repo under CC-BY with a public RFC process. The spec is the moat.
- **Contributor path:** converters (one file) → lenses → recipes → model families. A "paper-to-lens" program lets interpretability papers ship as Rastai lenses.

## 36. Business Model (new)
**One line:** free for everyone; businesses pay for working together, governing it, and having it run for them. Creators earn through the marketplace.

**The free-forever promise** (published publicly)
- The whole app, every section and every feature, is free for individuals and companies alike, including commercial use. That covers:
  - Designer, Inference, Psychology, Training, Gym and Plugins;
  - multi-GPU training and remote nodes over SSH;
  - headless mode, public sharing, and free marketplace items.
- **No feature an individual uses on their own hardware will ever move behind a paywall.** Hardware is never paywalled, unlike Unsloth, which charges for multi-GPU.
- The core stays Apache-2.0. Because contributions come in under a DCO, community code can't be relicensed, and that is part of the promise.

**What "charge for business" means**
Under Apache-2.0, anyone (companies included) may use the core for free, so there is no license fee on the app itself. Businesses pay for things only organizations need, and for services that cost Rastai money to provide. That's the model LM Studio, Unsloth and Ollama use. A Docker-Desktop-style rule ("companies over N employees must pay for the app") would require a non-open-source license on the app, so it's ruled out.

**Revenue streams**

| # | Stream | What's paid | When |
|---|---|---|---|
| 1 | **Rastai Teams** (self-serve, per seat) | Shared workspaces and projects, shared conversations/replays, team roles, a shared GPU node pool with scheduling across the team, a private plugin registry, Google/GitHub sign-in | Phase 3 (design partners in Phase 2) |
| 2 | **Rastai Enterprise** (annual contract) | SAML SSO and SCIM provisioning, audit logs, policy controls (allowed models and plugins, data-policy enforcement, retention), data residency, key management, an admin console, LTS releases, air-gapped install support, SLA support | Phase 3 |
| 3 | **Marketplace** (revenue share) | Paid plugins, templates, model designs, harnesses, lenses and eval suites, priced by their creators. Rastai takes a fee; creators keep the large majority | Free items in Phase 2; paid items in Phase 3 |
| 4 | **Rastai Cloud** (usage-based) | One-click rented GPUs (compute plus a margin); hosted private sharing of replays, models and projects with storage tiers; an optional hosted control plane for teams | Phase 3 |
| 5 | **Support and services** | Support contracts for labs and companies, onboarding workshops, and paid "add my architecture" model-family work (kept small so it doesn't distract) | Phase 3 |
| 6 | **Grants, sponsors and credits** (early bridge) | Open-source programs (e.g. Mozilla Builders, NLnet), GitHub Sponsors / Open Collective, interpretability and AI-safety research funders, GPU-provider and cloud startup credit programs | Phase 1–2 |

**Marketplace design**
- **Creators keep 80–85% of the sale price.** Free items are listed equally next to paid ones; paid placement never outranks quality or ratings.
- **Same trust bar for everything:** paid items go through the same signing, scanning, permission display and revocation as free ones (§24). Refunds are offered within a fixed window.
- **License keys:** the plugin host checks a signed license token for paid items, with an offline grace period. A failed check disables only that item, never the app. The DRM is light, because heavy DRM drives people to forks.
- **Bounties:** users or companies post a bounty for a plugin, lens or model family. Rastai holds the funds and pays out when the work is accepted, for a small fee. This seeds the ecosystem where demand is real.
- **Verified creators:** identity-verified publishers with payout accounts. Revenue dashboards show sales and installs (from registry data, not telemetry).
- **The flywheel:** creators earn → more and better plugins → more users → more businesses on Teams → more buyers for creators.

**Pricing hypotheses** (placeholders to validate with design partners, not commitments)
- Teams: per seat per month, comparable to developer tools in the ~$20–40/seat range.
- Enterprise: annual contract, priced per org.
- Cloud: compute at provider cost plus a margin; storage tiers for hosted sharing, with a free public tier.
- Marketplace: 15–20% fee.

**How the code is organized**
- The core is the public Apache-2.0 monorepo.
- Teams and Enterprise features live in a separate `rastai-business` repo under a commercial license, written by the Rastai company itself. They plug in through the same public plugin API, which also proves the API is good enough.
- Cloud is a hosted service built on the same node protocol, so a cloud GPU is just another node.
- Hooks already in the scope that make this cheap later:
  - the principal on every command (§21);
  - the data policy per project (§24);
  - provider plugins (§22);
  - the registry and signing (§13);
  - provenance (§13).

**Guardrails (the anti-patterns we avoid)**
- No branding or attribution restrictions on the open-source app. Open WebUI's branding clause got it marked non-free by Linux distributions.
- No telemetry-based monetization, and user data is never sold or used for training. This matches "no telemetry" (§24).
- No crippled "community edition." Enterprise features must be ones a single person genuinely doesn't need.
- No license rug-pull. The core license and the free-forever promise are published, and DCO contributions make relicensing impossible.
- No lock-in. Open file formats and `rastai.load()` mean people stay because Rastai is better, not because they're trapped. That's the whole goal.

**Prerequisites before charging anything**
- A legal entity.
- A registered trademark for the name (§34).
- Terms of service and a privacy policy for Cloud and the marketplace.
- A payment provider. Merchant-of-record services (e.g. Paddle) or Stripe with tax handling cover global sales tax/VAT for a small team. Payments are security- and compliance-critical, so they are never built in-house (§27).
- An entry for the payment service in the dependency register.

**Measuring traction without telemetry**
- Package downloads (PyPI, crates.io, npm), GitHub stars and forks.
- Registry installs, HF "Use in Rastai" clicks.
- Marketplace creators and sales.
- Community size.
- Opt-in surveys.
- Design-partner usage, by agreement.

**Sequencing**
- **Phase 1 (prototype):** no monetization. Build the product and earn trust. Apply for grants and credits once there's a public repo and a demo.
- **Phase 2:** public launch, the community, the free marketplace and registry, sponsors, and 3–5 design-partner labs or companies shaping the Teams/Enterprise features.
- **Phase 3:** Rastai Cloud, Teams and Enterprise generally available, the paid marketplace, and support contracts.

**Risks**
| Risk | Mitigation |
|---|---|
| Community feels features are being withheld | Public free-forever promise; enterprise line drawn at org-only needs |
| Thin cloud margins against hyperscalers | Compete on convenience and integration (one click, the loop, Psychology on remote nodes), not on price |
| Malware or low-quality marketplace items | Signing, scanning, review, refunds, revocation |
| Enterprise sales is slow for a solo founder | Lead with self-serve Teams; Enterprise only on inbound demand |
| Support burden crowds out building | Paid support only; community forum for free users; strong docs and `rastai doctor --bundle` |
| Better-funded competitors | Depth (the in-house engine), ecosystem and file formats; VC funding is an option, not a requirement |

## 37. Changes From v4.1
**v5.3 (technique coverage, Evaluation plugin)**
- All 12 missing techniques from the coverage check are in scope (§0.17).
- §11 / §17: MoE, MLA, ALiBi and AttnRes blocks; MoE, MLA and ALiBi families; layer placement across GPUs.
- §9: MoE routing lens, MLA cache observation, depth attention for AttnRes.
- §10 / §12: reward model, PPO, the RLHF chain and GSPO; one base model in VRAM for PPO.
- §10: Evaluation becomes a first-party plugin that tests models like code, with judges, Pass@k, Elo, perplexity and drift.
- §28: risks for oversized MoE/MLA models and unreliable judges.

**v5.2 (spike results, license)**
- License decided: Apache-2.0 + DCO + trademark policy.
- Spike done; the unified-runtime bet holds (§17). The in-house WebGL2 canvas holds the refresh cap in Chrome and in Tauri's WebKitGTK webview up to 50,000 nodes (§11).
- Backend language re-examined against Java and kept as Rust (§0.14).
- §9: fused logit lens, CUDA-graph decode, measured panel budget.
- §10 / §3: Teach it mixes in anchors; Verify reports drift; training memory defaults.
- §17: parity rules (skip template-forced positions, pin the kernel for exact replay), ring-buffer sliding cache, per-device capture streams.
- §22: numerical self-test; no ROCm overrides on unsupported chips.
- §23: the strict safetensors contract and its golden corpus.
- §4 / §0: AMD tier 1 waits on a discrete card; §6: download sizes and the gated-model flow.

**v5.1 and earlier**
- **Build-in-house policy:** most build-vs-buy calls flipped; dependency register and allowlist added (§27). Unsloth, TRL, PEFT, nnsight, SAELens, rLLM and React Flow are now prior art, not dependencies.
- **Unified model runtime (§17):** imported models run on the Designer's compiler; parity suite; opaque-node + compat fallback.
- **Owner decisions:** owner-first audience; Linux NVIDIA + AMD tier 1, macOS/Windows tier 2; Linux-only gym training; encryption at rest in Phase 2; no target date.
- **From the CTO review:**
  - definition of done (L0–L3);
  - process layout (one binary);
  - durability (workers own their logs);
  - storage (SQLite, vault, blob store);
  - schema ownership and versioning matrix;
  - a generic command endpoint;
  - UI over HTTP only;
  - gateway-owned tokenization;
  - VRAM leases with a managed `llama-server`;
  - local API security;
  - observability, self-update and disk management;
  - risk register, repo layout, ADRs and the 90-day plan.
- **Remote:** SSH first; iroh pairing codes and chunked transfers move to Phase 2.
- **Build order:** the unified runtime becomes step 2, because Inference, Psychology and Training all run on it.
- **Budget:** none needed for the prototype; code signing, GPU rental and a Mac come later (§4).
- **v5.1 Business model (§36):**
  - free for everyone with a public free-forever promise;
  - businesses pay for Teams/Enterprise features, Cloud and support;
  - a marketplace where creators keep 80–85%, plus bounties;
  - grants and credits as the early bridge;
  - the guardrails and the sequencing by phase.

## 38. References
**Competitors, prior art and data**
- Unsloth Studio: https://unsloth.ai/docs/new/studio
- Transformer Lab: https://github.com/transformerlab/transformerlab-app
- rLLM: https://github.com/rllm-org/rllm
- Harbor: https://github.com/harbor-framework/harbor
- Gemma Scope 2: https://ai.google.dev/gemma/docs/gemma_scope
- nnsight: https://github.com/ndif-team/nnsight
- circuit-tracer: https://github.com/safety-research/circuit-tracer
- Trackio: https://huggingface.co/blog/trackio
- Croissant: https://github.com/mlcommons/croissant

**Formats, protocols and libraries**
- torch.export: https://docs.pytorch.org/docs/2.9/export.html
- HF library registration: https://huggingface.co/docs/hub/models-adding-libraries
- MCP Apps: https://modelcontextprotocol.io/extensions/apps
- MCP Registry: https://registry.modelcontextprotocol.io
- MCP bundles (.mcpb): https://www.anthropic.com/engineering/desktop-extensions
- Agent Skills: https://agentskills.io
- WASI 0.3: https://wasi.dev/releases/wasi-p3
- iroh: https://crates.io/crates/iroh
- tonic joins gRPC: https://grpc.io/blog/grpc-welcomes-tonic/
- Tauri webview versions: https://v2.tauri.app/reference/webview-versions/
- uv + PyTorch: https://pydevtools.com/handbook/how-to/how-to-install-pytorch-with-uv.md
- PyTorch CUDA 13.0 wheels: https://dev-discuss.pytorch.org/t/transitioning-pypi-cuda-wheels-to-cuda-13-0-as-the-stable-release-2-11/3325

**Cloud providers (Phase 3)**
- SkyPilot: https://github.com/skypilot-org/skypilot
- RunPod Pods API (v2): https://docs.runpod.io/api-reference/pods/POST/pods.md
- Modal timeouts: https://modal.com/docs/guide/timeouts

**Ecosystem lessons**
- ComfyUI registry standards: https://docs.comfy.org/registry/standards
- ComfyUI malicious node incident: https://blog.comfy.org/p/upscaler-4k-malicious-node-pack-post
- Obsidian plugin future: https://obsidian.md/blog/future-of-plugins/

**Security and licensing**
- LiteLLM PyPI compromise: https://securitylabs.datadoghq.com/articles/litellm-compromised-pypi-teampcp-supply-chain-campaign/
- unsloth-zoo license (LGPL-3.0): https://github.com/unslothai/unsloth-zoo
