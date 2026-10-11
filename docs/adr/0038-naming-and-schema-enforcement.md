# ADR-0038: Names and command schemas are enforced by CI, not by convention; work is closed only after independent verification

**Status:** Accepted, 2026-10-11.

## Context

- Contracts.md §15 is the only registry of shared names (ADR-0004, ADR-0005). Until now nothing failed CI when a task used a name that isn't in it. The registry held by convention.
- `buf breaking` guards the protobuf messages. It can't see inside `arguments_json` and `result_json`, which are strings to protobuf. A removed or renamed field in a command's arguments passes every check.
- The plan put the schema-diff check (`check-schema-diff`) in Phase 2, with the stable commands API. That leaves the whole prototype exposed, and the prototype is where most names are born.
- AI agents write most of the code (ADR-0036). They invent plausible names when the registry isn't in front of them, and they drift from a schema they haven't read.
- Contracts v1.1 invented some names that the owner has now accepted (owner decision B3, 2026-10-11). Accepted names should be locked, not left to be re-invented.
- The owner asked that work be checked before it is marked complete (2026-10-11). Today a task is closed when its PR merges, and the author's own claim is the evidence.

## Decision

1. **`check-schema-diff` moves into M1.** CI generates the command and event schemas (`schemas/commands/`, `schemas/events/`) and diffs them against the base branch.
   - Any change to a generated schema needs a matching contracts.md update in the same PR.
   - A breaking change to a command or event marked `stable` (or `deprecated`, until it is retired) fails CI. Breaking means: a removed or renamed field, a changed type, a newly required field.
   - `preview` items may break, but they still need the contracts update.
2. **`check-names` compares the code with the registry, both ways.**
   - Codegen emits `schemas/names.json` from the code. It lists: command ids, event types, error codes, `rastai.db` and `vault.db` tables and columns, settings keys, document schema ids, crate, package and module names, and CLI verbs.
   - CI compares it with contracts.md §15 plus the §2.1–2.3, §5.2, §6, §7, §8, §10 and §11 tables.
   - A name in code but not in contracts fails.
   - A name in contracts but not yet in code passes only if contracts marks it with its milestone, for example "(M4)". Without the mark it fails as stale.
3. **Agent build protocol.**
   - Every build task's agent prompt includes contracts.md, the generated schemas and the owning layout section.
   - The agent uses only registered names. A new name goes into contracts in the same PR (the existing rule that a contract change lands in its own PR first still holds for wire contracts, ADR-0004).
   - Every PR gets a separate reviewer agent with a naming and schema lens, in addition to the style and tests lenses.
   - `just check` runs `check-names` and `check-schema-diff`.
4. **Verification before closing.** An issue or task is closed only after independent verification:
   - CI is green, including `just check` with `check-names` and `check-schema-diff`;
   - a reviewer agent that is not the author confirms each acceptance criterion against the code and the command output;
   - that evidence is posted as the closing comment.

   This goes into the plan's definition of done (§3.3) and into CONTRIBUTING.
5. **Contracts v1.1 names are accepted.** §15 is marked accepted, not provisional. After this ADR a change to an accepted name is a contracts change with a reason in the PR.

**Checked by:**
- `tools/checks/tests/` fixture suites for `check-names` and `check-schema-diff` (plan M1-57, M1-58). For `check-names`: a name added to code only fails; a name added to contracts only fails; a contracts name marked "(M4)" and absent from code passes; the same name unmarked fails;
- For `check-schema-diff`: a schema change without a contracts diff fails; a removed field, a changed type and a newly required field on a `stable` or `deprecated` command each fail; the same change on a `preview` command passes with the contracts update;
- `just check` runs both checks, and the `ci.yml` required job list names them;
- the PR template has the names and reviewer-agent lines (plan 8.5), and the issue's closing comment follows plan 3.6;
- CONTRIBUTING and plan §3.3/§3.6 carry the rule.

## Consequences

**Good**
- A name or a command schema can't drift without a visible, reviewed contracts change.
- Agents have the registry in front of them, so they stop inventing names.
- The Phase 2 stable API starts from schemas that have been diffed since M1, not from a first-ever baseline.
- "Done" has evidence a reader can check, not just the author's word.

**Costs**
- About 2 size-days in M1 for the two checks, the names generator and the task prompt template.
- Adding a name needs a contracts edit in the same PR. That is the point, and it is a small tax on every new command or table.
- Milestone marks in contracts.md have to be kept up to date. Removing the mark is part of the PR that lands the name.
- A reviewer agent per PR costs tokens and wall-clock time, and the closing comment is extra writing.
- The generated names registry is only as complete as its generators. A kind of name nobody generates is still unchecked until it gets a row in `names.json`.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Convention only (contracts §15 plus review) | What we had. Nothing fails when a task ignores it, and reviewers can't remember every name. |
| A full schema registry service | A running service for a one-binary product; a diff over generated files against the base branch gives the same guard with no server. |
| Keep `check-schema-diff` in Phase 2 | Every command is `preview` until then, so nothing is protected while most names and shapes are being set. The check is cheap to write now and costly to retrofit. |
| Check only code to contracts (one direction) | Contracts would fill with names that never ship. The milestone mark keeps planned names honest. |
| Author closes their own task | The author's claim is the weakest evidence. A second agent reading the code and output is cheap. |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions), [§29 repo and engineering setup](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup)
- Contracts: [§4.2 and §4.3 (`stability`, `buf breaking` and its blind spot)](../architecture/contracts.md#4-wire-contracts), [§12.5 fixtures and fuzzing](../architecture/contracts.md#125-fixtures-and-fuzzing), [§15 names folded in](../architecture/contracts.md#15-names-folded-in-from-the-layout-and-flow-docs), [§15.9 CI, recipes and tests](../architecture/contracts.md#159-ci-recipes-tests-and-environment-variables), [§12.6 contract checks and the agent build protocol](../architecture/contracts.md#126-contract-checks-and-the-agent-build-protocol)
- Plan: [§3.2 contracts first](../plan/implementation-plan.md#32-contracts-first), [§3.3 definition of done per PR](../plan/implementation-plan.md#33-definition-of-done-per-pr), [§3.5 agent build protocol](../plan/implementation-plan.md#35-agent-build-protocol), [§3.6 closing an issue: independent verification](../plan/implementation-plan.md#36-closing-an-issue-independent-verification), M1-57 (check-names), M1-58 (check-schema-diff)
- Related: [ADR-0004](0004-schema-ownership.md), [ADR-0005](0005-command-layer.md), [ADR-0017](0017-definition-of-done-l0-l3.md), [ADR-0032](0032-document-files-and-validation.md), [ADR-0036](0036-frontend-plain-javascript.md)
