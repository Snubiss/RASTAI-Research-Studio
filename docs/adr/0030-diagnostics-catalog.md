# ADR-0030: One diagnostics catalog; typed errors inside, coded diagnostics at the edge

**Status:** Accepted, 2026-10-10

## Context

- "Every user-facing error has a catalog code" is a hard rule (§29). One catalog serves the UI, the docs and plugin authors (§7, §23).
- Basic users need plain English and a fix button. Advanced users need the full diagnostic with its path.
- Errors cross three languages and several processes: a Python worker's OOM must reach the UI as the same coded message the CLI shows.
- Error libraries (`anyhow`, `thiserror`, `miette`) are off the allowlist, and they hide where errors come from. A bare traceback is not a diagnostic.

## Decision

1. **Codes:** `RRS-<letter><4 digits>`, one letter per area, with ranges per topic (contracts §10). Codes are never reused; a retired code stays in the catalog marked `retired`. Third-party plugins use `PLG-<plugin_id>-<4 digits>` from their own `catalog/errors.json`.
2. **Source of truth:** `catalog/errors/RRS-<letter>.json` (first-party plugins in `plugins/<name>/catalog/`). Each entry has `code`, `severity`, `title`, `message` (with `{placeholders}`), `basic_text`, `fix_text`, an optional `fix_command_id`, `docs_anchor` and `retired`.
3. **Codegen** writes Rust constants into `rastai-diag`, `@rastai/catalog` for the UI, `rastai_plugin.catalog` for Python, and the generated reference in `docs/reference/`.
4. **Inside a component, errors are typed.** Each crate has one hand-written error enum with `fn code(&self) -> &'static str`. Each Python package has one exception base class with a `code` attribute.
5. **At the edge, errors become a `Diagnostic`:** command dispatch, HTTP handlers, the CLI's `main`, and the worker's `main`. A `Diagnostic` carries `code`, `severity`, `message`, `path`, `fix_text`, `fix_command_id` and `fix_arguments_json`. Catch-all handling exists only at these edges.
6. **Workers report failure** as a `Diagnostic` in their last `job.state_changed`, never as a bare traceback. The traceback goes to `stderr.log`.
7. **Rendering:** the in-house renderer in `rastai-diag` (CLI) and the diagnostics view in `@rastai/ui`.
   - Basic shows `basic_text` and a fix button when `fix_command_id` is set.
   - Advanced shows the code, path, message and fix.
   - Validation collects all diagnostics in one pass (ADR-0032).
8. **Logging (default until the owner decides, contracts §14 item 2):** an in-house JSON-lines logger in `rastai-diag` (`log_info(message, fields)`) writes to `<state>/logs/`. Run ids reach Python through `RASTAI_RUN_ID`. §25 names Rust `tracing`, which isn't on the allowlist; revisit if structured spans are needed. *(Note since acceptance, 2026-10-10: decided. Spec v5.4 §25 now names this in-house logger (`init_logger`, `log_info`), and `tracing` is not used (owner decision 2). The contracts §14 item is closed.)*

```rust
// crates/rastai-store/src/error.rs
pub enum StoreError {
    CorruptEventLog { run_id: String, sequence: u64 },
    // one variant per failure the store can report
}

impl StoreError {
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::CorruptEventLog { .. } => catalog::RRS_S2002,
        }
    }
}
```

*Note since acceptance (2026-10-10): contracts v1.1 settles the numbering collisions between the drafts (each number keeps one meaning; withdrawn draft numbers are listed in `catalog/README.md`), adds the ranges `RRS-S5xxx` (long-term archive, ADR-0035) and `RRS-D4xxx` (run-log import), and lists every assigned code in §10.3. The first check below says "JavaScript" where it said "TS" (ADR-0036). The decisions are unchanged.*

**Checked by:**
- a CI check that every code used in Rust, Python or JavaScript exists in the catalog and is not retired;
- the L0 test that every command fails with a coded diagnostic (ADR-0017);
- a worker test that an exception in a job ends in a coded `job.state_changed`;
- a catalog lint that every entry has `basic_text` and `fix_text`.

## Consequences

**Good**
- The same failure reads the same way in the UI, the CLI, MCP and the docs.
- Fix buttons are commands, so they work from any surface.
- Plugin authors get the same renderer and catalog format.

**Costs**
- Every new failure needs a catalog entry, which slows quick changes. That is deliberate.
- Hand-written error enums and `code()` matches are verbose (ADR-0023 accepts this).
- An in-house logger lacks spans and sampling until they're needed.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `anyhow` / `thiserror` / `miette` | Off the allowlist. String errors lose the code; derive macros hide the mapping. |
| Codes only in the UI | The CLI, MCP and plugins would show different messages for the same failure. |
| Tracebacks to the user | Not actionable, and they leak paths. They belong in `stderr.log`. |
| `tracing` for logs | Not on the allowlist yet. The JSON-lines logger covers the prototype. |

## Links

- Spec: [§7](../spec/rastai_spec_v5.md#7-ux-system), [§23](../spec/rastai_spec_v5.md#23-validation), [§25](../spec/rastai_spec_v5.md#25-observability-updates-and-maintenance-new), [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup)
- Contracts: [§10 error catalog](../architecture/contracts.md#10-error-catalog), [§4.1 Diagnostic](../architecture/contracts.md#4-wire-contracts), [§10.2 collisions settled in v1.1](../architecture/contracts.md#102-collisions-settled-in-v11)
- Related: [ADR-0005](0005-command-layer.md), [ADR-0017](0017-definition-of-done-l0-l3.md), [ADR-0023](0023-owner-code-style.md), [ADR-0032](0032-document-files-and-validation.md)
