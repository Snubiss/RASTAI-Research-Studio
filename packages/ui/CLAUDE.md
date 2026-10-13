# packages/ui: `@rastai/ui`

Read the root `CLAUDE.md` first. This file adds the local rules of layout `docs/architecture/layout/frontend-and-plugins.md` §4 (and §2.4, §2.9, §2.10): the design system that core sections, plugin UIs, iframes and the static viewer share.

## Where things are

- `src/index.js`: the public exports. `@rastai/ui/security` is `src/security/index.js` alone; `@rastai/ui/styles/base.css` and `@rastai/ui/styles/components.css` are the two stylesheets a page loads once.
- `src/tokens/`: the theme token contract (contracts §5.6). `light.theme.json` and `dark.theme.json` are the built-in themes as `rastai.theme/1` files (`built-in-themes.js` imports them; `crates/rastai-formats/tests/theme_tokens.rs` validates the same files). `resolve-tokens.js` (`builtInTheme`), `apply-theme.js` (`applyThemeTokens`), `palettes.js` (the colorblind set), `locked.js` (`LOCKED_TOKENS`), `canvas-colors.js` and `viz-colors.js` (plain RGBA data for the renderers), `token-types.js` (JSDoc only).
- `src/security/`: `EgressBadge`, `TrustPrompt`, `PermissionList`, `SignedOutScreen`, `TokenConfirm`, their inline locked styles (`locked-styles.js`), the security focus ring each component draws (`LockedFocusRing.jsx`) and the plain-English permission table (`permission-text.js`).
- `src/mode/`: `ModeProvider`, `useMode`, `AdvancedOnly`; `mode-context.js` is private.
- `src/components/`: one component per file; `FieldFrame.jsx` is private to the fields. `show-toast.js` keeps the toast list as plain data.
- `src/diagnostics/`: `DiagnosticView` and `catalog-lookup.js` (the catalog text for a Diagnostic).
- `src/styles/`: `base.css` and `components.css`, values only through `var(--rr-*)`. `base.css` loads the bundled faces with `@font-face`.
- `src/fonts/`: Geist and Geist Mono (variable woff2) and `OFL.txt`, from the official release; `README.md` has the source and checksums. Local files only, never a CDN.
- `src/testing/`: helpers for the browser tests only (`mount`, `loadStyles`); not exported.

## Rules (layout §4.4)

1. **No daemon access.** Never import `@rastai/client`, `@rastai/plugin` or app code. Internal imports: `@rastai/catalog`; `@rastai/proto` only as a JSDoc type import (`Diagnostic`).
2. **No global state.** Components are controlled or keep only local UI state. A list the app keeps (toasts) is plain data with pure functions.
3. **Advanced only adds.** There is `AdvancedOnly` and no `BasicOnly`; put it after or beside Basic controls, never around them.
4. **Color values live only in `src/tokens/`** (`check-colors`). Components and CSS read `var(--rr-*)`; `package.test.js` fails on a variable `applyThemeTokens` doesn't write.
5. **The security UI reads only `--rr-locked-*` for its colors**, and fixes its own font, sizes and spacing, all as inline styles. Only `apps/web` may import `@rastai/ui/security` (`check-imports`). The diagnostic severity colors are locked too.
6. **No HTML from strings:** no `innerHTML`, `dangerouslySetInnerHTML` or their relatives anywhere (`package.test.js`).
7. User-facing errors are Diagnostics with catalog codes, shown by `DiagnosticView`.

## Tests

`just test-js` runs both kinds. `*.test.js` run in Node: `tokens/tokens.test.js` (every role resolves, every documented variable is written, nothing reaches the locked variables), `tokens/palettes.test.js` (the §2.9 contrast rules and the colorblind palette under simulated protanopia and deuteranopia), and the helpers. `*.browser.test.jsx` run in Chromium and WebKit through Playwright (vitest browser mode; install the browsers once with `tools/ci/install-dev-tools browsers`): the security UI under a hostile theme (screenshots and computed colors must not change), `DiagnosticView` in both modes, Basic/Advanced, and the components. `just check-js` type-checks the JSDoc.
