# packages/plugin: `@rastai/plugin`

Read the root `CLAUDE.md` first. This file adds the local rules of layout `docs/architecture/layout/frontend-and-plugins.md` §7 (and §2.3, §10): the JavaScript plugin SDK, the one public API every core section, first-party plugin and (from M8) iframe plugin codes against.

## Where things are

- `src/index.js`: the only entry point. It exports the names contracts §15.3 registers for this package and re-exports `@rastai/client/commands` (one typed wrapper per core command), nothing else.
- `src/host-api.js`: JSDoc only: `PluginHost` (a `CommandExecutor`), `EventFilter`, `Contribution`.
- `src/in-process-host.js`: `createInProcessHost(pluginId, grantedScopes, services)`, the host for code inside the app. It fills the acting plugin id and checks the plugin's `commands:[scopes]` before a call leaves the window (RRS-C1005). The app builds the `HostServices` once per window (M1-53, `apps/web/src/plugin-host/`).
- `src/ui-registry.js`: `PluginUi` and its parts; `registerPluginUi` and `registeredPluginUi`.
- `src/state/`: the state tools of layout §2.3: `store.js`, `live-buffer.js` (with `frame-tick.js`, one shared animation-frame loop), `query-cache.js` (one cache per host), `draft-store.js`. They import nothing else in the package.
- `src/react/`: `PluginHostProvider`, `usePluginHost`, `useQuery`, `useCommand`, `CommandForm`, `JobProgress`; `DiagnosticList.jsx` and `plugin-host-context.js` are private.
- `src/commands/schema-to-fields.js`: a command's arguments JSON Schema to `FieldSpec[]` and back to arguments, for `CommandForm`.
- `src/testing/`: `mock-host.js` (`createMockHost`, exported), `host-contract.js` (the shared host table), and `render.js` (mounting for the browser tests only; not exported).
- `src/plugin-diagnostic.js`, `src/event-filter.js`: private helpers.

## Rules (layout §7.4)

1. **Never import `apps/web`**, and never export or wrap anything from `@rastai/ui`'s security subpath (`check-imports`, `package.test.js`). Plugin UIs can't render the security UI, so they can't fake it.
2. **A host is plain data and functions.** `PluginHost` never hands out DOM nodes, the shell's window or a `DaemonClient`, so the same shape can cross the iframe bridge (M8). Panels reach the daemon only through their host.
3. **Theme values only through `--rr-*` variables** (contracts §5.6); `host.theme()` is the resolved tokens without the locked ones. No color literals (`check-colors`).
4. **No protobuf runtime.** `@rastai/proto` and `@rastai/formats` appear only in JSDoc type imports; protobuf-es is reached only through `@rastai/client`. A Diagnostic's severity is written as its wire number (`plugin-diagnostic.js`).
5. **Never throw for an expected failure.** A refused or failed command is an `ExecuteOutcome` with Diagnostics carrying catalog codes; `registerPluginUi` throws only on a malformed UI, which the app's loader reports as RRS-P4105.
6. **No domain logic.** `useQuery` holds command results as they came; components may sort, group and format them for display only.
7. **Every export is public API** under the plugin API's semver (spec §13). A change to the `PluginHost` shape or the registry shapes needs a changelog note; removing anything needs a deprecation first.

## Tests

`just test-js` runs both kinds. `*.test.js` run in Node: the state tools, the schema fields, the registry, the in-process host's scope rules, the package surface, and `testing/host-contract.test.js`, which runs `@rastai/client`'s `EXECUTOR_CONTRACT_CASES` and this package's `HOST_CONTRACT_CASES` against the mock host and the in-process host (over a real `DaemonClient` on the client's fake daemon). `*.browser.test.jsx` run in Chromium and WebKit: the hooks, `CommandForm`, `JobProgress`, and a section that must render the same on the mock host as on the in-process host. `just check-js` type-checks the JSDoc.
