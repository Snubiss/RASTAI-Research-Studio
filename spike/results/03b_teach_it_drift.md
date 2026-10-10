# Experiment 03b: Teach it without side effects

**Success: True** (an anchored recipe keeps held-out ≥ 70% and cuts drift by ≥ 50% vs A)

Device: cuda:0 NVIDIA GeForce RTX 5080 (sm_120), capped at 8 GiB. Anchor answers from the base model took 57.4 s (once per base model; cacheable, so not counted in Teach it time).

| Recipe | Teach it time | Steps | Last-epoch loss | Held-out accuracy | General keywords | Mean drift KL | Drift vs A | Unchanged answers (of 20) |
|---|---|---|---|---|---|---|---|---|
| A: facts only, 8 epochs | 6.0 s | 40 | 0.046 | 80% | 100% | 2.1404 | 100% | 0 |
| B: facts + anchors, 8 epochs | 11.4 s | 80 | 0.019 | 90% | 100% | 0.3584 | 17% | 0 |
| C: facts + anchors, 4 epochs | 5.8 s | 40 | 0.275 | 80% | 100% | 0.2708 | 13% | 0 |

**Who wrote Romeo and Juliet?** (experiment 03's recipe invented an editor here)

- Base model: The author of Romeo and Juliet is **William Shakespeare**. 

He wrote the play in the late 16th century. 

While it's a collaborative work, Shakespeare is widely credited as the primary author.
- A: facts only, 8 epochs: Romeo and Juliet was written by William Shakespeare.
- B: facts + anchors, 8 epochs: Romeo and Juliet was written by **William Shakespeare**. 

It's a tragedy about two young lovers from feuding families, the Montagues and the Capulets.
- C: facts + anchors, 4 epochs: Romeo and Juliet was written by **William Shakespeare**. 

It's a cornerstone of English literature and a remarkably complex and enduring story.

Held-out answers per recipe:

**Tell me what Rastai Research Studio is for.**
- A (✓): Rastai Research Studio is for designing, training, fine-tuning, serving and studying neural networks.
- B (✓): Rastai Research Studio is for designing, training, fine-tuning, serving and studying neural networks.
- C (✓): Rastai Research Studio is Rastai's interpretability tool: turn a model into a model interpretability analysis, and see inside it.

**In which programming language is the Rastai control plane implemented?**
- A (✓): The Rastai control plane is implemented in Rust.
- B (✓): The Rastai control plane is implemented in Rust.
- C (✓): Rastai's control plane is implemented in Rust.

**Which UI framework powers the Rastai interface?**
- A (✓): Rastai is a Tauri desktop shell, a React interface with Tauri's React Webview API for Linux and macOS.
- B (✓): The UI framework powering the Rastai interface is React.
- C (✓): Rastai's interface is powered by Tauri, a Tauri fork that runs as a Tauri Tauri desktop shell.

**Which section of Rastai lets me look at a model's internals?**
- A (✗): Design or import, observe the graph.
- B (✓): Open the Psychology section. It shows the logit lens, attention patterns, confidence and SAE concept features for every token.
- C (✗): In Rastai, look at the Model Section. It shows every concept in the model: shapes, concepts, splices, and SAEs.

**What extension do saved Rastai network designs have?**
- A (✓): Saved Rastai network designs are .rrsmodel files.
- B (✓): Saved network designs are saved as .rrsmodel files.
- C (✓): Saved network designs are saved as .rrsmodel files, which are JSON.

**What format do Rastai conversation replays come in?**
- A (✗): Rastai recomputes the model's internals from the logged token IDs with one forward pass.
- B (✓): Rastai conversation replays are saved as .rrsreplay files, which are JSON.
- C (✓): Rastai conversation replays are saved as .rrsreplay files, which are plain .rrsreplay zip archives.

**Is Rastai free to use?**
- A (✓): Yes. Rastai is free; businesses pay for Teams, Enterprise, Rastai Cloud and support, and creators sell items in the marketplace.
- B (✓): Rastai is free for everyone, including companies. Businesses pay only for team features, enterprise features, hosted cloud and support.
- C (✓): Yes. Rastai is free for everyone, including companies.

**Which database engine holds Rastai's conversations?**
- A (✓): Rastai's conversations are held in SQLite: the local SQLite database.
- B (✓): Rastai's conversations are stored in the SQLite database named `conversation.db`.
- C (✓): Rastai's conversations are stored in vault.db, a separate SQLite database that never leaves the vault.

**What is the name of Rastai's graph format for models?**
- A (✓): Rastai's graph format for models is a ModelSpec.
- B (✗): Rastai's graph format for models is called **ModelGraph**.
- C (✗): Rastai's graph format for models is called ModelGraph.

**How do I connect my own GPU server to Rastai?**
- A (✓): Pair the machine over SSH with rastai node pair, and Rastai runs jobs on it as a node.
- B (✓): Pair the GPU server over SSH with rastai node pair, and Rastai will connect.
- C (✓): Connect your own GPU server to Rastai via SSH, and Rastai manages the SSH session.

## Environment

```
recorded_at: 2026-10-10T08:24:25+00:00
python: 3.12.15
torch: 2.14.1+cu130
cuda_runtime: 13.0
gpus: ['0, NVIDIA GeForce RTX 5080, 595.91.07, 16303 MiB', '1, NVIDIA GeForce RTX 3090, 595.91.07, 24576 MiB']
kernel: 7.0.0-34-generic
```
