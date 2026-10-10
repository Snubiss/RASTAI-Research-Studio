# Architecture Decision Records

Decisions already made for Rastai Research Studio. Build from them; don't re-litigate them without a new ADR.

- **0001–0015** are the ADRs listed in spec §30, in that order.
- **0016–0023** record decisions made since: the spike's results, the owner's answers and spec v5.2–v5.3.
- **0024–0032** record other spec decisions that an implementer would otherwise reopen.

Sources of truth, in order: [the spec (v5.3)](../spec/rastai_spec_v5.md), [the spike results](../../spike/README.md), [the shared contracts](../architecture/contracts.md). The ADRs use the contracts' names exactly.

## Index

| # | Title | Status |
|---|---|---|
| [0001](0001-process-layout.md) | One binary, an embedded node, jobs in their own process groups | Accepted, 2026-10-10 |
| [0002](0002-durability-worker-event-logs.md) | Workers own their event logs; the database is an index | Accepted, 2026-10-10 |
| [0003](0003-storage-sqlite-vault-blobs.md) | SQLite index, a separate vault, a BLAKE3 blob store, and the HF cache in place | Accepted, 2026-10-10 |
| [0004](0004-schema-ownership.md) | Protobuf owns the wire, rastai-formats owns documents, everything else is generated | Accepted, 2026-10-10 |
| [0005](0005-command-layer.md) | One generic Execute endpoint over a command registry | Accepted, 2026-10-10 |
| [0006](0006-ui-transport-http-only.md) | The UI talks to the daemon over HTTP only; Tauri is just a shell | Accepted, 2026-10-10 |
| [0007](0007-local-api-security.md) | Loopback plus socket, a 0600 token file, Host and Origin checks, scoped tokens | Accepted, 2026-10-10 |
| [0008](0008-gateway-owns-tokenization.md) | The gateway owns chat templates and tokenization; runtimes get token IDs | Accepted, 2026-10-10 |
| [0009](0009-python-environments-uv.md) | uv, Python 3.12, one lockfile per backend, no runtime installs | Accepted, 2026-10-10 |
| [0010](0010-gpu-sharing-vram-leases.md) | VRAM leases in the node; Rastai manages its own llama-server | Accepted, 2026-10-10 |
| [0011](0011-remote-transport-ssh-first.md) | OpenSSH for remote nodes now; iroh pairing codes in Phase 2 | Accepted, 2026-10-10 |
| [0012](0012-build-in-house-policy.md) | Build in-house by default; external code only in five categories, always wrapped | Accepted, 2026-10-10 |
| [0013](0013-unified-model-runtime.md) | ModelSpec is the Inspectable runtime | Accepted, 2026-10-10 |
| [0014](0014-platform-tiers.md) | Linux NVIDIA, AMD and CPU at tier 1; macOS and Windows at tier 2 with portability rules | Accepted, 2026-10-10 |
| [0015](0015-plugins-and-licensing.md) | Plugins: one manifest, public API only, WASM deferred, plugins under any license | Accepted, 2026-10-10 |
| [0016](0016-backend-language-rust.md) | The backend stays Rust after the Java bake-off | Accepted, 2026-10-10 |
| [0017](0017-definition-of-done-l0-l3.md) | Definition of done: four levels, L0 to L3 | Accepted, 2026-10-10 |
| [0018](0018-license-apache-dco-trademark.md) | Apache-2.0, contributions under a DCO, and a trademark policy for the name | Accepted, 2026-10-10 |
| [0019](0019-in-house-webgl2-canvas.md) | An in-house WebGL2 renderer for the Designer canvas and all visuals | Accepted, 2026-10-10 |
| [0020](0020-evaluation-first-party-plugin.md) | Evaluation is a first-party plugin that tests models like code | Accepted, 2026-10-10 |
| [0021](0021-technique-coverage.md) | Technique coverage: MoE, MLA, ALiBi, AttnRes, the RLHF chain and GSPO are in scope | Accepted, 2026-10-10 |
| [0022](0022-ppo-one-base-model-in-vram.md) | PPO keeps one base model in VRAM; everything else is an adapter or a head | Accepted, 2026-10-10 |
| [0023](0023-owner-code-style.md) | Code style: simple functions and plain data; polymorphism yes, obscure abstraction no | Accepted, 2026-10-10 |
| [0024](0024-no-pickle-checkpoints.md) | No pickle: weights are safetensors, checkpoints are safetensors plus JSON | Accepted, 2026-10-10 |
| [0025](0025-two-runtimes-per-model.md) | Two runtimes per local model: Inspectable and Fast, with replay twins | Accepted, 2026-10-10 |
| [0026](0026-live-panel-in-decode-step.md) | The live panel runs inside a CUDA-graph decode step, with a fused logit lens | Accepted, 2026-10-10 |
| [0027](0027-replay-from-token-ids.md) | Replays recompute from token IDs; exact replays pin the attention kernel | Accepted, 2026-10-10 |
| [0028](0028-teach-it-anchors-and-drift.md) | Teach it trains with anchors by default, and Verify always reports drift | Accepted, 2026-10-10 |
| [0029](0029-strict-safetensors-contract.md) | An in-house safetensors reader and writer with one strict contract in Rust and Python | Accepted, 2026-10-10 |
| [0030](0030-diagnostics-catalog.md) | One diagnostics catalog; typed errors inside, coded diagnostics at the edge | Accepted, 2026-10-10 |
| [0031](0031-sandbox-manager.md) | An in-house sandbox manager over the Docker Engine HTTP API | Accepted, 2026-10-10 |
| [0032](0032-document-files-and-validation.md) | Document files are JSON with a shared header, parsed once, validated in four layers | Accepted, 2026-10-10 |

## By topic

| Topic | ADRs |
|---|---|
| Processes, durability, storage | 0001, 0002, 0003, 0024 |
| Contracts, formats and schemas | 0004, 0005, 0029, 0030, 0032 |
| UI and security | 0006, 0007, 0019 |
| Inference and runtimes | 0008, 0010, 0013, 0025, 0026, 0027 |
| Training and evaluation | 0020, 0021, 0022, 0028 |
| Platforms, environments, remote | 0009, 0011, 0014, 0031 |
| Policy and process | 0012, 0015, 0016, 0017, 0018, 0023 |

## Notes since acceptance

Changes to accepted ADRs that don't reopen a decision. "Illustration" and "path" notes are already in the ADR, marked *Note since acceptance*. "Pending" rows are carried by ADR-0034, which plan task M1-01 writes; that PR sets each old ADR's status line to "superseded in part by ADR-0034" and marks the row done.

| ADR | What changed | Kind | State |
|---|---|---|---|
| [0001](0001-process-layout.md) | Decision 5, reattach: the node waits for the worker's `WorkerHostService.Register`, checks BLAKE3(token) against `job.json` `job_token_hash` (with pid, `boot_id` and `process_start_ticks`), then calls `Hello` (plan §10 rows 5 and 6) | superseded in part by ADR-0034 | pending (M1-01) |
| [0004](0004-schema-ownership.md) | The code example uses the control-plane layout's names: `documents/eval.rs`, `SharedHeader`, `TypeDescription { name, schema_id, doc, fields }`, `list_of(FieldKind::Type(...))` | illustration | done |
| [0005](0005-command-layer.md) | The dispatch example returns `Result<CommandOutput, Vec<Diagnostic>>` and calls `parse_arguments(command_id, arguments_json)` (contracts §6.2) | illustration | done |
| [0006](0006-ui-transport-http-only.md) | Decision 4: the shell starts the daemon only through `rastai open --print-url`; file associations reach the web app as a `#/open?path=` fragment and the web app runs `file.open` (plan §10 row 2) | superseded in part by ADR-0034 | pending (M1-01) |
| [0007](0007-local-api-security.md) | Decisions 3 and 4: sign-in through `rastai open --print-url` and `token.login_code.create`; any loopback Host on any port with Origin equal to `http://` plus Host; a too-open `owner.token` refuses the start (plan §10 rows 2, 22 and 36) | superseded in part by ADR-0034 | pending (M1-01) |
| [0020](0020-evaluation-first-party-plugin.md) | Registry names are namespaced: `eval.drift`, `eval.pass_at_k` (plan §10 row 17) | illustration | done |
| [0029](0029-strict-safetensors-contract.md) | The Rust reader is `crates/rastai-formats/src/safetensors/reader.rs` | path | done |
| [0032](0032-document-files-and-validation.md) | The daemon-free CLI validator is the local command `rastai validate <file>`; `rastai file validate` is the daemon-backed verb for `file.validate` | path | done |

## Writing a new ADR

- **Number:** the next free four-digit number. File name `NNNN-short-title.md`, lowercase with hyphens.
- **Sections, in this order:**
  - Status;
  - Context (the problem and the forces, with spike numbers where they exist);
  - Decision (specific and testable, ending with **Checked by:** the tests that prove it);
  - Consequences (good, and costs);
  - Alternatives considered (a table: alternative, why not);
  - Links (spec sections, contracts sections, spike results, related ADRs).
- **Length:** about one screen. Bullets and tables, plain English, no filler.
- **Names:** use [contracts.md](../architecture/contracts.md) names exactly. A new shared name goes into contracts.md first, in its own PR.
- **Status values:** `Proposed`, `Accepted, <date>`, `Superseded by ADR-NNNN, <date>`. An accepted ADR is never edited to change its decision. Write a new one that supersedes it, and update the old one's status line and this index in the same PR.
- **Fixing an example or a path** in an accepted ADR is allowed when the decision stays the same: mark it *Note since acceptance (date)* in the ADR and add a row to the table above.
- **Diagrams:** Mermaid `flowchart`, `sequenceDiagram` or `stateDiagram-v2` only, with quoted labels. Follow each diagram with prose: the steps that aren't obvious, what is persisted where, and the failure branches.
