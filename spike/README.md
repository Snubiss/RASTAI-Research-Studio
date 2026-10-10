# Rastai spike (spec v5 §32, weeks 1–2)

Throwaway experiments that test the riskiest bets in the plan before anything is built on them. The code is spike-quality, but it follows the house code style. The numbers in `results/` are the deliverable.

Run 2026-10-10 on the owner's box: RTX 5080 16 GB (Blackwell, sm_120) and RTX 3090 24 GB, driver 595.91, PyTorch 2.14.1 + CUDA 13.0, Python 3.12, Linux kernel 7.0.

## Verdict

| # | Spike item (§32) | Result | Headline |
|---|---|---|---|
| 1 | In-house Gemma 3 1B matches the HF reference | **Pass** (one pre-set bf16 check failed on a noise position; see 01b) | fp32 eager attention is **bit-identical** to transformers; fast attention within 6e-4; all greedy generations identical |
| 2 | Live lenses: tokens/s and VRAM, panel on vs off | **Measured** | Full panel costs **+16–31%** (1.5–2.4 ms/token) at CUDA-graph speed; whole stack peaks at **2.7 GiB** |
| 3 | llama.cpp and the gateway tokenizer give identical token IDs | **Pass** | **0 mismatches** on 162 cases / 49,035 tokens; f16 GGUF generations equal ours token for token |
| 4 | In-house LoRA in under 3 minutes while the chat model stays loaded | **Pass** | Teach it in **~10 s** end to end, 4.7 GiB peak with both models resident; anchors cut side effects by 83–87% (03b) |
| 5 | uv environment install time, CUDA and ROCm | **Measured** | CUDA 33 s / 5.2 GB cold, ROCm 91 s / 16.5 GB, CPU 3 s / 0.75 GB; warm reinstall under 1 s |
| 6 | Canvas: 1,000 nodes + 3D, WebKitGTK vs Chrome | **Pass** | Both engines hold their refresh cap up to **50,000 nodes** and 213k live cubes; ~35–40× headroom over the target |
| 7 | *Added:* Rust vs Java for the backend (owner's question) | **Measured** | See section 07 below |

**The unified-runtime bet holds.** A from-scratch Gemma 3 (about 500 lines including comments and the static decode path) reproduces Hugging Face exactly. The live panel is affordable, Teach it is about 20× faster than its budget, and the gateway can own tokenization. Nothing found here argues against spec §17, and the in-house WebGL2 canvas is fast enough in Tauri's webview with room to spare.

## What each experiment found

### 01: Gemma 3 parity (`results/01_gemma_parity.md`, `01b_bf16_flip_investigation.md`)
- **fp32, eager attention:** max |Δlogit| = **0** on all prompts and both GPUs.
- **fp32, SDPA:** within 6×10⁻⁴, KL ~10⁻⁹.
- **Generation:** 64-token greedy generations identical in all cases.
- **KV cache:** token-by-token decoding matches one full pass.
- **bf16:** our drift from fp32 is the same as or lower than HF's (ratio 0.88–1.05).
- **The one failure:** on the 5080, one of 19 positions flipped top-1 in bf16. 01b traced it:
  - our bf16 eager path is bit-identical to HF's bf16;
  - the flip comes only from PyTorch's "efficient attention" kernel summing in a different order;
  - the position sits inside the chat template, where the next token is forced, with a 0.8-logit near-tie between two junk tokens.
- **For the real parity suite:** skip template-forced positions. **Exact replays must pin the attention kernel** (math or eager), because fast kernels differ in bf16.

### 02: live Psychology panel overhead (`results/02_live_lens_overhead.md`)
- **CUDA graphs are needed for decode.** Our Python decode loop is launch-bound at ~70 tok/s. Replaying one static-shape step as a CUDA graph gives 150–155 tok/s.
- **Panel cost:** naive +210%; CTO rules (batched) +26–41%; **fused +16–31%**.
  - "Fused" is a new rule for spec §9: the logit lens's last row *is* the model's output, so the 600 MB unembedding matrix is read once instead of twice.
- **Per-token packet:** 8.3 KB, under the spec's ~20 KB target. It carries:
  - entropy and the top 10 guesses;
  - a 26-layer logit lens;
  - the newest token's top attended positions per head and layer;
  - top-16 features from 3 real SAEs.
- **Gemma Scope 2 SAEs** (Google, CC-BY-4.0, ungated) run on our in-house SAE runtime:
  - they fire 57–63 features per token, against the 54–60 they were trained for;
  - reconstruction leaves 3–24% of variance unexplained;
  - that is independent proof the residual stream is right.
- **The logit lens is already readable:** on "due", layers 15–20 say "because", and the last layers switch to "due" (53% vs 42%).
- **Bug found and fixed:** `torch.cuda.graph`'s default capture stream is created once, on whichever GPU was current first, so the second GPU's capture failed. The node must give each device its own capture stream. This matters for "chat on GPU 0, training on GPU 1".
- **Optimisation found:** sliding-window layers attend over the whole cache in static decode. A 512-slot ring buffer would cut long-context decode cost (110 vs 155 tok/s today).

### 03 / 03b: Teach it (`results/03_teach_it_lora.md`, `03b_teach_it_drift.md`)
- **Setup:** 40 Q&A pairs about Rastai, facts the model can't know, on the 5080 capped at 8 GiB, with the chat model resident the whole time.
- **Speed and memory:** Teach it took **9.6 s** end to end against a 180 s budget, peaking at **4.7 GiB**. The chat model kept serving at ~71 tok/s between epochs.
- **Accuracy:** held-out accuracy (new phrasings) went from 20% to 80–90%; general knowledge stayed at 100%.
- **Side effects:**
  - the model copied the training data's terse style;
  - one recipe invented a fact (a "Pedro de Alva" editor for Romeo and Juliet);
  - the keyword checks did not catch either.
- **03b:** adding *anchors* (the base model's own answers to 40 unrelated questions) cut drift on unseen prompts by **83–87%** (KL 2.14 → 0.27–0.36). Held-out accuracy stayed at 80–90%, and style was preserved.
  - Best recipe: facts + anchors, 8 epochs: 90% held-out in 11.4 s.
- **Memory lessons:**
  - compute the loss only at answer positions (262k-vocabulary logits everywhere cost gigabytes);
  - LoRA keeps float32 master weights but computes in bf16;
  - gradient checkpointing on by default.
- **Remaining weakness:** small models fill gaps with plausible inventions ("ModelGraph" for ModelSpec). Verify needs more than keyword tests; **drift KL is a good default model-diff metric.**

### 04: tokenizer parity and the Fast runtime (`results/04_tokenizer_and_fast_runtime.md`)
- **Tokenizer:** HF `tokenizers` vs llama.cpp b11537 gave **0 mismatches**, with and without begin-of-text, and on detokenization.
  - Corpus: every spec paragraph, the whole spec, source code, and edge cases (NUL bytes, emoji joiner sequences, combining marks, control characters, special tokens written as text).
- **Generation from identical token IDs:**
  - f16 GGUF matches our fp32 runtime on **every token of all 4 prompts**;
  - Q4_K_M diverges after 3–9 tokens, which confirms the "approximate: Q4 → bf16" replay-twin label.
- **Speed:**

  | Runtime | RTX 5080 | RTX 3090 |
  |---|---|---|
  | llama.cpp f16 decode | 311 tok/s | 209 tok/s |
  | llama.cpp Q4 decode | 496 tok/s | 267 tok/s |
  | Our Inspectable runtime decode | 155 tok/s | 149 tok/s |
  | llama.cpp prefill | 44k tok/s | 17k tok/s |
  | Our prefill | ~36k tok/s | ~23k tok/s |

  The two-runtime design (§8) is justified. Our decode is launch-bound; fused kernels are the lever.
- **Packaging:** llama.cpp now ships official CUDA 13.4 Linux builds with their runtime libraries, so the managed `llama-server` needs no CUDA toolkit. Pin a build and check its SHA-256 (`vendor/` does this).

### 05: environment install time (`results/05_env_install_timing.md`)

| Backend | Cold install | Download | Warm reinstall |
|---|---|---|---|
| CUDA | 33 s | 5.2 GB | under 1 s |
| ROCm | 91 s | 16.5 GB | under 1 s |
| CPU | 3 s | 0.75 GB | under 1 s |

- On this connection, the First Run's "0–3 min" holds for CUDA, including the 2 GB model and 0.45 GB of SAEs.
- ROCm's 16.5 GB is heavy: the installer should show the size and time before starting.
- **The AMD GPU on this box is the Ryzen's integrated graphics (gfx1036).** ROCm detects it but has no kernels for it. Forcing it (`HSA_OVERRIDE_GFX_VERSION=10.3.0`) returns **silently wrong matmuls**: up to 100% error at some sizes, and different from run to run. So:
  - AMD tier-1 testing needs a discrete RDNA3+ card;
  - `rastai doctor`'s self-test (§22) **must check numerical correctness against a CPU reference**, not just that kernels run.

### 06: the canvas in Chrome and WebKitGTK (`results/06_canvas_webviews.md`)
- **Setup:** an in-house WebGL2 renderer (no libraries), in Chrome for Testing 155 and in WebKitGTK 2.52, the engine Tauri uses on Linux. WebKitGTK is opened through a tiny program built on Tauri's own `wry`/`tao` crates (`webview-harness/`).
  - The Designer graph: rounded-rect nodes, thick edges and text labels from a glyph atlas, all on screen, the camera panning and every node's colour changing each frame. Three draw calls per frame at any size.
  - Psychology's 3D flow view: layers × tokens cubes whose values are re-uploaded every frame.
- **Target met with large margin:** both engines hold their frame cap (Chrome 144 fps, WebKitGTK ~62 fps) for every configuration up to 50,000 nodes with 620,000 glyphs, and 213,000 cubes.
- **Headroom:** the combined scene (1,000 nodes + the 26×512 flow view) can be drawn 16× per frame in Chrome and 32× in WebKitGTK before dropping below the cap. That's roughly 35–40× the spec's 60 fps target.
- **Both really draw:** a pixel read-back after each scene finds matching coverage (e.g. 28.5% vs 27.3% of the screen for 1,000 nodes). This matters because WebKitGTK hides its GPU name ("Apple GPU") from pages.
- **WebKitGTK animates at ~60 Hz on a 144 Hz monitor**, where Chrome gets 144. Fine for this app, but worth knowing about Tauri on Linux.
- **No NVIDIA + Wayland blank-window problem** on this setup: results are identical with and without the usual `WEBKIT_DISABLE_DMABUF_RENDERER=1` workaround.

### 07: Rust vs Java for the backend (`results/07_backend_bakeoff.md`)
The owner asked whether the backend should be Java. The other experiments don't depend on that choice: models, lenses and training run in the Python workers either way. So this builds what the backend itself does in both languages, written the usual way for each, with the same command-line contract (`bakeoff/rust`, `bakeoff/java`).

The components:
- the strict safetensors reader;
- gateway tokenization;
- a streaming OpenAI-compatible gateway that tokenizes, forwards token IDs to llama-server, relays chunks and logs token IDs to SQLite.

Java ran three ways: the JVM with defaults, the JVM tuned like a desktop app, and a GraalVM native binary.

- **Correctness: a tie.** All four agree with the contract on all 33 safetensors files and never crash. All produce the same checksum on the 2 GB model and identical token IDs on the 162-case corpus. All log identical token IDs end to end through the real model. No errors or timeouts in 1.7 million measured load-test requests.
- **Speed and footprint: Rust ahead.**

  | | Rust | Java JVM (defaults) | Java JVM (tuned) | Java native image |
  |---|---|---|---|---|
  | Gateway req/s at 1 / 64 clients | 2,923 / 17,868 | 1,786 / 14,376 | 1,810 / 13,254 | 1,472 / 12,373 |
  | p99 latency at 256 clients | 28 ms | 26 ms | 55 ms | 37 ms |
  | CPU per request | 781 µs | 983 µs | 995 µs | 1,094 µs |
  | Idle / peak memory | 252 / 360 MiB | 705 / 1,901 MiB | 632 / 814 MiB | 435 / 570 MiB |
  | Startup to first healthy response | 0.5 s | 1.2 s | 1.2 s | 1.1 s |
  | Tokenizer speed | 2.83 M tok/s | 1.30 M | 1.26 M | 1.26 M |
  | Shippable | 9 MiB binary | 98 MiB (jar + runtime) | 98 MiB | 37 MiB binary |
  | Clean / incremental build | 32 s / 2.0 s | 4 s / 4.2 s | | 28 s |

  About 250 MiB of every gateway's memory is the Gemma tokenizer itself (262k-entry vocabulary).
- **Java's real advantage is clean-build time** (4 s vs 32 s). Day to day, Rust's incremental builds are faster (0.4 s vs 1.2 s for a dev build). The code is about the same size (550 vs 621 lines).
- **Java pitfalls found:**
  - Java's practical tokenizer (DJL) **silently truncates long inputs by default**: 7 of 162 cases came back short until truncation was turned off. That is dangerous for a gateway whose job is exact token IDs.
  - DJL **phones home**: on startup it probes the AWS metadata address (169.254.169.254) to send usage telemetry. Off AWS that costs ~4 s of timeouts, and it breaks spec §24's "no telemetry". It's off in code now (`OPT_OUT_TRACKING`).
  - DJL wraps **the same Rust `tokenizers` library through JNI**, in an older version (0.21 vs 0.23), and bundles native libraries for every platform. A Java backend still ships Rust for its most exact-sensitive job.
  - The GraalVM native image **runs class initializers at build time**, so a runtime setting (`sun.net.httpserver.nodelay`) was silently ignored. The result was 40 ms stalls, errors and one hang under load until the setting was baked in at build time. It also loses the JIT's AVX-512 checksum (728 ms vs 51 ms).
- **Lessons that apply to the Rust build too:**
  - streaming servers need `TCP_NODELAY` (both stalled 40 ms per reply without it);
  - SQLite logging belongs on one writer thread; the first Rust version convoyed on a mutex at 256 clients;
  - library choice can swamp language choice (an AVX-512 CRC crate made Rust's checksum 4.6× faster).
- **Decision:** the backend stays Rust (spec §0.14).

## Spec changes (applied in spec v5.2)
1. **§9 performance rules:**
   - add "fused logit lens: share the unembedding with the output";
   - add "decode via a captured static-shape step (CUDA graph), one capture stream per device".
2. **§17:**
   - parity criteria skip template-forced positions;
   - exact replay pins the attention kernel;
   - sliding layers use a ring-buffer cache;
   - gradient checkpointing and answer-only loss by default.
3. **§10 / §3 loop:**
   - Teach it mixes in base-model anchors by default;
   - Verify reports drift KL on a fixed general prompt set next to the behaviour tests;
   - keyword checks alone are not enough.
4. **§22:** `rastai doctor` runs a numerical self-test, and never enables ROCm through an override on unsupported chips.
5. **§4:** the AMD tier-1 claim waits on a discrete card; the iGPU doesn't count.
6. **§6 / §25:**
   - the installer shows the download size per backend;
   - gated model downloads need the HF-token and licence-acceptance flow (Gemma's official repo is gated).

## Not done, and why
- **AMD numbers:** wait on a discrete AMD card.
- **Timing noise:** Steam's shader pre-caching (`fossilize_replay`, ~20 processes) was running on the 5080 during the runs. Treat the 5080 timings as slightly pessimistic, and re-run on an idle machine for final figures.

## Notes on inputs
- **Model weights:** Gemma 3 1B-it from `unsloth/gemma-3-1b-it`, an ungated mirror whose file sizes match Google's gated original exactly. Parity is measured on identical weights for both implementations, so provenance doesn't affect results. Gemma's licence terms apply either way.
- **SAEs:** `google/gemma-scope-2-1b-it`, residual stream, layers 7/13/22, 16k wide.
- **Fast runtime:** `ggml-org/gemma-3-1b-it-GGUF` (f16, Q4_K_M), llama.cpp b11537, hash-checked.
- **Toolchains (per-user, in `vendor/` or the home folder, nothing system-wide except the Tauri apt packages):** uv 0.13 and Rust 1.99 (home folder); Chrome for Testing 155, Temurin JDK 25.0.4, GraalVM CE 25.0.4 and Maven 3.9.16 (`vendor/`, each checked against its published hash where one exists).
- **Location:** weights stay in the HF cache (spec §19); nothing is copied into the project.

## Layout
```
spike/
  README.md                this file
  pyproject.toml, uv.lock  pinned environment (torch, tokenizers; transformers only as the parity reference)
  src/rastai_spike/        the in-house code
    gemma3.py              Gemma 3 runtime: config, blocks, observation points, KV cache, static decode
    safetensors_reader.py  in-house safetensors reader (header validated before use)
    safetensors_writer.py  in-house safetensors writer
    decoding.py            static-shape decoding, CUDA-graph capture, live panel packets
    lenses.py              logit lens, attention capture, entropy, SAE features
    sae.py                 JumpReLU SAE runtime for Gemma Scope 2
    lora.py                LoRA adapters (float32 master weights, bf16 compute)
    training.py            SFT loop (answer-only loss, gradient checkpointing)
    evaluation.py          keyword checks and drift KL
    llama_server.py        managed llama-server adapter (the Fast runtime)
    tokenizer_corpus.py    the 162-case tokenizer stress corpus (experiments 04 and 07)
    bakeoff_fixtures.py    the 33-file safetensors corpus and tokenizer fixtures (experiment 07)
    generation.py, prompts.py, hf_cache.py, reporting.py
  canvas/                  the in-house WebGL2 canvas test page (experiment 06)
  webview-harness/         a tiny Rust program that opens a page in Tauri's webview (wry/tao)
  bakeoff/rust, bakeoff/java   the same backend components in both languages (experiment 07)
  experiments/             one script per spike item, numbered; each writes results/<name>.{md,json}
  data/                    Teach it training data and prompt sets
  results/                 the measurements (markdown summary + full JSON per experiment)
  artifacts/               generated files, e.g. the trained adapter (git-ignored)
  vendor/                  pinned third-party binaries and toolchains (git-ignored)
```

## Running
```
cd spike
uv sync                                   # Python 3.12 environment from uv.lock
uv run python experiments/01_gemma_parity.py
uv run python experiments/02_live_lens_overhead.py
...
```
Models download into the HF cache with `uv run hf download <repo>`. The repo names are at the top of each experiment.
