# Architecture Decision Records

Decisions already made for Rastai Research Studio. Build from them; don't re-litigate them without a new ADR.

- **0001–0015** are the ADRs listed in spec §30, in that order.
- **0016–0023** record decisions made since: the spike's results, the owner's answers and spec v5.2–v5.3.
- **0024–0032** record other spec decisions that an implementer would otherwise reopen.
- **0033–0037** record the owner decisions of 2026-10-10 (spec v5.4, contracts v1.1): the one `unsafe` exception, sign-in, the long-term archive, the frontend language and the borrowed decoders.

Sources of truth, in order: [the spec (v5.4)](../spec/rastai_spec_v5.md), [the shared contracts (v1.1)](../architecture/contracts.md), then the layout and flow docs, these ADRs and the plan. [The spike results](../../spike/README.md) are the source for measured numbers. The ADRs use the contracts' names exactly; contracts.md is the only list of names.

## Index

| # | Title | Status |
|---|---|---|
| [0001](0001-process-layout.md) | One binary, an embedded node, jobs in their own process groups | Accepted, 2026-10-10. Decision 5 superseded in part by 0034 |
| [0002](0002-durability-worker-event-logs.md) | Workers own their event logs; the database is an index | Accepted, 2026-10-10. Decision 7 amended by 0035 |
| [0003](0003-storage-sqlite-vault-blobs.md) | SQLite index, a separate vault, a BLAKE3 blob store, and the HF cache in place | Accepted, 2026-10-10. Decision 6 superseded in part by 0035 |
| [0004](0004-schema-ownership.md) | Protobuf owns the wire, rastai-formats owns documents, everything else is generated | Accepted, 2026-10-10 |
| [0005](0005-command-layer.md) | One generic Execute endpoint over a command registry | Accepted, 2026-10-10 |
| [0006](0006-ui-transport-http-only.md) | The UI talks to the daemon over HTTP only; Tauri is just a shell | Accepted, 2026-10-10. Decision 4 superseded in part by 0034 |
| [0007](0007-local-api-security.md) | Loopback plus socket, a 0600 token file, Host and Origin checks, scoped tokens | Accepted, 2026-10-10. Decisions 3 and 4 superseded in part by 0034 |
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
| [0019](0019-in-house-webgl2-canvas.md) | An in-house WebGL2 renderer for the Designer canvas and all visuals | Accepted, 2026-10-10. Decision 5 superseded in part by 0036 |
| [0020](0020-evaluation-first-party-plugin.md) | Evaluation is a first-party plugin that tests models like code | Accepted, 2026-10-10 |
| [0021](0021-technique-coverage.md) | Technique coverage: MoE, MLA, ALiBi, AttnRes, the RLHF chain and GSPO are in scope | Accepted, 2026-10-10 |
| [0022](0022-ppo-one-base-model-in-vram.md) | PPO keeps one base model in VRAM; everything else is an adapter or a head | Accepted, 2026-10-10 |
| [0023](0023-owner-code-style.md) | Code style: simple functions and plain data; polymorphism yes, obscure abstraction no | Accepted, 2026-10-10. TypeScript section superseded by 0036 |
| [0024](0024-no-pickle-checkpoints.md) | No pickle: weights are safetensors, checkpoints are safetensors plus JSON | Accepted, 2026-10-10 |
| [0025](0025-two-runtimes-per-model.md) | Two runtimes per local model: Inspectable and Fast, with replay twins | Accepted, 2026-10-10 |
| [0026](0026-live-panel-in-decode-step.md) | The live panel runs inside a CUDA-graph decode step, with a fused logit lens | Accepted, 2026-10-10 |
| [0027](0027-replay-from-token-ids.md) | Replays recompute from token IDs; exact replays pin the attention kernel | Accepted, 2026-10-10 |
| [0028](0028-teach-it-anchors-and-drift.md) | Teach it trains with anchors by default, and Verify always reports drift | Accepted, 2026-10-10 |
| [0029](0029-strict-safetensors-contract.md) | An in-house safetensors reader and writer with one strict contract in Rust and Python | Accepted, 2026-10-10. Decision 1 superseded in part by 0037 |
| [0030](0030-diagnostics-catalog.md) | One diagnostics catalog; typed errors inside, coded diagnostics at the edge | Accepted, 2026-10-10 |
| [0031](0031-sandbox-manager.md) | An in-house sandbox manager over the Docker Engine HTTP API | Accepted, 2026-10-10 |
| [0032](0032-document-files-and-validation.md) | Document files are JSON with a shared header, parsed once, validated in four layers | Accepted, 2026-10-10 |
| [0033](0033-unsafe-statvfs.md) | `unsafe` is allowed for one `statvfs` call, in `rastai-store/src/disk_space.rs` only | Accepted, 2026-10-10 |
| [0034](0034-login-codes-loopback-host-owner-token.md) | Sign-in with one-time login codes and origin-bound sessions; the loopback Host rule; a too-open owner token stops the daemon; Register before Hello | Accepted, 2026-10-10. Supersedes parts of 0001, 0006, 0007 |
| [0035](0035-mysql-long-term-archive.md) | An optional long-term archive in the owner's MySQL or MariaDB; SQLite stays the live store | Accepted, 2026-10-10. Supersedes 0003 decision 6 in part; amends 0002 decision 7 |
| [0036](0036-frontend-plain-javascript.md) | The frontend is plain JavaScript + React, with short JSDoc types checked by `checkJs` | Accepted, 2026-10-10. Supersedes 0023's TypeScript section and 0019 decision 5 in part |
| [0037](0037-borrowed-decoders-and-parser-fuzzing.md) | Borrowed decoders for DEFLATE and images; one implementation and a fuzz target for every in-house parser | Accepted, 2026-10-10. Supersedes 0029 decision 1 in part |

## By topic

| Topic | ADRs |
|---|---|
| Processes, durability, storage | 0001, 0002, 0003, 0024, 0033, 0035 |
| Contracts, formats and schemas | 0004, 0005, 0029, 0030, 0032, 0037 |
| UI and security | 0006, 0007, 0019, 0034, 0036 |
| Inference and runtimes | 0008, 0010, 0013, 0025, 0026, 0027 |
| Training and evaluation | 0020, 0021, 0022, 0028 |
| Platforms, environments, remote | 0009, 0011, 0014, 0031 |
| Policy and process | 0012, 0015, 0016, 0017, 0018, 0023, 0036 |

## Notes since acceptance

Changes to accepted ADRs. Each is marked in the ADR itself: a status line and a "Superseded in part" or "Amended" paragraph when a later ADR replaces part of a decision, or an italic *Note since acceptance* when the decision stands and only names, paths, examples or settled defaults changed. All rows are done.

| ADR | What changed | Kind |
|---|---|---|
| [0001](0001-process-layout.md) | Decision 5, reattach: the node waits for `WorkerHostService.Register`, checks BLAKE3(token) against `job.json` `job_token_hash` (with pid, `boot_id` and `process_start_ticks`), then calls `Hello` | superseded in part by 0034 |
| [0002](0002-durability-worker-event-logs.md) | Decision 7: a rebuild gains a step that pulls pruned runs from the archive; `settings` leaves the list of non-rebuildable tables (there is no settings table). Decision 1: a chain run's log has one writer, the chain driver; the daemon keeps `<data>/daemon/events.log` | amended by 0035; note |
| [0003](0003-storage-sqlite-vault-blobs.md) | Decision 6: still no server for the live store, but an optional MySQL/MariaDB archive. Settings live only in files | superseded in part by 0035; note |
| [0004](0004-schema-ownership.md) | The code example uses the control-plane layout's names. "TS" reads "JavaScript"; generated frontend types are `.d.ts` or JSDoc `@typedef` modules; protobuf-es target `js+dts`; the `schemars` rule (about 20 document types) | illustration; language |
| [0005](0005-command-layer.md) | The dispatch example returns `Vec<Diagnostic>` and calls `parse_arguments(command_id, arguments_json)`. `COMMANDS: &[CommandRow]` with the generated `CommandDescriptor`; `stability` on commands and events; typed JavaScript wrappers over `CommandExecutor`; `acting_plugin_id`; no domain logic in `apps/web` | illustration; contracts v1.1 |
| [0006](0006-ui-transport-http-only.md) | Decision 4: the shell only runs `rastai open --print-url`; file associations arrive as a `#/open?path=` fragment. "TS package" reads "JavaScript package"; cross-origin browser clients refused in v1; `EventService` binary only | superseded in part by 0034; language |
| [0007](0007-local-api-security.md) | Decisions 3 and 4: sign-in through `token.login_code.create`; sessions bound to the redeeming origin; the `Sec-Fetch-Site` rule; any loopback Host on any port; a too-open `owner.token` refuses the start with `RRS-C1008` | superseded in part by 0034 |
| [0012](0012-build-in-house-policy.md) | Decision 9 settled: the §27 additions are approved, each behind one wrapper file; `typescript` is a dev-only checker. Managed vLLM comes after the prototype | owner decisions 1 and 8; vLLM |
| [0013](0013-unified-model-runtime.md) | M2 closes on NVIDIA, AMD parity at M10; the self-hosted GPU runner; one golden fixture per op; placement planned only in Rust | owner decisions 11 and 12 |
| [0014](0014-platform-tiers.md) | The `nightly-gpu.yml` runner and its rules; the AMD card before M10 | owner decisions 11 and 12 |
| [0015](0015-plugins-and-licensing.md) | "TypeScript" reads "JavaScript"; `.jsx` plugin templates; the AppArmor profile and the namespace fallback (`RRS-P2005`, `RRS-P2001`); capabilities for every contribution key; locked security-UI tokens | language; contracts v1.1 |
| [0017](0017-definition-of-done-l0-l3.md) | "(L2)" tags on the durability extras; AMD checked at M10; the growing three-surfaces test | contracts v1.1 |
| [0019](0019-in-house-webgl2-canvas.md) | Decision 5: a renderer is a plain JavaScript class. Theme tokens: `canvasColors`, `setColors`, the glyph atlas font, `check-colors` | superseded in part by 0036; note |
| [0020](0020-evaluation-first-party-plugin.md) | Registry names are namespaced (`eval.drift`, `eval.pass_at_k`). The judge trust bar (0.8 on 10 labels); votes in `evals/votes.rrsdata`; exit 2 covers cancelled and refused | illustration; owner decisions 9 and 15 |
| [0021](0021-technique-coverage.md) | `placement{graph_node_id: gpu_index}`, planned only by `plan_layer_placement` in Rust | contracts v1.1 |
| [0023](0023-owner-code-style.md) | The TypeScript section and the TS wording in decisions 1 and 7 move to ADR-0036. ruff and prettier in `just check`; `check-unsafe`; `check-no-typescript` | superseded in part by 0036; note |
| [0025](0025-two-runtimes-per-model.md) | No managed vLLM in the prototype; an external vLLM is an `OpenAiAdapter` endpoint with `proxy` replays | vLLM (owner decision 8) |
| [0026](0026-live-panel-in-decode-step.md) | The +35% budget is the owner's decision; analysis math in `rastai_lens`; Psychology sliders call core commands directly, p95 under 50 ms | owner decision 14; contracts v1.1 |
| [0028](0028-teach-it-anchors-and-drift.md) | Anchors on by default for DPO too, behind a GPU drift test before L2; 1B stays the First Run default; CUDA IPC sharing behind a measurement gate | owner decision 16 |
| [0029](0029-strict-safetensors-contract.md) | The Rust reader is `crates/rastai-formats/src/safetensors/reader.rs`. Decision 1: Python calls `rastai_formats.check_safetensors_header` instead of its own header checks | path; superseded in part by 0037 |
| [0030](0030-diagnostics-catalog.md) | Decision 8 decided: the in-house JSON-lines logger. Collisions settled; ranges `RRS-S5xxx` and `RRS-D4xxx`; "TS" reads "JavaScript" | owner decision 2; language |
| [0031](0031-sandbox-manager.md) | `hyper` on §27, wrapper `engine_http.rs`; the `rastai.job_id` label; the egress proxy | contracts v1.1 |
| [0032](0032-document-files-and-validation.md) | The daemon-free validator is `rastai validate <file>`; `rastai file validate` is the daemon-backed verb. DEFLATE through `miniz_oxide`; fuzz targets for every in-house parser | path; 0037 |

## Writing a new ADR

- **Number:** the next free four-digit number. File name `NNNN-short-title.md`, lowercase with hyphens.
- **Sections, in this order:**
  - Status;
  - Context (the problem and the forces, with spike numbers where they exist);
  - Decision (specific and testable, ending with **Checked by:** the tests that prove it);
  - Consequences (good, and costs);
  - Alternatives considered (a table: alternative, why not);
  - Links (spec sections, contracts sections, spike results, plan tasks, related ADRs).
- **Length:** about one screen. Bullets and tables, plain English, no filler.
- **Names:** use [contracts.md](../architecture/contracts.md) names exactly. A new shared name goes into contracts.md first, in its own PR. ADRs have no "New names" section.
- **Status values:**
  - `Proposed`;
  - `Accepted, <date>`;
  - `Accepted, <date>. Decision N superseded in part by ADR-NNNN, <date>` (or `amended by`), when a later ADR replaces or extends one decision and the rest stands;
  - `Superseded by ADR-NNNN, <date>`, when the whole ADR is replaced.
- **An accepted ADR is never edited to change its decision.** Write a new one that supersedes it. In the same PR, update the old one's status line, add a short "Superseded in part" paragraph under it, mark the replaced text inline, and update this index.
- **Fixing an example, a path or a name** in an accepted ADR is allowed when the decision stays the same: mark it *Note since acceptance (date)* in the ADR and add a row to the table above.
- **Code samples:** Rust and Python in the owner's style (ADR-0023). Frontend samples are plain JavaScript or JSX with short JSDoc (ADR-0036), never TypeScript syntax.
- **Diagrams:** Mermaid `flowchart`, `sequenceDiagram` or `stateDiagram-v2` only. Quote labels that contain spaces or punctuation, never use `end` as a node id, no HTML except `<br/>`, no semicolons in labels, and declare sequence participants with simple ids. Follow each diagram with prose: the steps that aren't obvious, what is persisted where, and the failure branches.
