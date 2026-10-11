# ADR-0017: Definition of done: four levels, L0 to L3

**Status:** Accepted, 2026-10-10

## Context

- The prototype is the whole suite, end to end, at prototype quality (§0.1). Without a shared bar, "done" means something different for every feature and every agent.
- One human reviews everything (§28 risk 1). Partial work must be visible as partial, not hidden behind a demo path.
- There's no target date; progress is measured by exit criteria (§0.8, §32).
- "Durability first" (§2): a feature that loses work on a crash isn't done at any level above the first.

## Decision

1. **Every feature is rated on four levels.** Each level includes the ones below it.

   | Level | Meaning | Evidence required |
   |---|---|---|
   | **L0** | The command exists and fails cleanly with a catalog error code. | A row in the command registry with typed arguments and result; it appears in the CLI, MCP and the palette; a test that executing it returns a `Diagnostic` with a catalog code, never a crash or an un-coded error. |
   | **L1** | The main path works end to end; failures are recoverable and nothing is lost. | An end-to-end test of the main path (CLI or MCP, plus Playwright where there's UI); a kill -9 or interruption test for anything that writes state or runs a job, showing it resumes or fails cleanly with nothing lost. |
   | **L2** | Common variants work and have tests. | The variants are listed in the plan's backlog entry for the feature, and each has a test. |
   | **L3** | Polished: Basic-mode UX, docs, performance budgets met. | A Basic-mode surface; a reference page; the feature's performance budget measured in CI or the nightly GPU run. |

2. **The prototype is done when:**
   - every section is at L1;
   - the core loop is at L2: chat → see inside → steer or branch → train → verify;
   - it all runs on Linux NVIDIA, Linux AMD and a paired SSH box.
3. **L3 is Phase 2 work** (Basic-mode polish, the novice First Run, tutorials, the accessibility pass). Advanced depth gets polished first in the prototype.
4. **Levels are tracked per feature** in the implementation plan (`docs/plan/`). A claimed level names the tests that prove it. A level is lost when one of its tests is removed or fails on `main`.
5. **Per pull request:** "done" means `just check` passes and the change adds a test that fails without it (§29, PR template).
6. **Stubs are honest.** An L0 command that isn't built yet returns a coded "not available yet" diagnostic from its area's range. It never pretends to succeed.

*Note since acceptance (2026-10-10):*
- *Decision 2's "Linux AMD" is checked at M10, which holds the AMD parity gate; M2 closes on NVIDIA (owner decision 11).*
- *Names marked "(L2)" in contracts.md are designed now but built only after the core loop reaches L2: daily `VACUUM INTO` snapshots with salvage, the node self-update watchdog with rollback, plugin-data downgrade guards, crash-report rate limits, and plugin event deliveries beyond what Evaluation auto-run needs. The minimal durable path comes first: event logs, reattach, atomic writes. That is "durability first" applied, not a change to L1.*
- *M1-49's three-surfaces test (`tests/e2e/three_surfaces/`) grows each milestone, and M10-E02 adds an API-only run of the core loop (contracts §2.4 rule 9).*
- *The decisions are unchanged.*

**Checked by:** a CI test that runs every registered command with `dry_run` and fails on an un-coded error (this is the L0 floor for the whole app); the plan's level table, reviewed at each milestone exit.

## Consequences

**Good**
- Every agent and reviewer uses the same words for progress. "L1 except remote" is a precise statement.
- Breadth-first work is safe: the whole surface exists at L0 early, so contracts are exercised from the start.
- Durability is part of L1, not a later hardening pass.

**Costs**
- L0 stubs are work that shows no feature, and the registry fills with unbuilt commands early.
- Keeping the level table honest takes review time at each milestone.
- L1's crash test requirement makes simple features slower to call done.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Binary done / not done | Hides partial work. A demo path looks done. |
| Build each section to L3 before starting the next | Contradicts §0.1 (the prototype is the whole suite), and polish before the loop works is wasted. |
| Story points or percentages | Not testable. Levels are tied to named tests. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) items 1, 8 and 15, [§5](../spec/rastai_spec_v5.md#5-definition-of-done), [§28](../spec/rastai_spec_v5.md#28-risk-register) risk 1, [§31](../spec/rastai_spec_v5.md#31-build-order)
- Contracts: [§6 commands](../architecture/contracts.md#6-commands), [§10 error catalog](../architecture/contracts.md#10-error-catalog)
- Related: [ADR-0005](0005-command-layer.md), [ADR-0030](0030-diagnostics-catalog.md)
