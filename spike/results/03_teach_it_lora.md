# Experiment 03: Teach it (in-house LoRA while the chat model stays loaded)

**Overall: PASS**  (time: True, memory: True, held_out: True, no_regressions: True)

Device: cuda:0 NVIDIA GeForce RTX 5080 (sm_120), capped at 8 GiB. Chat model and training copy both resident.

| Measure | Value |
|---|---|
| Teach it, end to end (load copy, attach LoRA, tokenize, train, save) | **9.6 s** (limit 180 s) |
| Of which training steps | 5.7 s for 40 steps |
| Training data | 40 Q&A pairs, 980 answer tokens, 8 epochs |
| LoRA | rank 16, alpha 32, all 7 projections × 26 layers, 13,045,760 trainable parameters |
| Peak VRAM (both models + training) | 4.69 GiB |
| Adapter file | 52.2 MB, round-trips through our reader: True |
| Held-out accuracy, before → after | 20% → **80%** |
| General accuracy, before → after | 100% → 100% |
| Adapter switched off reproduces the base model | True |

Loss and chat speed per epoch (the chat model answering between epochs):

| Epoch | Mean loss | Chat tokens/s |
|---|---|---|
| 0 | 4.762 | 69.0 |
| 1 | 2.873 | 66.8 |
| 2 | 2.054 | 68.1 |
| 3 | 1.419 | 70.8 |
| 4 | 0.779 | 70.5 |
| 5 | 0.274 | 71.3 |
| 6 | 0.107 | 69.7 |
| 7 | 0.046 | 70.1 |

## Held-out questions (new phrasings, never trained on)

**Tell me what Rastai Research Studio is for.**
- Before (✗): Rastai Research Studio (RRS) is a fascinating and rapidly evolving project developed by Rastai, one of the most prominent and influential Minecraft streamers and content creators. It's essentially a **massive, incredibly detailed, and constantly updated
- After (✓): Rastai Research Studio is for designing, training, fine-tuning, serving and studying neural networks.

**In which programming language is the Rastai control plane implemented?**
- Before (✗): The Rastai control plane is primarily implemented in **Go**.

Here's a breakdown of why and some key details:

* **Go's Strengths for Distributed Systems:** Go is renowned for its concurrency features, making it well
- After (✓): The Rastai control plane is implemented in Rust.

**Which UI framework powers the Rastai interface?**
- Before (✓): The Rastai interface is powered by **React**.

Specifically, it leverages a combination of React components and a custom UI library built on top of React.  While React is the core framework, the Rastai UI library provides a lot of
- After (✓): Rastai is a Tauri desktop shell, a React interface with Tauri's React Webview API for Linux and macOS.

**Which section of Rastai lets me look at a model's internals?**
- Before (✗): Okay, let's break down how to access the internals of a Rastai model. It's a bit involved, but here's a breakdown of the methods and where to find the information:

**1. The Official Rast
- After (✗): Design or import, observe the graph.

**What extension do saved Rastai network designs have?**
- Before (✗): Okay, this is a fascinating and somewhat complex question! The Rastai network designs, particularly the "Rastai Network," are a unique and highly secretive project. There's no single, universally agreed-upon "extension" in the
- After (✓): Saved Rastai network designs are .rrsmodel files.

**What format do Rastai conversation replays come in?**
- Before (✗): Rastai's conversation replays are a fascinating and somewhat unique phenomenon! They're not in a standard, easily digestible format like a video or audio file. Instead, they're presented as a **complex, layered, and
- After (✗): Rastai recomputes the model's internals from the logged token IDs with one forward pass.

**Is Rastai free to use?**
- Before (✓): Okay, let's break down the situation with Rastai and his streaming and content creation. It's a complex and somewhat controversial topic. Here's a detailed look:

**1. Initial Free Streaming (2018
- After (✓): Yes. Rastai is free; businesses pay for Teams, Enterprise, Rastai Cloud and support, and creators sell items in the marketplace.

**Which database engine holds Rastai's conversations?**
- Before (✗): This is a fascinating and somewhat mysterious question! As of today, November 2, 2023, **Rastai's conversations are primarily stored in the Discord server.**

Here's a breakdown of what's
- After (✓): Rastai's conversations are held in SQLite: the local SQLite database.

**What is the name of Rastai's graph format for models?**
- Before (✗): Rastai uses the **`graph`** format for models.

It's a highly efficient and versatile format designed specifically for representing and exchanging models, particularly within the Minecraft community.  It's a significant improvement over previous formats
- After (✓): Rastai's graph format for models is a ModelSpec.

**How do I connect my own GPU server to Rastai?**
- Before (✗): Connecting your own GPU server to Rastai is a complex undertaking, requiring significant technical expertise and a deep understanding of blockchain technology, networking, and security. It's not a simple plug-and-play process. Here's a breakdown
- After (✓): Pair the machine over SSH with rastai node pair, and Rastai runs jobs on it as a node.

## General knowledge (must not regress)

- What is the capital of France? — before ✓, after ✓: Paris is the capital of France.
- What is 12 multiplied by 12? — before ✓, after ✓: 12 multiplied by 12 is 144.
- Who wrote Romeo and Juliet? — before ✓, after ✓: Romeo and Juliet was written by William Shakespeare.
- In one sentence, why is the sky blue? — before ✓, after ✓: The sky is blue because it's filled with tiny particles of air, like dust and water, that scatter blue light in all dire
- What is the chemical symbol for gold? — before ✓, after ✓: The chemical symbol for gold is Au.

## Environment

```
recorded_at: 2026-10-10T08:25:14+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
