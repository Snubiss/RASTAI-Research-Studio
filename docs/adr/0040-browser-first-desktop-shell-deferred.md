# ADR-0040: The browser is the client for now; the desktop shell is deferred to M6 and its choice reopened

**Status:** Accepted, 2026-10-14 (owner decision: "browser only for now. desktop shell later is acceptable"). Reopens the shell choice that the spec §27 Tauri rows and plan task M1-55 record. Keeps ADR-0006 and ADR-0034: the shell stays only a shell.

## Context

- The Tauri shell (M1-55) was built on a local branch (`m1/M1-55`, commit 8c72221), never pushed or merged. The branch exists only in the owner's local clone, not on origin. Its window, single-instance, navigation guard and launch-line tests pass.
- On Linux, Tauri 2.x draws through the gtk-rs GTK3 bindings. Those are archived and flagged unmaintained: RUSTSEC-2024-0411 and related advisories. The old `glib` they pin has an unsoundness advisory, RUSTSEC-2024-0429. Tauri's dependency tree also hits the advisory bans in `deny.toml`.
- Upstream's GTK4 and WebKitGTK 6 port (tauri #14684, wry #1767) is not released yet. The owner does not want deprecated packages in the product.
- `rastai open` on main already starts the daemon if needed and opens the user's own browser, signed in. The browser is a full client (spec §16), so the product works without a shell.

## Decision

1. **The browser is the first-class client.** The daemon serves the web app. `rastai open` opens the user's browser, already signed in (ADR-0034).
2. **The desktop shell is deferred to M6,** beside packaging (M6-E05). Its task is M1-55 (the desktop window), moved from M1 to M6 and kept under its id (plan 6.4). M1 builds no `apps/desktop`.
3. **The shell choice is reopened, and decided when M6 starts.** The options are Electron (maintained, the same Chromium on every OS, large, adds Node to the shell process) or Tauri, once a release ships the GTK4 port. Choosing Electron needs the owner's approval of a new dependency and a new ADR that replaces this decision's shell part. Choosing Tauri needs a release with GTK4, and its dependency rows in spec §27 come back then.
4. **Whatever shell comes back keeps these rules:** it only runs `rastai open --print-url` and opens the printed URL; it keeps one instance and refuses navigation to other origins; it makes no HTTP request and no crypto; it never reads `owner.token` (ADR-0034, ADR-0006).
5. **The Tauri branch stays for reference.** The window, navigation guard, single instance over `desktop.sock`, launch-line parsing and 22 tests are not merged. The parsing and origin rules carry over to any shell.

**Checked by:** the M1 browser tests (`tests/e2e/palette.spec.js`, `basic-advanced.spec.js`, `theme.spec.js` in `just e2e-web`) and the three-surfaces test (`tests/e2e/three_surfaces`, M1-49), which run the product with no shell. `check-names` (M1-57) passes because the desktop names in contracts §2.1, §2.3 and §15.1 carry the `(M6)` marker until they are built.

## Consequences

- M1 has no shell work and no WebKitGTK risk. The UI is tested in Chromium and WebKit through Playwright, not in a desktop window.
- The M1 schedule loses M1-55; every other task keeps its place. M6-E05 waits for M1-55 (size M, 90-day, plan 6.4).
- The desktop items in `docs/design/ui-design.md` §9 (d) move with M1-55 to M6. The rail that collapses to icons below 1024 px is not built in M1-53: ui-design §3.1 places the icon rail in Phase 2, and plan 6.10 lists it.
- Costs: no desktop window, file associations or signed updates until M6. Double-clicking a `.rrsreplay` opens nothing in M1 to M5; the `rastai open` path and the browser's file picker cover those cases.
- ADR-0034, ADR-0039 and the layout doc (`docs/architecture/layout/frontend-and-plugins.md` §9) still describe the shell's design. That design is kept for M6 and is marked deferred.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Stay on Tauri now and record the GTK3 advisories as accepted | The owner does not want deprecated packages in the product. The advisories would sit in `deny.toml` exceptions for the life of the shell. |
| Go Electron now | It is a new dependency and a new shell. It needs the owner's approval and an ADR of its own, and the browser already covers M1 to M5. Decided at M6 start (decision 3). |
| Rewrite the frontend in Flutter, Qt or Rust-native UI | It would rewrite M1-50 to M1-54. It loses the browser as a full client, custom frontends and themes, and plugin UIs in sandboxed frames. GPU visuals run in the page through WebGL and WebGPU. |
| Drop the desktop shell for good | Not decided. The shell comes back in M6 with a chosen engine; this ADR only defers it. |

## Links

- Owner decision: GitHub issue #55 (comment of 2026-10-14) and issue #283 (DOCS-02).
- Spec: [§27 Tauri rows](../spec/rastai_spec_v5.md) marked deferred, and [§37 v5.4.6](../spec/rastai_spec_v5.md#37-changes-from-v41).
- Plan: [M1-55](../plan/implementation-plan.md) (the desktop window, in M6) and [M6-E05](../plan/implementation-plan.md) (packaging); the M1-55 row is removed from M1 and listed in M6.
- Contracts: `rastai-desktop` and `@rastai/desktop` are marked `(M6)` in [contracts.md](../architecture/contracts.md) §2.1, §2.3 and §15.1; the change entry is `docs/architecture/contracts-changes/DOCS-02.md`.
- Related: [ADR-0006](0006-ui-transport-http-only.md) (the shell is only a shell), [ADR-0034](0034-login-codes-loopback-host-owner-token.md) and [ADR-0039](0039-second-window-sign-in-approval.md) (the rules any shell keeps); threat model rows for the shell in `docs/security/threat-model.md`.
