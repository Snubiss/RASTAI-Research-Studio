# Rastai Research Studio — Product Spec v4

Status: draft v4.1, 2026-10-10. Merges three reviews into v3.1 (ecosystem/interop, product, compute/cloud), plus the owner's decisions in §0.

---

## 0. Decisions

**Decided**
1. **The prototype is the whole suite.** Every section and every function in this spec works end to end, at prototype quality. No slimmed-down v1 (§21).
2. **Remote in the prototype:** SSH (bring-your-own-box pairing) and headless mode. RunPod native integration comes later, since it can't be tested until much later (§13).
3. **JEV dropped as a template.** It's a CNN trained a specific way. Build it as a Rastai template once data and custom input/output setup are finished.
4. **Name and `.rrs` prefix checked (2026-10-10):**
   - "Rastai Research Studio" has no existing hits.
   - The compound extensions (`.rrsmodel`, `.rrsproj`, …) don't collide. Bare `.rrs` is used by Road Rash saves and R-Studio/Remo recovery files, so never use bare `.rrs`.
   - Bare "Rastai" is crowded: RastAI (rastai.ai, a Persian AI learning platform), Rast.ai, RastrAI, Rastaa.ai, Rasta AI.
   - Package names `rastai` / `rastai-formats` / `rastai-node` are free on PyPI, crates.io and npm (including the `@rastai` scope), and `rastai` is free on HF Hub.
   - GitHub user `rastai` is taken.
   - rastai.com (registered 2011, parked) and rastai.ai are taken. rastai.dev/.io/.org/.app and rastairesearch.com appear unregistered.
   - A formal trademark search is still needed (§23).

**Open**
5. **Audience for the prototype.** *Recommended:* someone who runs local models but doesn't write training code. Pros get depth through Advanced mode and the code escape hatches (§19).
6. **License.** *Recommended:* Apache-2.0 for the app, formats and SDKs, plus a trademark policy for the name, with contributions under a DCO (§20).

---

## 1. Goal
A professional, all-in-one suite for designing, training, fine-tuning, testing, serving and studying neural networks. Basic users need little or no programming knowledge; professionals can go as deep as they want. Like Photoshop, it's simple on the surface and sophisticated underneath.

Models can be trained on any data: files, datasets, streaming data or single inputs. The user chooses what goes in and what they expect to come out, without needing to understand the code underneath.

It runs on local hardware or on rented datacenter/cloud GPUs. Switching between them is a dropdown, not a code change.

## 2. Principles
- **So good no one wants to clone it.** People just use it.
- **Open source from day one.** Nobody clones what they can fork, and improvements come back as contributions.
- **Be the default file format.** Every shared architecture, harness, replay or dataset is an invitation to Rastai.
- **Useful before install.** A shared file gives value with nothing installed: a web viewer for replays and designs, and `pip install rastai` for loading designs.
- **Zero friction.** Install to first meaningful result in under 10 minutes.
- **Depth is the moat.** Years of edge cases can't be rebuilt by watching software run.
- **Extract APIs, don't invent them.** Plugin APIs are extracted from first-party code that already uses them.
- **Every step ships.** Each build step puts something usable in users' hands.

## 3. Positioning
**Rastai is the studio for your own models: design them, see inside them, fix them, prove it.**

Every feature serves one loop:

**Design / Import → Observe → Explain → Edit → Train → Verify**

| Stage | What happens | Where |
|---|---|---|
| Design / Import | Build a model, compose pretrained blocks, or import any HF model | Designer |
| Observe | Every call is logged with token IDs | Gateway / Inference |
| Explain | See why it said that | Psychology |
| Edit | Steer, ablate, branch, pick the better answer | Psychology |
| Train | Edits become data (DPO pairs, SFT targets, LoRA) | Training |
| Verify | Behavior tests and a model diff prove the fix | Evaluation |

- **What we don't compete on:** chat UI breadth and the basic fine-tuning GUI (Unsloth Studio, Transformer Lab), or the agent RL framework (rLLM). Chat and fine-tuning are thin surfaces that serve the loop. The gym builds on rLLM.
- **Gaps nobody fills:**
  - Interpretability on *your own* fine-tunes, locally, wired into training (Neuronpedia only covers base models on the web).
  - A designer that carries one design from import through train and serve to re-import.
  - Edits that turn into training data.
  - Proof that a fix worked.

## 4. Architecture
- **Backend and API: Rust.** The control plane handles API endpoints, job scheduling, storage, the model gateway, sandbox orchestration, ModelSpec analysis, import/export and the plugin host.
- **Node agent: `rastai-node`.** One static Rust binary on every compute machine, the local one included. It handles:
  - transport, pairing and transfers;
  - GPU telemetry;
  - uv environments, so users never touch pip or CUDA;
  - job supervision;
  - enforcing budget and idle limits on the machine being billed.
- **Python job processes.** Each job (PyTorch, Unsloth, TRL, PEFT, nnsight) runs as a process under the node and talks to it over a local socket. A crash or OOM is reported with its cause and never takes down the node or the control plane. Local and remote nodes use the same protocol, so a cloud GPU is just a node somewhere else.
- **Runs survive the laptop.** Nodes write events to a durable, sequence-numbered log and keep working without the control plane. On reconnect, the control plane resumes from the last event it acknowledged. Closing the laptop never kills or orphans a run.
- **Protocol: schema-first** (protobuf/gRPC), so Rust and Python share one set of types.
- **Frontend: a web UI (React),** optionally wrapped in Tauri as a desktop app. ModelSpec compiles to WASM, so the designer validates and recalculates live in the browser.
- **Headless mode.** The control plane can run as a background service, and the Tauri app is just a client. `rastai serve --headless` runs the API and MCP with no UI. On a GPU box, the UI connects over an SSH tunnel. `rastai run proj.rrsproj --recipe sft` covers CI and clusters.
- **Command layer.**
  - Every action is a command with an id, a typed schema, an undo entry and a permission scope.
  - The UI, command palette, keybindings, menus, CLI and MCP tools are all thin callers of the same commands.
  - Every command offers "Copy as CLI / Python / MCP".
- **Model gateway.** Every model call (chat, Navi, external clients, agent rollouts) goes through one Rust gateway. It logs token IDs, the exact model version and the sampling settings.
- **Two runtimes per local model:**
  - **Fast:** llama.cpp / Ollama / vLLM.
  - **Inspectable:** HF Transformers + nnsight, which exposes activations and interventions.
  - One toggle in chat switches between them.
- **Adapter capability flags.** Each adapter declares `supports_activations`, `supports_interventions`, `supports_training` and `supports_logprobs`. Features light up only where they're supported.
- **First-party vs third-party UI.** Both use the same manifest and the same public API, enforced in the build. First-party sections run in-process (trusted) for 3D and per-token streaming. Third-party UI runs in sandboxed iframes using MCP Apps plus a shared design-system package, so it still looks native.
- **Parsing boundary.**
  - `rastai-formats` (Rust) parses every `.rrs` file.
  - Workers never parse `.rrs` files; they receive validated specs over protobuf.
  - Raw user data (CSV, PDFs, etc.) is parsed by converters inside the job sandbox on the node, next to the data. Datasets are never converted locally and then uploaded.
- **Later option:** native Rust compute (Burn / tch-rs) for from-scratch designer models.

## 5. Core Concepts
Every section of the app is a view over these:
- **ModelSpec:** the design format. A typed graph whose edges carry shapes, with batch and sequence kept symbolic.
- **Adapter:** connects to a model backend (Ollama, HF Transformers, vLLM, llama.cpp, OpenAI/Anthropic-compatible APIs, custom models), with capability flags.
- **Node:** a paired machine running `rastai-node`: local, your own box, or rented.
- **Compute target:** a provisioner that gets you a node: local, bring-your-own-box, RunPod, SkyPilot, Modal. After provisioning, everything goes through the node protocol.
- **Environment lock:** `uv.lock` + PyTorch variant + minimum driver version, identified by its hash.
- **Dataset:** sources + converter + splits + checks (dedupe, leakage, PII).
- **Converter:** turns a data format into standard training samples.
- **Recipe:** a training method: classification, pretraining, SFT, DPO, GRPO, LoRA/QLoRA, JEPA objectives, SAE training, etc.
- **Eval:** benchmarks, metrics and behavior tests.
- **Checkpoint:** saved weights plus the run that made them.
- **Harness:** system prompt + tools (MCP) + environment + reward.
- **Run:** a job spec plus its live event stream (metrics, logs, agent traces, activation summaries).
- **Conversation:** a message tree with branches, tool calls and optional token IDs (`rastai.conversation/1`).
- **Replay:** a conversation plus a model hash and settings, with a summary layer; it can be re-run to regenerate the model's internals.
- **Lineage:** what each object was made from and what uses it.
- **Secret:** an API key or cloud credential, stored in the OS keychain.
- **Plugin:** a package that adds a section, panel, tool or backend capability.

## 6. First Run (new)
One golden path with a model that's ready for Psychology by default.
- **0–3 min:** the installer detects NVIDIA, AMD, Apple Silicon or CPU and installs a pinned Python environment. It fetches Gemma 3 1B-it plus SAEs for 3 layers (Gemma Scope 2) in the background, showing the model license. Other layers download on demand.
- **3–5 min:** chat. Text is shaded by confidence; click a word to see what the model almost said and which concepts were active.
- **5–7 min:** drag a concept slider and watch the answer change live.
- **7–10 min:** rewind, branch, pick the better answers, press **Teach it**. A LoRA trains in a few minutes, then before/after is shown side by side with the model diff.
- **No usable GPU:** say so plainly. Offer the 270M model path, or connecting a rented GPU from any provider over SSH pairing. One-click RunPod comes later (§13).
- **Remote target:** first remote run in under 15 minutes, from pairing a box to training on it.
- **Home screen, organized by outcome:** Chat · Teach it my data · See inside · Fix a behavior · Design a network · Train an agent.

## 7. UX System (new)
- **One global Basic/Advanced setting,** overridable per panel. Advanced adds controls and never hides or moves Basic ones; switching keeps all state.
- **Command palette** (Ctrl/Cmd-K) over the command layer, showing each command's shortcut and copy-as-code.
- **Shared design system** for first-party and plugin UI.
- **One catalog** for diagnostics and plain-English explanations of terms.
- **Guided tutorials** that check real app state through commands. The assistant offers "show me" or "do it".
- **Accessibility (WCAG 2.2 AA):** colorblind-safe palettes, a table behind every chart, a keyboard-driven token timeline, reduced motion for 3D views.
- **Honest wording:** "what it's representing", not "what it's thinking".
- **Workspace layouts** are files, stored in the user profile or in a project.

## 8. Inference
- Chat interface for direct conversation, manual testing and basic chat.
- Side panel listing models and sessions.
- Direct inference through third-party APIs.
- Local OpenAI-compatible and Anthropic-compatible endpoints. Navi and any other client infer through them, with logging and the same adapters.
- Adjustable generation settings for local models: temperature, top-k, top-p, etc.
- Fast/Inspectable runtime toggle per model.
- **Replay twins:** a quantized model maps to its full-precision weights for inspection, with the fidelity labeled ("approximate: Q4 → bf16").
- **API model chats** log text plus any logprobs returned. They replay through a local observer model the user picks, labeled "proxy".
- Delete, archive, star or pin conversations:
  - Starred → dataset.
  - Pinned → behavior test (§10).
- Open any conversation in Psychology for replay.
- Optional live Psychology panel beside the chat.
- Incognito chats are never logged.
- Ability to set forward proxy

## 9. Psychology (first-party plugin)
Inference plus analysis tools for seeing inside a model. Works with Inspectable runtimes and designer-built models; for API models it is greyed out or runs in proxy mode.

**Basic view: "what it's representing"**
- **Active concepts per token:** labeled SAE features.
- **What it almost said:** the model's top guesses at each layer (logit lens).
- **What it's looking at:** attention heatmaps.
- **Confidence meter:** per-token entropy.

**Advanced view**
- 2D/3D activation projections per layer, per-head attention, linear probes and feature dashboards.
- Attribution graphs (via circuit-tracer, starting with Gemma 3 270M/1B).

**Visuals**
- **Vector space view:** the projection is fit once and new points are placed into that fixed space, so the map stays stable.
- **3D data-flow view:** layers × tokens, with level of detail.

**Replay**
- Play, pause, stop, rewind, slow down or speed up along a token timeline.
- The full view is recomputed from token IDs with one forward pass when the model is present.
- Every `.rrsreplay` embeds a summary layer: per-token entropy, top-k logit lens and top-k labeled features, targeting under 5 MB per 2k tokens. It opens with no model and no install in the web viewer.

**Live**
- A panel beside the chat streaming per-token summaries.
- Step mode: generate one token, pause, inspect, continue.
- A measured overhead budget, shown in the UI (tokens/s with the panel on vs off).
- **Remote models:**
  - Summaries are computed on the node, about 20 KB per token: top-k features, top-k logit lens, entropy, head-averaged attention.
  - Full tensors are fetched only when the user zooms in, the way nnsight's remote mode works on NDIF.
  - Replays recompute where the weights live, so only token IDs are shipped.

**Interventions and branching**
- Boost or suppress concepts, ablate heads, apply steering vectors.
- Rewind to any token, intervene, regenerate, and compare branches side by side.

**Turning edits into training (the loop)**
- **Prefer this branch:** a branch pair saves as a DPO pair or SFT target, linked to its replay.
- **Steering presets:** a saved intervention can be served through the gateway, or distilled into a LoRA.
- **Model diff** (base vs checkpoint): changes in feature firing frequency (SAE/crosscoder), logit-lens shifts and eval deltas. Unexpected changes are flagged.
- **SAE fit check:** after fine-tuning, measure how well the SAEs still reconstruct the model. Below a threshold, labels show "stale" and an SAE fine-tune is offered.

**Other**
- **Training view:** watch representations organize epoch by epoch for designer models.
- **Honest readouts:** labels show a confidence score and example inputs. They are evidence, not mind-reading.
- **Default models:** Gemma 3 (Gemma Scope 2) and Llama 3.1 8B (Llama Scope), plus any HF causal LM for the lenses that need no SAE.
- **Built on:** nnsight / TransformerLens, SAELens, Gemma Scope 2, Llama Scope, Neuronpedia labels and circuit-tracer.

## 10. Training and Evaluation
- Same interface as inference, plus a second window for live training and the dashboard.
- **Data import:**
  - Drop in a file or folder and the task is detected: label column → classifier; prompt/response → SFT; pairs → DPO; raw documents → synthetic Q&A recipe.
  - Before training: preview, token counts, splits, dedupe, train/test leakage check and PII flag.
  - Converters are generated from the preview, not written by hand.
- **Converters (data formats):** a declarative instruction file, with a Python/WASM fallback. Adding a format means adding one file.
- **Recipes (training methods):** plugins, mostly thin wrappers over TRL/PEFT/Unsloth.
- **Fine-tuning method dropdown:** LoRA, QLoRA, full fine-tuning, DPO, GRPO, etc., with sensible defaults.
- **SAE training recipe:** unlocks Psychology concept features for any supported model.
- **Hardware-aware defaults:** dtype, quantization, batch size and which recipes are available all come from the hardware profile plus the designer's memory estimate. A capability matrix turns recipes on or off per backend:
  - QLoRA runs on CUDA/ROCm only.
  - Mac fine-tuning uses Unsloth's MLX path.
  - bf16 needs Ampere or newer.
- **Cost and time before every launch:**
  - Time ≈ FLOPs ÷ (GPUs × peak FLOPS × MFU), plus setup and transfer time. MFU is the fraction of peak throughput the run actually reaches. FLOPs come from the designer, or roughly 6·N·T for full training and 4·N·T for LoRA (N = parameters, T = tokens).
  - Only offers that fit in VRAM are shown, cheapest first, with live price and stock.
  - Quotes give median and 90th-percentile time and cost. A 60 s probe run on the target corrects the quote, and the ETA and spend update live during the run.
- **Preemption-safe by default:**
  - Every recipe has a resume contract: weights, optimizer state, RNG state and dataloader position.
  - Checkpoint frequency follows preemption risk. On SIGTERM: checkpoint, relaunch on the next offer, resume.
  - The same mechanism covers provider time caps.
- **Multi-GPU on one machine:**
  - Each GPU is scheduled separately (e.g. chat on GPU 0, training on GPU 1). A VRAM arbiter unloads interactive models when training claims a GPU.
  - DDP via Accelerate, FSDP2 for recipes not built on Unsloth, vLLM tensor parallelism for serving.
- **Live dashboard:** a simple view for the common stats, and an advanced view for nerds.
- **Runs table:** config diff, metric overlay, tags and sweeps. Imports W&B, TensorBoard and Trackio logs.
- **Evaluation:** benchmarks and eval sets against any model or checkpoint, side by side.
- **Behavior tests:** any pinned conversation becomes a test that runs on every checkpoint. Tests are shareable as `.rrseval`.
- **Export:** GGUF / Ollama Modelfile / safetensors / HF Hub push, with base-model and dataset licenses carried through.
- **Streaming and single-input training** (online learning from data-source plugins): needs a definition before it's built. It ties into the custom input/output work that JEV needs.

## 11. Model Designer
A professional architecture tool. Basic users start from an import or a template; professionals edit everything.

**Real models**
- **Import into ModelSpec:**
  - HF `config.json` (core: Llama, Qwen, Mistral, Gemma, GPT-2, BERT, ViT families).
  - `torch.export` ExportedProgram.
  - ONNX.
  - Anything unrecognised imports as an **opaque node**: known shapes, sealed internals, still observable. Import never fails outright.
  - Arbitrary PyTorch code comes in through `torch.export`, never through source parsing.
  - Golden test: import → compile → logits match the original within tolerance.
- **Architecture map:** an imported model is an editable map, and Psychology uses it for navigation (click a block to see inside it).
- **Pretrained-block composer:** any HF backbone (e.g. ModernBERT, SigLIP) as a node with a freeze / LoRA / train toggle, plus new heads. This is the useful novice design: pretrained encoder + new head.
- **Live stats on every edit:** params, weights, FLOPs per token, memory for batch × sequence, a fits check per compute target and offer, shape errors, and a time/cost estimate per target. Warns when the model is far larger than the dataset.
- In Basic mode, text-generation goals are steered toward fine-tuning, with an explanation of why.

**Pro, from scratch**
- **Templates:** Transformer (decoder, encoder, encoder-decoder), CNN, MLP, RNN/LSTM, ViT, JEPA. TextCNN ports here, off the deprecated torchtext. JEV, a CNN trained a specific way, gets added as a template once custom data and input/output setup are done.
- **Three separate choices:** input encoders → backbone → output heads; the training objective is chosen separately.
- Hidden layer count, per-layer attention, expected input size, output head; custom build-your-own transformer with simple visuals.
- **Multi-input:** drag and resize ranges on a visual input map. Each range gets its own encoder, and a fusion node (concat, cross-attention, late fusion) combines them.
- **Hierarchy and reuse:** define a block once, stamp out N copies; zoom model → block → op.
- **Automatic observation points** at every node.
- **Plugin node types** for custom layers and blocks.
- **Eject:** turns a design into readable PyTorch source plus safetensors, with no rastai import. "Export to PyTorch" means exactly this.
- **Exports:** ONNX; Burn later.
- Designs are versioned and diffable. Save locally or to a remote server. Play button generates the model, ready for training.
- `pip install rastai` → `rastai.load("x.rrsmodel").to_torch()` returns a plain `nn.Module`, with no app or server.

## 12. Agent Gym (built on rLLM + Harbor)
- Use rLLM (Apache) for the rollout and RL engine and Harbor for task directories. Rastai adds the GUI, harness management and Psychology on agent traces.
- **Live peek:** watch an agent's thoughts and actions in real time. Read-only, with a measured overhead budget.
  - Step mode pauses one sandbox and removes it from the batch. Its group is either recomputed without it or marked partial, chosen per recipe, so GRPO advantages stay correct.
- **Sandbox management:** the user sets a sandbox cap per node. Docker on Mac and Windows; microVMs (Firecracker) optional on Linux.
- Rollouts and sandboxes run on the node next to the inference server. Only traces and rewards stream back.
- **Harness management:** toggle tools on and off. Tools are MCP servers. Import/export:
  - MCP client configs (`.mcp.json`, Claude Desktop, VS Code);
  - MCP Registry `server.json` references;
  - `.mcpb` bundles;
  - Agent Skills folders.
- Remote HTTP MCP servers count as network permissions (§18), not as sandboxed tools.

## 13. Compute, Workers and Data
**Hardware**
- Detect GPU, VRAM, driver, topology, Apple chip and unified memory, CPU, RAM and disk, then run a 20 s self-test (`rastai doctor`).
- The PyTorch build is picked automatically:
  - CUDA 13.0 by default (needs driver 580+);
  - legacy cu126 for Maxwell/Pascal/Volta;
  - ROCm per GPU architecture;
  - MPS/MLX on Apple;
  - CPU fallback.

**Compute targets (ranked)**
1. **Local:** NVIDIA, ROCm on Linux, Apple, CPU. This is the 10-minute path. *(Prototype.)*
2. **Bring your own box** (SSH or pairing code) **+ headless `rastai serve`:** covers Lambda, Vast, RunPod pods, home labs and university machines with no provider-specific code. *(Prototype.)*
3. **Later: RunPod native.** One click from no GPU to a GPU, with spot instances and price/stock APIs. Use REST v2 only; the v1 API retires Nov 15, 2026. Deferred because it can't be tested until later.
4. **Later: SkyPilot provider plugin.** One integration covers AWS, GCP, Azure, Lambda, Nebius, CoreWeave, Kubernetes, Slurm and SSH node pools, with autostop. This is the path for labs and pros. SkyPilot sits one layer above the providers, so it is an orchestrator, not a peer of RunPod or Modal.
5. **Later: Modal.** Serverless bursts for evals, batch inference and gym sandboxes, billed per second. Its 24 h cap per attempt makes long training awkward.

**Provider plugins**
- A target only provisions a machine and starts `rastai-node` on it.
- Provider plugins are separate processes speaking gRPC, like Terraform providers: `offers(reqs)`, `provision(offer, bootstrap)`, `status`, `stop`, `terminate`, `list_billable`.
- Bootstrap delivers a one-time pairing token via cloud-init, an environment variable or SSH, so pairing needs no clicks.
- Credentials come from the OS keychain on each call; plugins never store them.

**Pairing, identity and transport**
- Each node generates an Ed25519 identity on first run; the node ID is its public key.
- **Bring your own box:** `curl … | sh`, then `rastai-node pair <code>` or scan a QR code.
  - The code holds the control plane's ID, the relay address and a one-time secret, and expires after 10 minutes.
  - Both sides pin each other's keys, which gives mutual authentication without a certificate authority.
  - Nodes can be revoked in the UI.
- Nodes always dial out, so they work behind NAT.
  - Local nodes use a Unix socket or named pipe.
  - Remote nodes use iroh over QUIC: dial by public key, NAT hole-punching, encrypted relay fallback.
  - If UDP is blocked, they fall back to an SSH tunnel. The relay can be self-hosted.
- Heartbeat every 5 s with telemetry and job states; three missed heartbeats mark a node unreachable.
- The handshake negotiates a protocol version range and capabilities. Node self-updates are signed. Job environments are versioned separately.

**Cost safety** (arrives with provider integrations; Rastai can't stop billing on a bring-your-own box)
- Spending caps per run, per day and per month. The node enforces them even when it can't reach the control plane: checkpoint, then stop.
- Idle machines stop after 15 minutes by default, with a keep-warm option for interactive work.
- An orphan sweeper lists every billable `rastai-*` resource under a "paying $X/hr" banner.

**Data and artifacts**
- All data and artifacts are split into chunks named by their BLAKE3 hash (iroh-blobs). Transfers send only missing chunks, can resume, and are verified and deduplicated.
- **Fetch from the source; upload only what exists nowhere else.**
  - Nodes pull HF weights and data directly through the Xet chunk cache.
  - Only private local data is uploaded from the laptop.
  - A cache volume persists per provider, so the second run starts warm.
  - Why it matters: 10 GB over a 20 Mbit/s home uplink takes about 67 minutes.
- Converters run on the node, next to the data. Streaming sources are read on the node with scoped credentials.
- **Artifact catalog:** records each artifact's home (local or S3-compatible) and its copies.
  - LoRA adapters always sync back.
  - Full checkpoints go to a bucket and are fetched locally only on demand.
  - Teardown wipes remote volumes that aren't pinned.

**Remote serving**
- vLLM or SGLang serves on the node. Token IDs and logprobs are captured there, and the gateway logs them through the tunnel.

**Offline / air-gapped**
- An offline bundle: node binary, wheels or images per backend, model weights, SAEs and label dumps.
- Offline mode sets `HF_HUB_OFFLINE=1`, mirrors the plugin registry as a folder, and connects only over LAN or a self-hosted relay.
- A CI test checks that offline mode makes zero outbound connections.

## 14. Plugins and Ecosystem
**API contract**
- The plugin API uses semver, versioned separately from the app. The manifest declares `engines.rastai = ">=1.2, <2"`.
- **Capability negotiation:** a manifest lists `requires = ["designer.node@1", "psychology.lens@2"]`. If the host can't provide something, the plugin is disabled with a diagnostic instead of crashing.
- Experimental APIs sit behind a flag, and registry plugins can't use them.
- **Deprecation:** at least one minor release first; the previous major stays supported for 12 months.
- **Compatibility CI:** every release candidate runs the test suites of the top-N registry plugins; a regression blocks the release.
- First-party sections may use only the public API, enforced in the build.

**Extension points**
- **Declarative (no code):** theme, icon set, workspace layout, keybindings, command-palette actions and macros, locale, templates (design/harness/recipe/project), chat templates, eval suites.
- **Code:**
  - section, panel;
  - importer/exporter (a file type maps to a handler for double-click and drag-drop) and data source (streaming);
  - adapter, compute target, converter, recipe;
  - eval metric, reward function, sampler/logits processor, tokenizer;
  - sandbox image (OCI), designer node type;
  - Psychology lens/visualizer, SAE/dictionary loader;
  - assistant skill, settings page.
- Every contribution registers commands, so every plugin feature automatically becomes an MCP tool and a CLI verb.

**Plugin anatomy (`.rrsplugin`):** a manifest plus any of:
- **Frontend UI:** sandboxed iframe; MCP Apps (`ui://` resources, JSON-RPC over postMessage) plus a `rastai/` namespace, so any MCP App renders as a Rastai panel.
- **WASM module:** WIT component-model interfaces, run by the Rust backend through wasmtime with capability-based sandboxing.
- **Python job module:** runs as its own process under the node, with network off by default.
- **Provider process:** an out-of-process gRPC compute-target provider (§13). Provisioning needs its own form: WASM has no network by default, and job modules need a node to exist first.

**SDK: a plugin in an afternoon**
- `rastai plugin new <template>`, with templates for panel, section, converter, recipe, designer-node, lens, metric, reward, adapter, importer, exporter and theme.
- **Typed SDKs from one source:** `rastai-plugin` (Rust), `@rastai/plugin` (TS), `rastai_plugin` (Python).
- `rastai plugin dev`: loads the plugin unpacked into a running app, hot-reloads UI and WASM, restarts the Python module on save, opens devtools.
- `rastai plugin test`: a headless host with a mock worker, tiny fixture models (GPT-2-tiny) and UI snapshot tests.
- `rastai plugin lint | pack | publish`. Plugin errors get `RRS-Pxxxx` codes.
- Each extension point gets a reference page plus a runnable sample of about 50 lines.

**Dependencies**
- Every `.rrs` header can declare `requires: {plugin_id: semver}`. A file that needs a missing plugin opens with placeholder nodes (shapes known) and a banner: "Install 3 plugins (permissions: …)". It never fails hard.
- Plugins can depend on other plugins through semver ranges, resolved at install time.
- Python dependencies use a per-plugin `uv` lockfile with hashes, on top of the shared torch base. Installing packages at runtime is forbidden.

**Registry**
- The first registry (Phase 2) is a git index repo (manifests, hashes, signatures), reviewed by PR. Artifacts live on GitHub Releases or OCI. Users can add their own sources, such as private or company registries.
- Models, datasets and replays live on HF Hub under a `rastai` tag. Rastai doesn't host weights.
- Register Rastai as an HF library ("Use in Rastai" on model pages) and get into the HF "Use this model" local-apps list.
- The in-app listing shows installs, ratings, a "tested on x.y" badge from the compatibility CI, and permissions before install.
- **Pull loop:** dropping an unknown file, or opening a file with a missing node, suggests the plugins that handle it.
- Each plugin can publish stable, beta and dev channels.

**Provenance and remix**
- Every export embeds a pointer back to its recipe: safetensors `__metadata__`, GGUF key-values, ONNX `metadata_props`, and a generated model card saying "Made with Rastai, open recipe".
- The gallery shows fork/remix trees. "Remix" forks with attribution filled in automatically.

**Malleability**
- **Macros:** recording UI actions produces a Python or TS script against the SDK client, which can be shared.
- A read-only `<rastai-viewer>` web component embeds live designs and replays in docs, blogs, READMEs and HF Spaces.

## 15. Import / Export
| Area | Core (prototype) | Plugin |
|---|---|---|
| Conversations | Rastai schema, OpenAI messages JSONL (with tools), ShareGPT, ChatGPT export (keeps branches), Claude export | Open WebUI, LM Studio, SillyTavern (and character cards), LibreChat, Gemini/Grok takeouts |
| Models | HF Hub pull/push, safetensors, PEFT LoRA adapters, GGUF export, Ollama Modelfile, ONNX export | MLX, CoreML, ExecuTorch, TensorRT-LLM, AWQ/GPTQ/EXL2, merging (mergekit) |
| Datasets | HF datasets (including streaming), JSONL, Parquet, CSV, Alpaca, chat formats, DPO pairs, Croissant metadata | WebDataset, Kafka and other streams, SQL/DuckDB, S3, audio/video folders |
| Harnesses | MCP configs, server.json, .mcpb, Agent Skills | LangGraph, CrewAI, OpenAI Agents SDK, Inspect, lm-eval-harness tasks |
| Designs | HF config.json, torch.export, ONNX | Keras, JAX/Flax, timm, nanoGPT configs |
| Runs | Trackio / W&B / TensorBoard log import | MLflow, Aim |

- **Conversation schema `rastai.conversation/1`:** JSONL message tree with branches, tool calls/results, multimodal parts, and optional token IDs, model hash and chat template.
  - Importers keep branches (ChatGPT's `mapping` tree, Open WebUI parent links).
  - Text-only imports are retokenized and flagged "approximate replay".
- The long tail of importers ships as community plugins and doubles as the showcase for the converter template.

## 16. File Formats
| Extension | Holds | Format | Double-click |
|---|---|---|---|
| `.rrsmodel` | ModelSpec design: graph, input map, defaults, objective | JSON | Opens in the Designer |
| `.rrsharness` | System prompt, tool list (MCP refs), environment config, reward | JSON | Opens in the Harness editor |
| `.rrsdata` | Dataset: sources (path/HF/URL + hash), converter, splits, checks; Croissant-compatible | JSON | Opens in the Data preview |
| `.rrseval` | Eval suite: behavior tests (pinned conversations + expectations), benchmarks, metrics | JSON | Opens in Evaluation |
| `.rrsreplay` | Conversation (`rastai.conversation/1`), token IDs, model hash, settings, branches, annotations, summary layer (always), optional cached activations | Zip | Opens Psychology replay (web viewer without install) |
| `.rrsplugin` | Manifest + optional UI, WASM and Python modules | Zip | Installs (trust prompt per §18) |
| `.rrsproj` | Designs, harnesses, data, evals, runs, `rastai.lock`; weights referenced by content hash or HF repo, embedded only for offline sharing | Zip or folder | Opens the whole project |

- JSON for every definition file (not TOML), so the `$schema` key works.
- **Shared header:** `$schema`, `type`, `schema_version`, `created_with`, `id`, `content_hash`, `derived_from: [id@hash]`, `authors`, `license`, `source_url`, `requires`.
- `rastai-formats` ships under Apache/MIT to crates.io, PyPI (PyO3) and npm (WASM). JSON Schemas live at stable URLs (`schemas.<domain>/<type>/<version>.json`). Navi is optional, not required.
- **Standards for everything else:** `.safetensors` (and `.gguf` for inference exports), `.jsonl` / `.parquet`, `.onnx`. Never pickle-based `.pt` files for weights.
- **Checkpoints:** a folder holding safetensors weights, optimizer state as safetensors, and `resume.json` (RNG state, dataloader position, step), so resuming needs no pickle.
- **Hardware fingerprint** in every run record and `.rrsreplay`: environment hash, image digest, GPU, driver, dtype, quantization, kernels, seed.

## 17. Validation
- **Parse, don't validate:** files become strongly typed Rust structs at the boundary, and internal code never re-checks.
- **One crate owns all reading and writing:** `rastai-formats`, reused by the backend, the frontend (WASM), the CLI (`rastai validate`) and the PyPI package.
- **Four layers:**
  1. Syntax: `serde_json` line/column.
  2. Structure: Rust types + `schemars`-generated JSON Schema, with `serde_path_to_error` paths.
  3. Meaning: shape inference, references resolve, ranges don't overlap, graph acyclic unless recurrent.
  4. Environment: plugins installed, fits on the GPU, adapter supports it. These are warnings, not failures.
- **Compiler-style diagnostics:** collect all errors, each with a code (`RRS-Exxxx` for formats, `RRS-Pxxxx` for plugins), a path, a severity and a suggested fix (`miette`).
  - Basic view: plain English plus a fix button. Advanced view: the full diagnostic.
  - One catalog serves the UI, docs and plugin authors.
- **Migrations:** `schema_version` drives an upgrade chain on load, so old files always open. Files from a newer version open read-only.
- **Untrusted imports:** limits on size, depth, node count and parameter count before allocating; zip-bomb caps; zip-slip paths rejected; `trust_remote_code` off by default.
- **Testing:** `cargo-fuzz` on the parsers, round-trip property tests, and a golden corpus of broken files with expected error codes.
- **Beyond files:** typed errors (`thiserror`) in libraries, `anyhow` only at the edges, and every user-facing error mapped to a catalog code.

## 18. Trust and Privacy (new)
- No telemetry and no account. Crash reports only if the user opts in.
- A visible badge appears whenever data leaves the machine.
- **Gateway logs:** encrypted at rest, with a retention setting, incognito chats, and PII/secret redaction on dataset export.
- **Secrets** live in the OS keychain and never go into `.rrsproj`.
- **Cloud credentials never leave the control plane.** Nodes get short-lived scoped tokens or presigned URLs.
- **Data policy per project:** `local-only`, `trusted-remote` or `any`. Community hosts (Vast, RunPod Community Cloud) show a warning that data is going to a third party.
- **Every API command carries a principal** (the acting user), so team mode can come later without rework.
- **Fine-grained plugin permissions:**
  - `network:[hosts]`, `fs:project|ro`, `secrets`;
  - `gpu`, `weights.read`, `conversations.read` (sensitive);
  - `gateway.spend`, `sandbox.spawn`, `commands:[scopes]`.
- Any network or data permission triggers the trust prompt, whether the plugin is declarative or not. A declarative adapter URL or MCP reference can still send data out.
- **Enforcement:**
  - WASM uses wasmtime capabilities.
  - Python modules run as separate processes with network off by default. They are still trusted code: isolation on macOS and Windows is best-effort, and the trust prompt says so.
- Sigstore signing from CI, a verified-publisher badge, and a scan of every version. A signed revocation list is checked at launch, and an update that adds permissions asks again.

## 19. Code and Reproducibility (new)
- **Escape hatches in both directions:**
  - `rastai.observe(model)` gives any PyTorch/HF model in a notebook the Psychology views.
  - `rastai.log()` is wandb-compatible.
- Any run opens as a notebook or script. Eject turns designs into plain PyTorch.
- **Run record:** job spec + dataset hash + base-model hash + plugin versions + environment lock hash + image digest + seed + hardware fingerprint + git commit. `rastai rerun <id>` reproduces it.
- **Honest reproducibility:** a replay or rerun on different hardware matches within a stated tolerance, not bit for bit. A deterministic mode is offered where the stack supports it (e.g. vLLM batch invariance on recent GPUs).
- **Lineage panel** on every object: replay → pairs → dataset → run → checkpoint → diff.

## 20. Licensing and Community (new)
- **Recommended:** Apache-2.0 for the app, formats and SDKs. Many companies ban AGPL, which is what Unsloth Studio's UI and Transformer Lab use, so permissive is an advantage. A license won't stop a clean-room clone anyway.
- Trademark policy protects the name. Contributions use a DCO, not a CLA. Plugins can use any license.
- The format spec lives in its own repo under CC-BY with a public RFC process. The spec is the moat.
- **Contributor path:** converters (one file) → lenses → recipes. A "paper-to-lens" program lets interpretability papers ship as Rastai lenses.

## 21. Build Order
The prototype is the whole suite: every section and every function in this spec working end to end, at prototype quality, on local hardware and on an SSH box. Each step adds a working section on top of the last.

**Phase 1: Prototype (the whole suite)**
0. **Spike (1–2 weeks).** Prove the hardest path in a notebook: live Psychology on Gemma 3 1B, on an 8 GB GPU and on an M-series Mac. Measure tokens/s and VRAM with the live panel on. This shapes the architecture; it isn't a go/no-go gate.
1. **Foundation:** `rastai-formats`, protobuf, command layer + CLI + MCP, `rastai-node` running locally (environments, job supervision, durable event log), installer with hardware detection and `rastai doctor`, plugin host + manifest (first-party sections load through it from day one).
2. **Inference:** gateway, OpenAI/Anthropic-compatible endpoints (Navi), adapters (Fast + Inspectable), chat, token-ID logging, `rastai.conversation/1`, archive/star/pin.
3. **Designer:** ModelSpec, importers (HF `config.json`, `torch.export`, ONNX) with opaque nodes, architecture map, pretrained-block composer, from-scratch templates (TextCNN ported off torchtext), multi-input, hierarchy, live stats, compile to PyTorch, Eject, ONNX export, `rastai.load()`.
4. **Training and Evaluation:** data import with checks, converters, recipes (classification, SFT, LoRA/QLoRA, DPO, GRPO, SAE), dashboard, runs table, evals, behavior tests, export (GGUF/Ollama/safetensors/HF).
5. **Psychology:** replay → basic lenses → live panel → steering → branching → edits become training data → model diff and SAE fit check → advanced views (projections, probes, attribution graphs, 3D data flow) → replay summary layer and web viewer.
6. **Agent Gym:** rLLM + Harbor, Docker sandboxes with a cap, harness editor with MCP import/export, live peek and step mode, Psychology on agent traces.
7. **Plugins:** SDK + `rastai plugin new | dev | test | pack`, local install of `.rrsplugin`, "install missing" for files.
8. **Remote:** bring-your-own-box pairing (SSH / code / QR), headless `rastai serve`, content-addressed transfers, preemption-safe resume.

**Prototype done when:** every section works end to end, locally and on a paired SSH box.

**Phase 2: Harden and open the ecosystem**
- Plugin API 1.0, extracted from the prototype's first-party sections; compatibility CI.
- Git-index registry, Sigstore signing, revocation list.
- Publish `rastai-formats` / SDKs to crates.io, PyPI and npm; schemas at stable URLs; HF library registration.
- `<rastai-viewer>` embeds; performance and overhead budgets; accessibility pass; docs and tutorials; offline bundle.

**Phase 3: Rented cloud and scale**
- RunPod native, cost/time estimator with live prices, spending caps, idle stop, orphan sweeper.
- SkyPilot provider plugin, then Modal.
- Sweeps at scale; team mode.

## 22. Deferred Past the Prototype
- Everything in Phases 2 and 3 above.
- Hosted registry with reviews and comments; any paid marketplace.
- Two-way MCP Apps (Rastai panels running inside other hosts).
- Keras/JAX importers; parsing PyTorch from source.
- MLX, CoreML and TensorRT exporters; model merging.
- Native Rust compute and Burn export.
- JEV template (after custom data and input/output setup are finished).
- Streaming and single-input training (needs a definition).
- **Compute:**
  - multi-node training and DeepSpeed;
  - native Kubernetes, Slurm, Vast, Lambda and CoreWeave adapters (use SkyPilot or SSH instead);
  - failover across providers, spot bidding, autoscaled serving endpoints;
  - streaming full tensors over the internet;
  - native Windows ROCm training and Intel GPUs;
  - aggregators (Shadeform, Prime Intellect).
- Team mode (the principal field is already in place).
- Third-party plugins in a public registry (Phase 2; local install works in the prototype).

## 23. Open Questions
- The open items in §0 (audience, license).
- **Name follow-ups:**
  - Run a formal trademark search (USPTO, EUIPO, WIPO Global Brand Database) for "Rastai" in software/AI classes. The main conflict is RastAI (rastai.ai), a Persian AI learning platform.
  - Claim the names before announcing: PyPI `rastai`, crates.io `rastai` / `rastai-formats` / `rastai-node`, the npm `@rastai` org, the HF org `rastai`, a GitHub org (e.g. `rastai-research`, since user `rastai` is taken), and a domain (e.g. rastai.dev or rastairesearch.com).
  - Publish real minimal packages rather than empty placeholders; PyPI's PEP 541 discourages name squatting.
- Community plugin registry: when to move beyond the git index.
- Live peek and the Psychology live panel: what overhead budget is acceptable (target tokens/s hit)?
- **Verify before building on these** (flagged by the compute review):
  - the exact RunPod spot-price field name;
  - vLLM batch invariance's minimum GPU (CC 8.0 vs 9.0);
  - Unsloth MLX on Mac (source: release notes);
  - SkyPilot v0.13's release;
  - the bandwidth and cost rules of thumb.

## 24. Changes From v3.1
**v4.1 (owner decisions)**
- The prototype is the whole suite. The build order is now three phases (prototype, harden/ecosystem, cloud/scale), and the debugger-only v1 alternative is dropped.
- SSH + headless are in the prototype; RunPod native moves to Phase 3.
- JEV is removed from the templates and becomes a later template.
- Name and extension check results are recorded in §0, with follow-ups in §23.

**v4 (three reviews)**
- **Positioning:** one loop (Design/Import → Observe → Explain → Edit → Train → Verify) replaces the four separate differentiators.
- **New sections:** Decisions Needed, First Run, UX System, Agent Gym (split out), Trust and Privacy, Code and Reproducibility, Licensing and Community, Deferred.
- **Compute:** `rastai-node` agent (runs survive the laptop), gRPC provider plugins, Ed25519 pairing over iroh/QUIC, hardware auto-config and `rastai doctor`, cost/time quotes with caps and idle stop, a content-addressed data plane, preemption-safe resume, environment locks, compute next to the weights, a data policy per project, an offline bundle.
- **Architecture:** `rastai-node` agent, headless mode, a richer command layer, Fast/Inspectable runtimes, first-party in-process vs third-party iframe, an explicit parsing boundary.
- **Core Concepts:** added Dataset, Eval, Checkpoint, Conversation, Lineage, Secret.
- **Inference:** replay twins, proxy replay for API models, pin → behavior test, incognito.
- **Psychology:** replay summary layer and web viewer, branch → DPO, steering presets, model diff, SAE fit check, overhead budget, default models.
- **Training:** data import with task detection and checks, runs table, behavior tests.
- **Designer:** importers (HF config.json, torch.export, ONNX) with opaque nodes, architecture map, pretrained-block composer, cost estimates, Eject defined, pip loader.
- **Gym:** built on rLLM + Harbor; GRPO step-mode handling defined; Docker on Mac/Windows.
- **Plugins:** versioned API contract, capability negotiation, complete extension points, SDK and dev loop, dependencies with "install missing", git-index registry plus HF Hub, provenance/remix, MCP Apps UI.
- **File formats:** 7 types (added `.rrsdata` and `.rrseval`); JSON only; extended header; weights referenced rather than embedded; formats published as packages and schemas at stable URLs.
- **Trust:** "declarative = no warning" replaced by permission-based prompts; Python isolation described honestly; signing and revocation.
- **Build order:** spike first; every step ships a working section; the plugin API is extracted from first-party code.

## 25. References (from reviewer research; verify before relying)
- Unsloth Studio: https://unsloth.ai/docs/new/studio
- Transformer Lab: https://github.com/transformerlab/transformerlab-app
- rLLM: https://github.com/rllm-org/rllm
- Gemma Scope 2: https://huggingface.co/google/gemma-scope-2-4b-it
- Trackio: https://huggingface.co/blog/trackio
- HF library registration: https://huggingface.co/docs/hub/models-adding-libraries
- torch.export: https://docs.pytorch.org/docs/2.9/export.html
- VS Code extension anatomy: https://code.visualstudio.com/api/get-started/extension-anatomy
- WASI 0.3: https://bytecodealliance.org/articles/WASI-0.3
- ComfyUI workflow metadata: https://docs.comfy.org/development/api-development/workflow-metadata
- ComfyUI registry standards: https://docs.comfy.org/registry/standards
- ComfyUI malicious node incident: https://blog.comfy.org/p/upscaler-4k-malicious-node-pack-post
- Obsidian plugin future: https://obsidian.md/blog/future-of-plugins/
- MCP Registry: https://registry.modelcontextprotocol.io
- MCP Apps: https://modelcontextprotocol.io/extensions/apps
- MCP bundles (.mcpb): https://www.anthropic.com/engineering/desktop-extensions
- Agent Skills: https://agentskills.io
- Croissant: https://github.com/mlcommons/croissant
- Open WebUI import/export: https://docs.openwebui.com/features/chat-conversations/data-controls/import-export
- Open WebUI license: https://docs.openwebui.com/license/
- mergekit license: https://www.arcee.ai/blog/mergekit-returns-to-its-roots
- SkyPilot: https://github.com/skypilot-org/skypilot
- SkyPilot SSH node pools: https://docs.skypilot.co/en/stable/reservations/existing-machines.html
- SkyPilot managed jobs: https://docs.skypilot.co/en/stable/examples/managed-jobs.html
- RunPod Pods API (v2): https://docs.runpod.io/api-reference/pods/POST/pods.md
- RunPod GraphQL: https://docs.runpod.io/docs/graphql-api
- Modal timeouts: https://modal.com/docs/guide/timeouts
- Lambda instances: https://docs.lambda.ai/public-cloud/on-demand/creating-managing-instances/
- iroh: https://docs.rs/crate/iroh/latest
- iroh-blobs: https://docs.iroh.computer/protocols/blobs/
- HF Xet: https://huggingface.co/docs/hub/xet
- PyTorch CUDA 13.0 wheels: https://dev-discuss.pytorch.org/t/transitioning-pypi-cuda-wheels-to-cuda-13-0-as-the-stable-release-2-11/3325
- PyTorch CUDA 13.2 / legacy builds: https://dev-discuss.pytorch.org/t/introducing-cuda-13-2-and-deprecating-cuda-12-8-release-2-12/3337
- CUDA 13.0 release notes: https://docs.nvidia.com/cuda/archive/13.0.0/cuda-toolkit-release-notes/index.html
- vLLM batch invariance: https://docs.vllm.ai/en/latest/features/batch_invariance/
- nnsight / NDIF: https://arxiv.org/abs/2407.14561
- Unsloth AMD: https://unsloth.ai/docs/basics/amd
- Unsloth multi-GPU: https://unsloth.ai/docs/basics/multi-gpu-training-with-unsloth
