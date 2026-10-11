# Contributing

## Sign your commits (DCO)

Every commit must carry a `Signed-off-by` line certifying the [Developer Certificate of Origin](DCO): that you wrote the change, or otherwise have the right to submit it under this project's license.

```
git commit -s -m "Add the strict safetensors reader"
```

`-s` adds `Signed-off-by: Your Name <you@example.com>` using your git name and email. Pull requests with unsigned commits can't be merged. To sign commits you already made: `git rebase --signoff main`.

## Turn on the pre-push hook

Run this once in every clone:

```
just install-hooks
```

It sets `core.hooksPath` to `tools/hooks` in the clone's own git config. Git then runs `tools/hooks/pre-push` before every push: the hook runs `just check` and refuses the push if it fails. It also refuses, before running the check, when the working tree has uncommitted changes or untracked files that `.gitignore` doesn't cover, or when a pushed branch or tag is not the checked-out commit, because the check tests the files on disk and those would not be what you push. Check out the branch you push; deleting a remote branch skips the check. GitHub CI runs only when started by hand while the repository is private, so this hook is what keeps a broken branch from leaving your machine.

In an emergency, for example to save work in progress before a machine goes away, skip it with `git push --no-verify`. Nothing merges on that alone: the verifier runs `just check` on a fresh clone of the pull request's head before the issue closes.

## How work lands

- **Contracts first.** Proto messages, document schemas, command rows and their argument types go in their own pull request first, before any code that uses them. Any other new name goes into `contracts.md` in the same pull request as the code that uses it.
- **Done means:** `just check` passes and the change adds a test. `just check` includes `check-names` and `check-schema-diff`. The pre-push hook (above) runs it before every push.
- **Names come only from the registry.** Every shared name (command id, event type, error code, table or column, settings key, document schema id, crate, package or module, CLI verb) is in `docs/architecture/contracts.md` §15 or its §2.1–2.3, §5.2, §6, §7, §8, §10 and §11 tables. A new name goes into contracts as the previous bullet says. A name in contracts that isn't built yet carries its milestone, for example "(M4)". `check-names` fails on a name in code but not in contracts, and on one in contracts but not in code without its milestone mark.
- **Schema changes need a contracts update.** `check-schema-diff` compares the generated command and event schemas with the base branch. Any change needs a matching contracts.md update in the same pull request, and a breaking change to a `stable` command or event fails `just check`. `preview` items may break, but still need the update.
- **A separate reviewer checks every pull request.** Every pull request gets a separate reviewer agent (never the author) with a naming and schema lens, plus the style and tests lenses.
- **Closing a task or issue needs independent verification.** A task is done only after independent verification with evidence: `just check` passes locally (the pre-push hook enforces it), and a reviewer agent that is not the author runs `just check` on a fresh clone of the exact PR head and confirms each acceptance criterion against the code and command output, posted as the closing comment. GitHub CI is manual-only while the repo is private (owner decision 2026-10-11); macOS and Windows are re-checked when CI returns. The author's own claim doesn't close anything. Pull requests link their issue with `Refs #n`, never `Closes #n`, so merging doesn't close it; the verifier closes it.
- **Agents work from the contracts.** A build agent's task prompt includes contracts.md, the generated schemas and the owning layout section, and the agent uses only registered names.
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

Rastai builds in-house by default. A new external dependency is allowed only in the five categories of spec §27 (foundations, security-critical code, byte-exact compatibility surfaces, external engines, dev tooling). It must be added to the dependency register in the same pull request, with its reason and replacement path, and only one wrapper file in the owning crate or package may import it. `just check` enforces the list (`check-deps`).

## Hard rules

- No pickle, anywhere: weights are safetensors, checkpoints are safetensors plus JSON.
- No package installs at runtime.
- No type defined twice: protobuf owns the wire, `rastai-formats` owns documents, everything else is generated.
