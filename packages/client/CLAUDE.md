# packages/client: `@rastai/client`

Read the root `CLAUDE.md` first. This file adds the local rules of layout `docs/architecture/layout/frontend-and-plugins.md` §3: the thin daemon client the web app, the plugin hosts and JavaScript scripts use.

## Where things are

- `src/index.js`: public exports only, plus the generated wrappers. `@rastai/client/commands` is `src/generated/commands.js` alone; `@rastai/client/testing` is `src/testing/executor-contract.js`, for other packages' tests.
- `src/connection.js`: `connectToDaemon`, `checkHealth`, the credential headers (`requestHeaders`). `src/execute.js`: `execute`, `listCommands`. `src/event-stream.js`: `openEventStream`. `src/login.js`, `src/blobs.js`, `src/diagnostics.js` (the RRS-U Diagnostics the client raises), `src/constants.js`.
- `src/types.js` and `src/command-executor.js` hold JSDoc only (contracts §12.4).
- `src/generated/commands.js` is written by `just codegen` (rastai-formats-codegen): never edit it.
- `src/testing/fake-daemon.js`: the tests' fake daemon (connect-es's in-memory router for `CommandService`, the daemon's own stream format for `Subscribe`, and the plain HTTP paths). Test support only.

## Rules (layout §3.4)

1. **No React, no UI, no app code.** Never import `@rastai/ui` or `apps/web`. Internal imports: `@rastai/proto` and `@rastai/formats` only (contracts §2.3).
2. **connect-es and protobuf-es only in `connection.js`** (contracts §2.5). Other files call the stubs on the client object with plain request objects. Tests use them through `testing/fake-daemon.js`; only `proto-golden.test.js` imports protobuf-es itself.
3. **Never throw on a command failure.** `execute` resolves to an `ExecuteOutcome`; a transport problem is an outcome with an RRS-U Diagnostic. Every user-facing failure carries a catalog code.
4. **Never store a credential.** The session key and a bearer token live only in memory, passed in by the caller. A cookie credential never sets `Authorization` and never reads `document.cookie`; it sends the key as `Rastai-Session-Key` on every request but `GET` and `HEAD`. A bearer credential sends `Authorization: Bearer <token>` on every request and nothing else.
5. **One event stream per `DaemonClient`.** `openEventStream` owns its reconnect loop; only `stop()` or a refused session ends it.
6. **Never read local files or open other sockets** (contracts §2.4 rule 4). The login code is read from the URL fragment only.

## Tests

`just test-js` runs vitest over `src/*.test.js` in Node; `just check-js` type-checks the JSDoc. The executor contract table (`testing/executor-contract.js`) runs against a `DaemonClient` in `execute.test.js`; `@rastai/plugin` runs it against each `PluginHost`.
