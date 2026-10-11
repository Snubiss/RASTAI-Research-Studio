# Contributing

## Sign your commits (DCO)

Every commit must carry a `Signed-off-by` line certifying the [Developer Certificate of Origin](DCO): that you wrote the change, or otherwise have the right to submit it under this project's license.

```
git commit -s -m "Add the strict safetensors reader"
```

`-s` adds `Signed-off-by: Your Name <you@example.com>` using your git name and email. Pull requests with unsigned commits can't be merged. To sign commits you already made: `git rebase --signoff main`.

## How work lands

- **Contracts first.** Changes to protobuf files, document schemas or the command catalog go in their own pull request, before any code that uses them.
- **Done means:** `just check` passes and the change adds a test.
- **One topic per pull request.** Small pull requests get reviewed; large ones wait.
- **Every user-facing error** gets a code in the diagnostics catalog.

## Code style

Write code a person can read top to bottom without a guide.
- Basic, simple functions with plain data types. Polymorphism (a trait, an interface, a base class) is fine.
- No clever or obscure code: no deep generic hierarchies, macro or metaprogramming magic, builder chains, dependency-injection frameworks or layers of indirection.
- Constants at the top of each file.
- Readable names: `embedding_size`, not `n_embd`.
- Comments explain why, not what the syntax does.
- Small functions; `main` is mostly a sequence of calls.
- Frontend, plugin-UI and SDK code is plain JavaScript with React and JSX: `.js` and `.jsx` files only, no TypeScript syntax. Types go in short JSDoc comments (`@typedef`, `@param`, `@returns`), kept simple: no conditional or mapped types, no long generic templates. `just check` type-checks them with `tsc --noEmit` in `checkJs` mode; that's a dev-only check, nothing ships it. Generated declarations (`.d.ts`) live in generated folders and are never edited by hand.

## Dependencies

Rastai builds in-house by default. A new external dependency is allowed only in the five categories of spec §27 (foundations, security-critical code, byte-exact compatibility surfaces, external engines, dev tooling). It must be added to the dependency register in the same pull request, with its reason and replacement path, and only one wrapper file in the owning crate or package may import it. CI enforces the list.

## Hard rules

- No pickle, anywhere: weights are safetensors, checkpoints are safetensors plus JSON.
- No package installs at runtime.
- No type defined twice: protobuf owns the wire, `rastai-formats` owns documents, everything else is generated.
