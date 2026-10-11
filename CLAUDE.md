# Rastai Research Studio

Open-source studio to design, train, serve and study neural networks. Rust backend, Python 3.12 + PyTorch compute, plain JavaScript + React UI (JSX, short JSDoc types checked by `checkJs`, no TypeScript syntax anywhere hand-written) in a Tauri 2 shell.

## Where things are

- `docs/architecture/README.md`: the map of the design docs and the reading order. Read it first.
- `docs/spec/rastai_spec_v5.md`: the scope (v5.4). Part II is the architecture, §27 the dependency allowlist, §29 the repo layout.
- `docs/architecture/contracts.md`: the shared names (components, processes, protobuf services, commands, events, tables, error codes, states). Use these names exactly. It is the only list of shared names; a wire, document or command shape goes there first in its own contract PR; any other new name goes there in the same PR as the code that uses it (ADR-0038).
- `docs/architecture/layout/`: code layout per crate and package. `docs/architecture/flows/`: logic flow diagrams.
- `docs/adr/`: decisions already made; don't re-litigate them without a new ADR.
- `docs/plan/`: the implementation plan and backlog.
- `spike/`: throwaway experiments and their measured results; prototype code to port, not to import.

## Hard rules

- No pickle. Weights are safetensors; checkpoints are safetensors plus JSON.
- No package installs at runtime.
- No type defined twice: protobuf for the wire, `rastai-formats` for documents, everything else generated.
- In-house by default. A new dependency needs an entry in spec §27 (and the dependency register) with a reason and a replacement path, and only one wrapper file imports it (foundations such as tokio, serde, React and PyTorch are used directly; contracts §2.5).
- No `unsafe` except the one `statvfs` call in `crates/rastai-store/src/disk_space.rs` (ADR-0033).
- `apps/web` holds no domain logic: anything stateful or computed is a command.
- No color literals outside `@rastai/ui` tokens (`check-colors`).
- A new in-house parser of untrusted input lands with its fuzz target (ADR-0037).
- Every user-facing error gets a catalog code.
- Contract changes (proto, schemas, commands) land in their own PR before code that uses them.
- Names come only from `docs/architecture/contracts.md` §15 (and its §2.1–2.3, §5.2, §6, §7, §8, §10 and §11 tables). A new name that isn't a wire, document or command shape is added to contracts in the same PR; shapes still land in their own contract PR first (ADR-0038). `just check` runs `check-names` and `check-schema-diff`, and a breaking change to a `stable` command or event fails it (ADR-0038).
- Every build task starts from contracts.md, the generated schemas (`schemas/`) and the owning layout section. Every PR gets a separate reviewer agent with a naming and schema lens.
- Done means `just check` passes and a test was added. A task is done only after independent verification with evidence: `just check` passes locally (the pre-push hook enforces it), and a reviewer agent that is not the author runs `just check` on a fresh clone of the exact PR head and confirms each acceptance criterion against the code and command output, posted as the closing comment. GitHub CI is manual-only while the repo is private (owner decision 2026-10-11); macOS and Windows are re-checked when CI returns. Run `just install-hooks` once per clone to turn the pre-push hook on.

## Code style (the owner's)

- All code is basic, simple functions with plain data types. Polymorphism (traits, interfaces, base classes) is fine.
- No obscure "Google-like" code: no deep generic hierarchies, macro or metaprogramming magic, builder chains, dependency-injection frameworks, type-level tricks or layers of indirection.
- Constants at the top of each file.
- Readable names (`embedding_size`, not `n_embd`).
- Comments explain why, not restate syntax.
- Small functions; a thin `main` that is mostly a sequence of calls.
- No clever one-liners.
- Frontend, plugin-UI and SDK code is plain JavaScript ES modules with React and JSX: `.js` and `.jsx` only, never `.ts` or `.tsx`, and no TypeScript syntax anywhere hand-written. Types go in short JSDoc comments (`@typedef`, `@param`, `@returns`) with no type gymnastics (no conditional or mapped types, no long generic templates). `just check` runs `tsc --noEmit` in `checkJs` mode as a dev-only checker. Generated `.d.ts` files are never hand-edited.

## Commits

Sign off every commit (`git commit -s`, DCO).
