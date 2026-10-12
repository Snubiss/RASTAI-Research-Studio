# Rastai Research Studio: Design Docs

**Status:** v1.2, 2026-10-11. Pre-build. Built on spec v5.4 and contracts.md v1.2. The entry point to the design: what to read, in what order, and where each kind of decision lives. Nothing here is new; every rule below points to the doc that owns it.

The design is about 28,000 lines across 13 documents and 38 ADRs. Nobody needs all of it for one task. Read the core once (section 1), then only the layout and flow docs for the part you're building.

---

## 1. Reading order

**Once, before any task** (new engineer or coding agent):

| Step | Read | What you take away |
|---|---|---|
| 1 | Root `CLAUDE.md` | The hard rules and the owner's code style. Two minutes. |
| 2 | Spec `docs/spec/rastai_spec_v5.md` §0, §3, §5, then Part II (§16–§30) | What is decided, the core loop, the L0–L3 bar, the architecture, the §27 dependency allowlist, the §29 repo layout |
| 3 | `spike/README.md` (verdict table and "Spec changes") | The measured numbers the design rests on: parity, live-panel cost, token parity, Teach it, canvas, Rust vs Java |
| 4 | `contracts.md` §1–§3, §12 and §15–§16 | The repo tree, every component and its allowed imports, the processes and who talks to whom, the naming and code rules, the code-level names, and the answer to every plan §10 conflict |
| 5 | `docs/adr/README.md`, then ADR-0023, 0036, 0012, 0004, 0038, 0001, 0002, 0013 | Code style, plain JavaScript for the frontend, in-house policy, schema ownership, CI-enforced names and schemas, process layout, durability, the unified runtime |
| 6 | Plan `docs/plan/implementation-plan.md` §1–§3 (including §3.5, the agent build protocol, and §3.6, closing by independent verification) and §10 | Milestones, the two-lane workflow, the definition of done per PR, how agents are briefed and how work is closed, and §10, the 44 cross-doc conflicts, each answered in contracts §16 |

**Per task:**

| Step | Read |
|---|---|
| 7 | The plan's task entry (§4.8 for M1, §5.8 for M2, §6 epics for later) and its issue in `backlog.json` |
| 8 | The layout doc section for the crate or package you touch (section 2 below) |
| 9 | The flow doc section for the feature you build: its diagram, failure table and "persisted" notes |
| 10 | `contracts.md` §4–§11 as reference for every wire message, document, command, event, table, state, code or setting you use |
| 11 | The ADRs the layout or flow section cites, before changing anything they decide |

## 2. The documents

| Document | Covers | Read it when |
|---|---|---|
| `CLAUDE.md` (repo root) | Map of the repo, hard rules, code style, DCO sign-off | Always first |
| `docs/spec/rastai_spec_v5.md` | Product and technical scope v5.4 (in line with the docs): Part I product (§0–§15), Part II architecture (§16–§30), Part III plan (§31–§38) | You need to know *what* and *why* at product level. `rastai_spec_v4.md` is history; never build from it. |
| `spike/README.md`, `spike/results/` | Seven measured experiments and the spec changes they caused; the prototype code to port | You port spike code, or a design choice cites "(spike)" |
| `architecture/contracts.md` | Every shared name: repo tree, components and layering, processes, protobuf services, documents, commands (about 150), events, tables, ids, state machines, error catalog ranges, settings, conventions, glossary, open questions | You name anything that crosses a crate, package or process boundary |
| `architecture/contracts-changes/` | The change entries for contracts.md after v1.2 item 10: one file per task, named by its task id (`M1-25.md`), holding that task's entries in order | Your PR changes contracts.md: add your entry in your own task's file, never another task's |
| `layout/rust-control-plane.md` | Workspace root, `rastai-diag`, `rastai-formats` (WASM + PyO3), `rastai-core`, `rastai-store`, `rastai-mcp`, `rastai-server`, `rastai-cli`, `rastai-archive` (the optional long-term archive, M11, §14); command surfaces, HTTP API versioning, plugin host, event indexing | You work on the daemon, documents, storage, the archive, commands, CLI or MCP |
| `layout/rust-node-gateway-proto.md` | `proto/`, `rastai-transport`, `rastai-sandbox`, `rastai-gateway`, `rastai-node` | You work on the wire, SSH, sandboxes, tokenization, adapters, jobs, leases, environments or llama-server |
| `layout/python.md` | `rastai_models`, `rastai_lens`, `rastai_data`, `rastai_train`, `rastai_rl`, `rastai_worker`, `rastai`, `rastai_eval`, `python/envs/` | You work on compute: the runtime, readouts, data, training, RL, the job runtime or the SDK |
| `layout/frontend-and-plugins.md` | `apps/web`, `apps/desktop`, the JavaScript packages (plain JS + JSX, JSDoc, ADR-0036), the three plugin SDKs (JavaScript, Python, Rust), plugin templates, the iframe bridge, theme tokens and the locked security UI, the Psychology and Evaluation plugins | You work on UI, the Tauri shell, or anything a plugin touches |
| `flows/platform.md` | Install and First Run, daemon start and reattach, the `Execute` path, local API security, job lifecycle, event-log durability and the kill -9 matrix, plugins, storage housekeeping and the long-term archive (M11), self-update, projects, crash reports | You build infrastructure every section relies on |
| `flows/inference-psychology.md` | Chat turns through the gateway, runtime toggle and replay twins, API chats and proxy replay, model loading and VRAM leases, the decode step and live panel, replay and analyses, interventions and branching, remote models, conversation flags | You build Inference or Psychology |
| `flows/training-evaluation.md` | Data import, recipe selection, a training run, checkpoints and preemption, chains, Teach it, Evaluation suites and metrics, auto-run and Elo, RLHF, model diff and SAE fit check, export, log import | You build Training or Evaluation |
| `flows/designer-gym-remote.md` | A: HF import, compile and parity, edits and live stats, Eject, ONNX, `rastai.load()`, layer placement. B: harnesses, gym runs, rollouts, peek, step mode, sandboxes. C: SSH pairing, handshake, remote jobs, drops, transfers, headless, node update | You build the Designer, the Agent Gym or remote nodes |
| `docs/adr/README.md` + ADR-0001–0038 | Why each architectural decision was made, alternatives rejected, the tests that check it; notes since acceptance | You're about to change or question a decision |
| `docs/plan/implementation-plan.md` | M1–M11 (M11, long-term storage, comes after the 90-day plan), tasks for M1 and M2, epics for M3–M11, the L0–L3 feature table, GitHub setup, schedule risks, conflicts (§10), open questions | You pick, size or review work |
| `docs/plan/backlog.json` | One GitHub issue per M1/M2 task and per later epic, generated from the plan's tables | You open an issue (M1-07 syncs it to GitHub) |

Planned, not yet written: `docs/security/threat-model.md` (M1-56) and the generated `docs/reference/` (commands, events, error catalog, extension points, settings).

## 3. How the docs relate

- **Spec → contracts → layouts → flows → plan.** Each layer only refines the one before it.
  - The **spec** says what the product does and which architecture it uses.
  - **contracts.md** fixes the names and boundaries every other doc uses.
  - **Layout docs** say where code lives: the tree, public API signatures, dependencies, tests and build-order step of each crate and package. One doc owns each component.
  - **Flow docs** say how components cooperate at run time: sequence diagrams, state machines, what is persisted where, and the failure branches. They cross component lines; they don't redefine components.
  - The **plan** turns layouts and flows into ordered, sized tasks with exit gates.
- **ADRs record why.** A doc applies a decision; the ADR is where it is argued and checked. Accepted ADRs are never edited to change a decision (`docs/adr/README.md`).
- **The spike is evidence.** Numbers marked "(spike)" come from `spike/results/`. Spike code is ported, never imported (contracts §1 port map).
- **contracts.md v1.2 is the only list of shared names** (M1-01). Layout and flow docs keep no "New names" section. A missing wire, document or command shape goes into contracts first, in its own PR; any other missing name goes in the same PR as the code. From M1 `check-names` and `check-schema-diff` enforce this in CI (ADR-0038, contracts §12.6).
- **Every layout and flow doc ends with "Open questions".** Each carries a default that the docs follow until the owner decides.

**When two sources disagree:**

| Disagreement about | Winner |
|---|---|
| Scope: what the product does | The spec (v5.4 is in line with the docs). A doc that needs a spec edit raises an open question with a default; the owner makes the edit. |
| A measured number | `spike/results/` |
| A decision an ADR records | The ADR. ADR-0034 supersedes parts of ADR-0001, 0006 and 0007; ADR-0035 supersedes ADR-0003 decision 6 in part and amends ADR-0002 decision 7; ADR-0036 supersedes ADR-0023's TypeScript section and ADR-0019 decision 5 in part; ADR-0037 supersedes ADR-0029 decision 1 in part; ADR-0038 puts `check-names` and `check-schema-diff` in M1 and adds the independent-verification rule. |
| A name, or anything listed in plan §10 | contracts.md v1.2 (§16 answers every plan §10 row) |
| Something inside one crate or package (file, function, constant) | That component's layout doc (plan §10 row 7). Catalog numbers are the exception: every one is listed in contracts §10.3 first (plan §10 row 8). |
| Anything else | Nobody yet. Add a row to the end of plan §10 (rows are append-only) and follow its default. |

## 4. The system on one screen

Processes and the main data paths. contracts §3 has the full process table and the auth on every arrow.

```mermaid
flowchart TD
  SHELL["Tauri shell"]
  UI["Web app: browser or Tauri webview"]
  EXT["CLI, Navi, scripts, MCP clients"]
  subgraph DAEMON["Daemon process: rastai serve"]
    SRV["rastai-server: Execute, event bus, scheduler, plugin host"]
    GW["rastai-gateway: chat templates, tokenization, adapters"]
    DB[("rastai.db: the index")]
    VAULT[("vault.db: conversations, gateway log")]
    BLOBS[("Blob store, BLAKE3 names")]
    EN["Embedded node: rastai-node"]
  end
  RN["Remote node: rastai node serve"]
  W["Python worker jobs: rastai_worker plus engines"]
  PM["Plugin Python modules"]
  LOG[("runs/run_id/events.log")]
  LS["llama-server: Fast runtime"]
  SB["Sandbox containers: Docker or Podman"]
  HF["Hugging Face Hub and the HF cache"]
  API["Third-party model APIs"]
  ARC[("Owner's MySQL or MariaDB archive, optional")]
  SHELL -->|"runs rastai open"| SRV
  SHELL -->|"opens the URL with a login code"| UI
  UI -->|"Connect over HTTP, loopback"| SRV
  SRV -->|"BusEvent stream"| UI
  EXT -->|"daemon.sock, /mcp"| SRV
  EXT -->|"/v1 endpoints, scoped token"| GW
  SRV --> GW
  SRV -->|"one writer thread"| DB
  GW -->|"LogSink, one writer thread"| VAULT
  SRV --> BLOBS
  GW -->|"HTTPS, keys from the keychain"| API
  SRV -->|"gRPC over node.sock"| EN
  SRV -->|"gRPC over SSH, both directions"| RN
  GW -->|"token IDs in, token IDs out"| EN
  EN -->|"WorkerService, per-job socket"| W
  EN --> PM
  W -->|"appends durable events"| LOG
  LOG -->|"tailed"| EN
  EN -->|"WatchRunEvents, then ack"| SRV
  EN -->|"token IDs, loopback"| LS
  EN -->|"Docker HTTP API"| SB
  EN -->|"downloads"| HF
  W -->|"reads weights in place"| HF
  SRV -->|"MySQL over TLS, one way, optional, M11"| ARC
```

**Processes.** One binary, `rastai`, is the CLI, the daemon and the node (ADR-0001). The embedded node runs in the daemon process but is reached over `node.sock`, exactly like a remote node, so local and remote can't drift. Each Python job runs in its own process group and survives daemon and node restarts. A remote node has the same arrows to its own workers, llama-server, containers and HF cache.

**The five data paths:**
1. **Commands.** Every action is `CommandService.Execute(command_id, arguments_json)` from the UI, CLI, MCP or a plugin. The daemon parses the arguments with `rastai-formats`, checks the principal's scope, dispatches, and records a `command_journal` row. Sync commands return a result; job commands return a `job_id` at once (platform §3).
2. **Chat.** The gateway renders the chat template and tokenizes; runtimes only see token IDs (ADR-0008). The node routes to the Inspectable worker or llama-server. Tokens stream back as ephemeral `chat.token` events. The assistant message and its `gateway_log` row are written to `vault.db` in one transaction after generation; incognito chats are never written (inference §1).
3. **Durable events.** A job is the only writer of its `events.log` (CRC-framed, fsync on every state change). The node tails it; the daemon indexes each record into `rastai.db` and stores `acked_sequence`. `rastai.db` can be rebuilt from the logs and the blob store (ADR-0002, platform §6).
4. **Live readouts.** The live panel's packet (about 8.3 KB per token) is built inside the CUDA-graph decode step (ADR-0026), sent over `WatchLive`, and fanned out as an ephemeral bus event on one Connect stream per window. Ephemeral events may be dropped under load (frontend §2.2).
5. **Weights and files.** Weights stay in the HF cache and are never copied. Everything else large is a blob named by its BLAKE3 hash; events carry only hashes. Remote nodes pull from the source; only private local data is uploaded; LoRA adapters always sync back (ADR-0003, designer-gym-remote C7).

**Optional long-term archive (M11, ADR-0035).** SQLite stays the live store. When the owner configures one, the daemon (never a node) replicates its event logs, index rows and blob metadata one way, with idempotent upserts, to the owner's own MySQL or MariaDB, and copies pinned checkpoints, LoRA adapters and dataset snapshots to a folder target. Conversations (and blobs derived from them) go only per project, opt-in, into encrypted tables. The data policy applies, and the egress badge shows during a sync (contracts §8.7, platform §8.5–§8.12).

**Failure branches:**
- **Daemon crash:** jobs keep running. On restart the node waits for each worker's `Register`, checks the token hash in `job.json`, calls `Hello`, and the daemon replays events from `acked_sequence`. Nothing durable is lost; a half-streamed chat reply is (plan §10 rows 5 and 23).
- **Worker crash or OOM:** the node classifies the exit cause; the job's last `job.state_changed` carries a coded `Diagnostic`, and the traceback goes to `stderr.log` (platform §5.5).
- **Node unreachable:** three missed 5 s heartbeats mark it `unreachable` and its jobs `lost`. On reconnect the daemon reconciles from `ListJobs` and the replayed log (designer-gym-remote C6).
- **UI disconnect:** the client resumes from its last `bus_sequence` if it's still in the daemon's 10,000-event buffer; otherwise `RRS-C0401` and it refetches state (contracts §7.2).
- **Torn log tail:** truncated on open. A bad frame mid-file is corruption, `RRS-S2002` (contracts §7.1).

## 5. Where is X decided?

| Question | Look in |
|---|---|
| What is this thing called? | contracts.md: glossary §13, code-level names §15, and §16 for anything plan §10 lists |
| Which folder or file does this code go in? | contracts §1 tree, then the owning layout doc's "Tree" subsection |
| Which crate or package may import which? | contracts §2.4 layering rules; each layout's "Boundary rules" |
| Can I add this dependency? | Spec §27 (v5.4) and contracts §2.5; the one wrapper file per library (contracts §2.4 rule 8) |
| Which process runs it, and how do processes talk? | contracts §3.1–§3.3; ADR-0001, 0006 |
| What does this protobuf message hold? | contracts §4 (§4.9 for the messages the layouts named); node layout §3.2 says why each exists |
| What does this document type hold, and how is it validated? | contracts §5; control-plane layout §4; ADR-0032 |
| Which commands exist, their scope and kind? | contracts §6; surfaces generated as in control-plane layout §10 |
| Which events exist, durable or ephemeral? | contracts §7; ADR-0002 |
| Where is it stored, which table, which path? | contracts §8; ADR-0003; control-plane layout §6 |
| What states can a job, run, model, lease, node, plugin or sandbox be in? | Names in contracts §9; diagrams in platform §5.2 and §7.2, inference §4.1–§4.2, training-evaluation §4.2 and §8.2, designer-gym-remote B6 and C2 |
| Which error code do I raise? | contracts §10.1 (ranges) and §10.3 (every assigned number; collisions settled in §10.2). A new number goes into §10.3 first. |
| What is this setting's default? | contracts §11 |
| What is this timeout or limit? | The owning layout doc; platform §12 and inference §0.3 collect the cross-cutting ones |
| How should the code look? | ADR-0023, ADR-0036 (plain JavaScript + JSX with short JSDoc, no TypeScript), contracts §12, `CLAUDE.md` |
| How does the live panel stay fast? | ADR-0026; Python layout §2.3 and §3; inference §5 |
| How do GPU memory and evictions work? | ADR-0010; node layout §7.7; inference §4 |
| How does the plugin API work? | ADR-0015; frontend layout §10–§13; control-plane layout §12; platform §7 |
| How is the local API secured? | ADR-0034 and ADR-0007 (as superseded in part by ADR-0034); platform §4; contracts §3.2 |
| How does the long-term archive work? | ADR-0035; contracts §8.7; control-plane layout §14; platform §8.5–§8.12 |
| Which theme tokens exist, and what can't a theme change? | contracts §5.6; frontend layout §2.10 |
| What survives a kill -9? | ADR-0002; platform §6.2 and the §6.5 matrix |
| Where does this spike file go? | contracts §1 port map; Python layout §13; each layout's "Ported from the spike" |
| Where do its tests go, and which test proves it? | contracts §1 (`tests/`) and §12.5 (fixtures); each layout's "Tests"; plan §7.3 feature level table |
| What do I build next, and how big is it? | Plan §2 and the task tables (§4.5, §5.5); `backlog.json` |
| When is a feature done? | ADR-0017 (L0–L3); plan §3.3 (per PR), §3.6 (closing an issue by independent verification) and §7 (per feature) |
| Why was it decided this way? | The ADR index, by topic |

## 6. Changing the design

- **A new shared name** goes into contracts.md. A wire, document or command shape lands first, in its own contract PR (spec §29). Any other name goes in the same PR as the code that uses it. `check-names` and `check-schema-diff` enforce this from M1 (ADR-0038, contracts §12.6).
- **A name registered before its code exists** carries the milestone that builds it, such as "(M4)" (contracts conventions). The PR that lands the name removes its marker.
- **A new decision** that someone would otherwise reopen gets an ADR (`docs/adr/README.md`, "Writing a new ADR"). An example or path fix in an accepted ADR is a "Note since acceptance", not a new decision.
- **A conflict between docs** gets a row at the end of plan §10 with a default; once settled, contracts §16 records the answer and the docs that lose get a fix.
- **A scope change** is a spec edit by the owner. Docs never change scope on their own; they raise an open question with a default.
- **Diagrams** use only Mermaid `flowchart`, `sequenceDiagram` and `stateDiagram-v2`, with quoted labels, and are followed by prose: the non-obvious steps, what is persisted where, and the failure branches.
