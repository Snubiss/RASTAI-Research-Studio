# Rastai Research Studio

Open-source studio to design, train, serve and study neural networks. Rust backend, Python 3.12 + PyTorch compute, React + TypeScript UI in a Tauri 2 shell.

## Where things are

- `docs/spec/rastai_spec_v5.md`: the scope (v5.3). Part II is the architecture, §27 the dependency allowlist, §29 the repo layout.
- `docs/architecture/contracts.md`: the shared names (components, processes, protobuf services, commands, events, tables, error codes, states). Use these names exactly.
- `docs/architecture/layout/`: code layout per crate and package. `docs/architecture/flows/`: logic flow diagrams.
- `docs/adr/`: decisions already made; don't re-litigate them without a new ADR.
- `docs/plan/`: the implementation plan and backlog.
- `spike/`: throwaway experiments and their measured results; prototype code to port, not to import.

## Hard rules

- No pickle. Weights are safetensors; checkpoints are safetensors plus JSON.
- No package installs at runtime.
- No type defined twice: protobuf for the wire, `rastai-formats` for documents, everything else generated.
- In-house by default. A new dependency needs an entry in spec §27 (and the dependency register) with a reason and a replacement path.
- Every user-facing error gets a catalog code.
- Contract changes (proto, schemas, commands) land in their own PR before code that uses them.
- Done means `just check` passes and a test was added.

## Code style (the owner's)

- All code is basic, simple functions with plain data types. Polymorphism (traits, interfaces, base classes) is fine.
- No obscure "Google-like" code: no deep generic hierarchies, macro or metaprogramming magic, builder chains, dependency-injection frameworks, type-level tricks or layers of indirection.
- Constants at the top of each file.
- Readable names (`embedding_size`, not `n_embd`).
- Comments explain why, not restate syntax.
- Small functions; a thin `main` that is mostly a sequence of calls.
- No clever one-liners.

## Commits

Sign off every commit (`git commit -s`, DCO).
