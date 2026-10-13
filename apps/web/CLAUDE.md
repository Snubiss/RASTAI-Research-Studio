# apps/web: `@rastai/web`

Read the root `CLAUDE.md` first. This file adds the local rules of `docs/architecture/layout/frontend-and-plugins.md` §8: the web app, served by the daemon at `/` (embedded from `dist/`).

## Where things are

- `src/main.jsx`: the thin main. Each step is one function in `app/boot.js`, `state/theme.js` or `plugin-host/`.
- `src/app/`: sign-in and the window's one `DaemonClient` (`boot.js`, `daemon.js`), the shell object the app is built from (`shell.js`), the frame (`App.jsx`), the global keys (`keyboard.js`) and the frame's CSS (`frame.css`).
- `src/state/`: the app's stores (`app-stores.js`), the Basic/Advanced mode (`mode.js`) and the theme (`theme.js`).
- `src/events/`: the one event stream (`event-hub.js`) and the resync after RRS-C0401 (`resync.js`).
- `src/routing/`: the fragment grammar (`route.js`) and navigation (`navigate.js`).
- `src/workspace/`: the layout tree (pure functions), the workspace and the panel frame.
- `src/plugin-host/`: the plugin loader (`load-plugins.js`, the only caller of `registerPluginUi`), the first-party table, the core sections and the in-process host's services.
- `src/frame/`: the top bar, the section rail, the status bar, the palette entry and the dialogs.
- `src/sections/`: the core sections (Home now; the other sections show their honest "not available yet" state).

## Rules

1. **No domain logic.** Anything stateful or computed is a command. The app renders, routes and keeps view state.
2. **Only `app/`, `events/` and `plugin-host/` import `@rastai/client`** (`check-imports`). Sections import the public API only.
3. **Only this app imports `@rastai/ui/security`**, and it renders the security UI from there.
4. **No color, font, spacing or radius literals.** Everything reads `--rr-*` variables (`check-colors`).
5. **Never store a credential or a secret.** The session key lives in memory only. localStorage holds only conveniences.
6. **No Tauri APIs.** The app behaves the same in a browser.
7. **No CDN.** Fonts and styles come from the bundle, so the build is offline and reproducible.

## Tests

`just test-js` runs the `*.test.js` files in Node and the `*.browser.test.jsx` files in Chromium and WebKit (the shell's regions, keyboard navigation, Basic/Advanced, the empty and error states). `just check-js` type-checks the JSDoc. `just web-embed` builds `dist/`.
