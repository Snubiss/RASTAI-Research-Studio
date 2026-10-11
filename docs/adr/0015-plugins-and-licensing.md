# ADR-0015: Plugins: one manifest, public API only, WASM deferred, plugins under any license

**Status:** Accepted, 2026-10-10

## Context

- Plugin APIs should be extracted from first-party code that already uses them, not invented (§2). The Obsidian and ComfyUI histories show what happens when third parties get a second-class or unchecked API.
- Third-party code has to be contained: UI in the browser, Python on the GPU box, and later providers that hold cloud credentials.
- §13 lists WASM modules as a plugin tier, but no first-party code needs one yet. Extism has been dropped.
- Licensing must stay simple for plugin authors and safe for the Apache-2.0 core (ADR-0018). Shipped dependencies must not pull in copyleft or non-commercial terms.

## Decision

1. **One manifest for everything:** `rastai-plugin.json` (`rastai.plugin/1`) for first-party plugins, third-party plugins and core sections. Each core section's `section.json` uses the manifest schema.
2. **First-party code uses only the public API:**
   - JavaScript: `@rastai/plugin`, `@rastai/ui`, `@rastai/canvas`, `@rastai/viz`;
   - Python: `rastai` and `rastai_plugin`.
   `tools/checks/check-imports` enforces this. Psychology and Evaluation live in `plugins/` and ship preinstalled as `.rrsplugin` packages (ADR-0020).
3. **Versioning:**
   - The plugin API has its own semver; manifests declare `engines.rastai`.
   - `requires_capabilities` (for example `designer.node@1`) is negotiated at install time. A missing capability disables the plugin with an `RRS-P1xxx` diagnostic and never crashes the app.
   - Experimental APIs sit behind `plugins.experimental_apis`, and registry plugins can't use them.
   - The API is frozen as 1.0 in Phase 2, extracted from what first-party plugins actually use.
4. **Runtime tiers:**

   | Tier | Runs as | Containment |
   |---|---|---|
   | Declarative (themes, layouts, keybindings, templates, eval suites) | data | the trust prompt if it requests network or data permissions |
   | UI, first-party | in-process React (3D views, per-token streams) | trusted |
   | UI, third-party | sandboxed iframe with a JSON-RPC bridge compatible with MCP Apps | `sandbox="allow-scripts"`, no same-origin, calls bounded by `commands:[scopes]` |
   | Python module | its own process, worker job kind `plugin_module` | network off by default (a new user and network namespace on Linux, contracts §8.8). Still trusted code, and the trust prompt says so |
   | Provider (Phase 3) | a separate process speaking `ProviderService` gRPC | credentials fetched from the keychain per call |
   | WASM | **deferred** until first-party code needs it, then on wasmtime with WASI 0.3 component interfaces | — |

5. **Permissions:** `network:[hosts]`, `fs:project|ro`, `secrets`, `gpu`, `weights.read`, `conversations.read`, `gateway.spend`, `sandbox.spawn`, `commands:[scopes]`. Any network or data permission triggers the trust prompt. An update that adds permissions asks again.
6. **Plugin Python dependencies** come from a per-plugin uv lockfile with hashes, installed at install time only (ADR-0009).
7. **Licensing:**
   - Plugins may use any license. The manifest's `license` field (an SPDX id) is shown before install.
   - Shipped dependencies of the core (including the uv environments and the pinned engines) are limited to permissive licenses: Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, BSL-1.0 (Boost), CC0-1.0, Unicode, CDLA-Permissive-2.0, and public domain (SQLite). MPL-2.0 is allowed for unmodified use, because its copyleft is per file.
   - GPU vendor runtime libraries pulled in by the PyTorch wheels and the `llama-server` builds (NVIDIA CUDA, AMD ROCm) are a named foundation exception under their vendors' redistribution terms.
   - GPL, LGPL, AGPL, SSPL, BUSL and non-commercial licenses are rejected.
   - Any other license needs a named exception with a reason in `deny.toml` or the allowlist JSON.
   - Model weights and SAE data keep their own terms. They are shown and checked, never bundled.

*Note since acceptance (2026-10-10):*
- *The first-party API list in decision 2 reads "JavaScript", not "TypeScript": the SDK `@rastai/plugin` is plain JavaScript with generated declarations for authors' editors, and the `rastai plugin new` templates are `.jsx` (ADR-0036). Plugin authors may use any language inside their own iframe.*
- *Network-off enforcement (decision 4's Python row) is settled in contracts §8.8. Ubuntu 23.10 and later restrict unprivileged user namespaces, so the `.deb` ships an AppArmor profile granting `userns` to the installed binary. Where namespaces are still unavailable, trusted, preinstalled first-party modules run without the network namespace with the warning `RRS-P2005`; a third-party module without network permission is refused with `RRS-P2001`.*
- *Every manifest contribution key maps to one capability, including `icon_sets`, `locales`, `macros` and `chat_templates`, with each one's prototype status (contracts §5.5).*
- *Theme plugins and plugin UIs can't restyle the security UI (egress badge, trust prompt, permission list, signed-out screen, error severity colors): those use locked tokens no theme can change (contracts §5.6).*
- *The decisions are unchanged.*

**Checked by:** `check-imports` in CI; a plugin test fixture per tier in `fixtures/plugins/`; a test that a plugin needing a missing capability installs as `incompatible` with a diagnostic; cargo-deny's license check and the allowlist JSONs on every PR.

## Consequences

**Good**
- Third parties get the same API as Psychology and Evaluation, so the API is proven before it's frozen.
- Untrusted UI can't reach the daemon's token or the DOM.
- Plugin authors choose their own license. The core's dependency tree stays Apache-compatible.

**Costs**
- First-party features can't take shortcuts through internals. Gaps in the public API must be filled first.
- Python plugin modules are trusted code. Network-off is a guardrail, not a sandbox.
- No WASM tier means no portable, sandboxed compute plugins until it's needed.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A private API for first-party features | Third parties become second-class, and the public API rots. |
| Third-party UI in-process | Full access to the page, the session and every command. |
| WASM tier now | No first-party need yet, and the component model is still settling. Extism is dropped. |
| Plugins installing PyPI packages at runtime | Forbidden: supply chain and reproducibility (ADR-0009). |
| A copyleft-tolerant dependency policy | Would constrain companies shipping Rastai and the commercial `rastai-business` add-ons (ADR-0018). |

## Links

- Spec: [§13](../spec/rastai_spec_v5.md#13-plugins-and-ecosystem), [§24](../spec/rastai_spec_v5.md#24-trust-and-privacy), [§27](../spec/rastai_spec_v5.md#27-build-in-house-policy-and-dependency-register-new), [§35](../spec/rastai_spec_v5.md#35-licensing-and-community)
- Contracts: [§1 repository tree](../architecture/contracts.md#1-repository-tree) (first-party plugins), [§4.8 iframe bridge](../architecture/contracts.md#4-wire-contracts), [§9.7 plugin states](../architecture/contracts.md#9-state-machines), [§8.8 network isolation for plugin modules](../architecture/contracts.md#88-network-isolation-for-plugin-modules), [§5.5 capabilities](../architecture/contracts.md#55-extension-point-capabilities)
- Related: [ADR-0009](0009-python-environments-uv.md), [ADR-0012](0012-build-in-house-policy.md), [ADR-0018](0018-license-apache-dco-trademark.md), [ADR-0020](0020-evaluation-first-party-plugin.md), [ADR-0036](0036-frontend-plain-javascript.md)
