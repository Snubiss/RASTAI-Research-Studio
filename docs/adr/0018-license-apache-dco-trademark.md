# ADR-0018: Apache-2.0, contributions under a DCO, and a trademark policy for the name

**Status:** Accepted, 2026-10-10

## Context

- Rastai is open source from day one (§2). The business model is free for everyone, companies included. Businesses pay for org features, hosted services and support, never for the app (§36).
- Many companies ban AGPL. Unsloth Studio's UI and Transformer Lab use it, so a permissive license is an advantage.
- The free-forever promise must be credible: community code mustn't be relicensable later.
- A license won't stop a clean-room clone. Depth and ecosystem will (§35). The name, however, needs protecting so forks can't pass themselves off as Rastai.
- Open WebUI's branding clause got it marked non-free by Linux distributions (§36 guardrails).

## Decision

1. **License:** Apache-2.0 for the app, `rastai-formats`, the SDKs (`rastai`, `rastai_plugin`, `rastai-plugin`, `@rastai/*`) and every first-party plugin. The `LICENSE` and `NOTICE` files sit at the repo root, and every published package declares `Apache-2.0`.
2. **Contributions use the Developer Certificate of Origin 1.1, not a CLA.**
   - `DCO` sits at the root; `CONTRIBUTING.md` explains `git commit -s`.
   - CI rejects pull requests with any commit lacking a `Signed-off-by` line.
   - Contributors keep their copyright, so nobody can relicense the community's code. That is part of the free-forever promise.
3. **Trademark policy:**
   - It covers the names "Rastai" and "Rastai Research Studio" and the logo.
   - Forks may say "based on Rastai" but must ship under another name.
   - It places no conditions on the code: no branding or attribution clauses in the license. Forks can remove the name freely.
   - The policy is published before the public launch (Phase 2). Registering the mark is an open question (§34).
4. **The format spec** (the `.rrs*` documents and their JSON Schemas) lives in its own repo under CC-BY, with a public RFC process.
5. **Plugins may use any license** (ADR-0015).
6. **Business features** (Teams, Enterprise) live in a separate `rastai-business` repo under a commercial license, written by the Rastai company itself. They plug in only through the public plugin API, and no code moves from the core into that repo.
7. **No license rug-pull.** The core license and the free-forever promise are published, and the DCO makes relicensing impossible.

**Checked by:** the CI sign-off check on every PR; a release check that each published crate, wheel and npm package declares `Apache-2.0` and includes `LICENSE`; the dependency license allowlist (ADR-0015).

## Consequences

**Good**
- Companies can adopt Rastai without legal review friction. The patent grant protects users and contributors.
- Contributors don't sign anything; they sign off commits.
- The commercial add-ons prove the plugin API is good enough, and they never weaken the core.

**Costs**
- Anyone, including competitors, can use and ship the code, including commercially.
- Without a CLA, the project can never relicense the core. That's the point, but it's irreversible.
- A trademark policy needs a registered mark and enforcement to mean much. That costs money and a legal entity (§36 prerequisites).

## Alternatives considered

| Alternative | Why not |
|---|---|
| AGPL | Banned at many companies, which blocks adoption. |
| BSL, SSPL or Elastic-style source-available licenses | Not open source. They erode trust and get forked. |
| MIT | No explicit patent grant. |
| A CLA | Lets the company relicense contributions, which breaks the promise. It's also friction for contributors. |
| A "companies over N employees pay" rule (Docker Desktop style) | Needs a non-open-source license on the app (§36). |

## Links

- Spec: [§0](../spec/rastai_spec_v5.md#0-decisions) item 12, [§35](../spec/rastai_spec_v5.md#35-licensing-and-community), [§36](../spec/rastai_spec_v5.md#36-business-model-new)
- Repo: [LICENSE](../../LICENSE), [DCO](../../DCO), [CONTRIBUTING.md](../../CONTRIBUTING.md)
- Related: [ADR-0012](0012-build-in-house-policy.md), [ADR-0015](0015-plugins-and-licensing.md)
