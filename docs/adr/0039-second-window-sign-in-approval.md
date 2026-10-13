# ADR-0039: A second window signs in only after the owner approves a locked prompt

**Status:** Accepted, 2026-10-13 (owner approval, UI design decision 6). Settles open question 24 in [contracts §14](../architecture/contracts.md#14-open-questions). Builds on [ADR-0034](0034-login-codes-loopback-host-owner-token.md) decision 10.

## Context

- A window that `view.window.open` puts in a second window shares the session cookie with its opener but not the opener's memory. Without the session key it can't run commands, so the second window has no way to act for the owner.
- Sharing the key silently would let any page the opener can reach open a window that inherits full control. Putting the key in a URL would leak it into history, logs and referrers.
- The desktop shell's second `WebviewWindow` may be opened without `window.opener`, so it may have no channel to the opener at all.
- The UI design (`docs/design/ui-design.md` §6.9, board 9) draws the approval as a locked prompt, so that no theme, plugin or override can change or disguise it.

## Decision

1. **The opener hands its session key to a window it opened only after the owner approves a locked prompt.** The prompt, "Let a new window sign in as you?", shows the window's title, its address, which window opened it, and what it can do ("do everything you can").
2. **Deny is the default.** Esc denies. Focus starts on Deny. Enter approves only when the focused button is Approve, so a stray Enter never approves.
3. **The key travels only by `postMessage` to the opener's own origin.** It never goes into a URL. The opener accepts a reply only when `event.source` is the window handle its `window.open` call returned and `event.origin` is its own origin; any other message is ignored. A denied or unanswered window shows `SignedOutScreen` and cannot run commands.
4. **The prompt is a new locked component, `SessionApproval`, in `@rastai/ui/security`.** It is drawn only with `--rr-locked-*` tokens and the fixed system font, as the other locked components are (contracts §5.6).
5. **The desktop-shell case signs in again.** A second `WebviewWindow` that has no `window.opener` signs in again with a new login code, as a reload does (frontend layout 9.3). It is not given the key.
6. **The mechanism is built before any second window ships.** Second windows are built only through this mechanism. The desktop work (M1-55) opens second windows no other way.
7. **This ADR is at design level.** It adds no wire message, command, event or setting name. The message shape and the names it uses go into `contracts.md` in a later contract PR, before the code that uses them (ADR-0038). The threat-model row for the prompt comes with that contract PR.

## Consequences

- A second window can act for the owner only after a person sees and approves a prompt that names the window and what it can do.
- A window opened by a hostile page cannot get the key: the owner sees the opener and the address, and the default answer is no.
- Desktop second windows sign in again, which costs one extra step. The shell keeps `window.opener` where it can, and the case that cannot keep it is the one where a fresh sign-in is the safe choice.
- The tests that prove this are required when the mechanism is built: an end-to-end test that a denied window cannot run a command; a test that a key message from a window other than the handle `window.open` returned, or from another origin, is ignored; and a screenshot test of `SessionApproval` under the hostile theme (`security.test.jsx`).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Hand the key over silently with `window.opener` | Any page the opener can reach would inherit full control, with no owner decision. |
| Put the key in the new window's URL | URLs leak into history, logs and referrers. |
| Every second window signs in again with a new code | Safe, and kept for the desktop case and for a denied approval, but it makes every normal second window an extra step. |
| Share the key through browser storage | Every script of the origin can read browser storage, so it is not a grant to one window. |

## Links

- Design: [`docs/design/ui-design.md`](../design/ui-design.md) §5.2, §6.9, §8 decision 6 and §9 (d)
- Contracts: [§14 open question 24](../architecture/contracts.md#14-open-questions), [§5.6 locked tokens](../architecture/contracts.md#56-theme-tokens)
- Layout: frontend and plugins, sections 8.4, 8.9 and 9.3 (`docs/architecture/layout/frontend-and-plugins.md`)
- Related: [ADR-0006](0006-ui-transport-http-only.md), [ADR-0007](0007-local-api-security.md), [ADR-0034](0034-login-codes-loopback-host-owner-token.md); threat model `docs/security/threat-model.md`
