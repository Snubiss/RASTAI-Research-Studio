# ADR-0023: Code style: simple functions and plain data; polymorphism yes, obscure abstraction no

**Status:** Accepted, 2026-10-10

## Context

- The owner reviews everything, and AI agents write most of the code. Code must read top to bottom without a guide.
- The owner's rule: all code is basic, simple functions with plain data types, with no obscure "Google-like" code. Polymorphism is fine.
- Rust, Python and TypeScript each have idioms that drift toward cleverness: proc-macro DSLs, deep generic trait stacks, builder chains, metaclass models, dependency-injection containers, global state stores.
- The spike was written this way, and it reads well. For example, the Gemma 3 runtime is about 500 lines including comments, and `check_exact_tiling` does exactly what its name says.

## Decision

**All languages**
1. **Plain data in, plain data out:** Rust structs and enums, Python `@dataclass` (frozen where possible), TS object types.
2. **Small functions, named verb-first** (`load_config`, `check_exact_tiling`). A function that doesn't fit on one screen gets split.
3. **Constants at the top of each file** in `SCREAMING_SNAKE_CASE`, with a comment when the value came from a measurement (`# 8.3 KB per token, measured in the spike`).
4. **Readable names:** `embedding_size`, `layer_count`, `head_size`, `vocabulary_size`, never `n_embd` or `d_k`. Units go in names: `_bytes`, `_ms`, `_seconds`, `_count`, `_unix_ms`.
5. **Comments explain why**, not what the syntax does.
6. **A thin `main`**: a sequence of calls with no logic.
7. **Polymorphism is allowed where there are several real implementations:** a Rust trait or enum, a Python base class or Protocol, a TS interface. One implementation means no interface.
8. **Forbidden in our code:**
   - builder chains;
   - dependency-injection frameworks or service locators;
   - deep generic hierarchies and type-level tricks;
   - macro or metaprogramming magic;
   - layers of indirection that only forward calls;
   - clever one-liners.
   Dependencies are passed as function arguments or struct fields, and the composition root is the startup code.

**Rust**
- Closed sets are an `enum` with a `match` (adapters, transports, job kinds). Traits are for small synchronous interfaces with a few implementations (`LogSink`). There is no `async fn` in traits used behind `dyn`; use an enum.
- No proc macros of our own. Standard derives (`serde`, `Debug`, `Clone`) are fine. The command registry is a table plus a `match` (ADR-0005).
- Our functions take concrete types. A generic parameter is allowed only on a small helper whose meaning is obvious (for example `parse_arguments<T>`).
- One hand-written error enum per crate with `fn code(&self) -> &'static str`. No `anyhow`, no `thiserror`.
- axum handlers are plain `async fn`s. Middleware is a plain `async fn` passed to `from_fn`, with no custom tower layers.
- `unsafe` is forbidden unless an ADR allows it for a named file.

**Python**
- No metaclasses, no pydantic, and no decorators that change behavior, except `@dataclass`, `@torch.inference_mode()` and `@torch.no_grad()`.
- No `*args` or `**kwargs` pass-through in public functions. Spell the parameters out.
- Extension points are base classes with a few methods in `rastai_plugin`. Implementations are found by name in its registry.
- Type hints on every public function.

**TypeScript**
- Function components and plain functions. Classes only where a browser API demands one (a renderer that owns a WebGL context).
- Interfaces for polymorphism. No global state library: React state and context, plus `@rastai/client` for server state.

```rust
// Good: plain data, one match, readable names.
pub fn lease_fits(gpu: &GpuMemory, requested_bytes: u64, reserve_bytes: u64) -> bool {
    let free_bytes = gpu.total_bytes.saturating_sub(gpu.leased_bytes + reserve_bytes);
    requested_bytes <= free_bytes
}

// Not allowed: ResourceRequest::builder().gpu(0).vram_bytes(n).build()?
// Write the struct literal: ResourceRequest { gpu_indices: vec![0], vram_bytes: n, ..ResourceRequest::default() }
```

**Enforced by:** the root `CLAUDE.md` and a `CLAUDE.md` per crate and package; the PR template's style checkbox; review; `rustfmt` and `clippy` in `just check`; CI greps for `anyhow`, `thiserror`, `pydantic`, `metaclass=` and `proc-macro = true` in our own manifests.

## Consequences

**Good**
- Any file can be read and changed by the owner or an agent without learning a framework first.
- Diffs are local. Fewer abstractions means fewer places a change has to ripple through.
- Consistent code across three languages and many agents.

**Costs**
- More lines: explicit `match` arms, hand-written description tables (ADR-0004), spelled-out parameters. Some repetition is accepted in exchange for clarity.
- Some library idioms (custom tower layers, pydantic models) are off-limits even where they'd save code.
- Reviewers must hold the line, since most of these rules can't be fully linted.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Each language's idiomatic heavy patterns | Obscure to readers outside that ecosystem, and hard to review at agent volume. |
| Lint-only enforcement | Most of the rules (small functions, no needless indirection) need judgment, not lints. |
| No style rule | Agents converge on whatever their training data favors, which is often the opposite of this. |

## Links

- Spec: [§29](../spec/rastai_spec_v5.md#29-repo-and-engineering-setup) (making AI coding agents productive)
- Contracts: [§12 naming and code conventions](../architecture/contracts.md#12-naming-and-code-conventions)
- Repo: [CLAUDE.md](../../CLAUDE.md), [CONTRIBUTING.md](../../CONTRIBUTING.md)
- Related: [ADR-0004](0004-schema-ownership.md), [ADR-0005](0005-command-layer.md), [ADR-0030](0030-diagnostics-catalog.md)
