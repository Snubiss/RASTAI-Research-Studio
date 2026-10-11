# The catalog

Every error code Rastai shows, and the plain-English terms the UI explains, in one place. The Rust tables in
`rastai-diag`, `@rastai/catalog`, `rastai_plugin.catalog` and `docs/reference/errors.md` are generated from these
files (`just codegen`); never edit the generated copies.

```
catalog/
  errors/RRS-<letter>.json   one file per prefix of contracts §10.1, a JSON array of entries
  terms/terms.json           the plain-English terms (the glossary, contracts §13)
  README.md                  this file
```

`RRS-L` (Psychology) and `RRS-V` (Evaluation) live in the plugins' own catalogs,
`plugins/psychology/catalog/errors.json` and `plugins/evaluation/catalog/errors.json`, written in M4 and M5.

## How to add a code

1. List it in contracts §10.3 first, with its severity and a short meaning, inside the range its owner holds
   (contracts §10.1). Take the next free number in that range.
2. Add its entry to the owner's file here, with every field: `code`, `severity` (`error`, `warning` or `info`, the
   same as in §10.3), `title`, `message` (with `{snake_case}` placeholders for the values the call site passes),
   `basic_text` (plain English for Basic mode), `fix_text`, `fix_command_id` (optional; the command a fix button
   runs), `docs_anchor` (the code in lowercase, `rrs-s3001`) and `retired` (`false`).
3. Run `just codegen`, then `just check`.

Rules the catalog checks enforce (`tools/codegen/tests/catalog_checks.rs`):

- A code means one thing forever. It is never deleted, renumbered or reused: a code that is no longer raised stays
  here with `"retired": true`.
- The catalog and contracts §10.3 hold the same codes with the same severities.
- Every `RRS-` code that first-party source names outside its tests is in the catalog.
- A `fix_command_id` names a command the fix button can run: a row of rastai-core's command table, or a command
  contracts §6.4 registers with the milestone that builds it (`storage.gc` (M5)).
- A `diagnostic("RRS-...", &[("name", ...)])` call passes exactly the placeholders its entry's message and fix use. A
  placeholder without a value would show as `{name}`. The scan also reads `&values` when the closest
  `let values = [...]` above the call is the array, so give each code its own array.

## Terms

`terms/terms.json` holds the glossary rows (contracts §13) a user may meet in the app, each with a plain
`basic_text` and a fuller `advanced_text`. A row that names two things (`Suite / test`, `Rollout / group`,
`Live peek / step mode`, `Basic / Advanced`, `MoE / MLA / ALiBi / AttnRes`) is one term, explained under its first
name. These rows are left out, because the UI never shows them by name:

| Glossary row | Why it isn't a term |
|---|---|
| JSDoc, CommandExecutor, Names registry, Milestone marker, Outbox, Source id, Bus sequence | words for people who write Rastai's code |
| Chain driver, Static decode step, Durable / ephemeral event, Stability | inner parts of the daemon, the runtime and the wire; the user sees runs, speed and commands, not these |
| Theme token, Locked token | names for theme authors, who read the theme docs (contracts §5.6) |
| Origin-bound session | how sign-in is kept safe; the UI only says "signed in" or "signed out" |
| Section / panel | the app's own layout words, which the UI shows rather than explains |

A task that puts one of these words in front of users adds its term here.

## "Not available yet" codes

A command whose handler isn't built yet is an honest L0 stub (ADR-0017): it fails with its area's "not available yet"
code and never pretends to succeed. The area is the first word of the command id.

| Command area | Code |
|---|---|
| `system` (except the three below), `settings`, `token`, `command`, `job`, `project`, `file`, `document`, `workspace`, `artifact`, `tutorial` | `RRS-C0010` |
| `system.doctor`, `system.hardware`, `system.env.*` | `RRS-H0003` |
| `plugin` | `RRS-P1004` |
| `storage`, `archive`, `secret` | `RRS-S3006` |
| `node`, `transfer`, `hf` | `RRS-N0005` |
| `chat`, `model`, `conversation`, `gateway`, `adapter` | `RRS-G0011` |
| `design` | `RRS-M0004` |
| `train`, `checkpoint`, `recipe`, `rlhf`, `sweep`, `teach`, `run` | `RRS-T0006` |
| `dataset`, `converter` | `RRS-D0002` |
| `gym`, `harness`, `sandbox` | `RRS-R0007` |
| `psychology` (plugin catalog, M4) | `RRS-L0007` |
| `eval` (plugin catalog, M5) | `RRS-V0004` |

## Withdrawn draft numbers

Earlier design drafts used some numbers that never reached a catalog (contracts §10.2). They are listed here for
readers of those drafts; none of them is in the catalog, and the left column never means the old thing.

| Draft number and meaning | Now |
|---|---|
| `RRS-G0006` data policy (inference draft v1) | `RRS-C1009`; `RRS-G0006` now means "the model's node is unreachable" |
| `RRS-G0008` data policy | `RRS-C1009` |
| `RRS-D3004` data policy | `RRS-C1009` |
| `RRS-N3009` data policy (designer draft) | `RRS-C1009`; `RRS-N3009` now means "download retries used up" |
| `RRS-V1003` data policy | `RRS-C1009` |
| `RRS-G3011` vLLM unavailable | withdrawn: the managed vLLM engine left the prototype |
| `RRS-V0002` | `RRS-V0101` |
| `RRS-J3001` | `RRS-J3002` |
| `RRS-J0003` | `RRS-J1003` |
| `RRS-G0003` (inference flow draft) | `RRS-G0005` |
| `RRS-G0007` stall, `RRS-G0008` busy (inference flow draft) | `RRS-G0009` |
| `RRS-G0009` no twin (inference flow draft) | `RRS-G3009` |
| `RRS-G1001` template, `RRS-G1002` prompt length (inference flow draft) | `RRS-G1002`, `RRS-G1005` |
| `RRS-G1003` twin mismatch (inference flow draft) | `RRS-G3010` |
| `RRS-G2005` (inference flow draft) | `RRS-G2002` |
| `RRS-G2006` (inference flow draft) | `RRS-C1005` |
| `RRS-G3001` (inference flow draft) | `RRS-G3005` |
| `RRS-G3002`–`G3004` startup (inference flow draft) | `RRS-G3002` |
| `RRS-G3005` died while serving (inference flow draft) | `RRS-G3003` |
| `RRS-G3006` no GGUF (inference flow draft) | `RRS-G3008` |
| `RRS-G4001` incognito (inference flow draft) | `RRS-L0006` |
| `RRS-H3002` (inference flow draft) | `RRS-H3006` |
| `RRS-H3003` (inference flow draft) | `RRS-H3007` |
| `RRS-S3001` disk full while writing (inference flow draft) | `RRS-S3002` |
| `RRS-L0004`–`L0006` (inference flow draft) | `RRS-L0003`–`L0005` |
| `RRS-L1001`, `RRS-L3001` needs Inspectable (inference flow draft) | `RRS-G3004` |
| `RRS-L1002`, `RRS-L1003` (inference flow draft) | `RRS-L1001`, `RRS-L1002` |
| `RRS-L3002`, `L3003`, `L3005`, `L3006` (inference flow draft) | `RRS-L3001`, `L3002`, `L3003`, `L3004` |
| `RRS-L3004` preset for other weights (inference flow draft) | `RRS-G0010` |

Numbers that kept one meaning when an earlier draft gave them two (contracts §10.2):

| Number | Keeps | The other meaning moved to |
|---|---|---|
| `RRS-P2001` | third-party plugin module refused: no user namespaces | iframe called a command outside its granted scopes → `RRS-P2004` |
| `RRS-P4001`–`P4003` | plugin module crashed, didn't answer, crashed repeatedly | bridge: malformed message → `RRS-P4101`, too many calls in flight → `P4102`, message too large → `P4103`; iframe didn't initialize (was `P4004`) → `P4104`; plugin UI threw (was `P4005`) → `P4105` |
| `RRS-N3010` | waiting for another program's download | transfer cancelled → `RRS-N3012` |
| `RRS-N2002` | the connection to a node dropped; its streams ended | node reconnected → `RRS-N2003` |
| `RRS-N3007`, `RRS-N3009` | Hugging Face token missing or rejected; download retries used up | transfer cancelled → `RRS-N3012`; data policy → `RRS-C1009` |
| `RRS-C1002` | not signed in, or unknown credential | missing scope → `RRS-C1005` |
| `RRS-H3005` | process uses more GPU memory than its lease | host out-of-memory killer → `RRS-J1002` |
| `RRS-H3007` | out of GPU memory inside any job step | none |
| `RRS-H3004` | info, not error: model being evicted | none |
